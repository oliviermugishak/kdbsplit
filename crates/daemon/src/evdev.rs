use anyhow::{Context, Result};
use kbdsplit_shared::{DeviceFingerprint, KeyboardDevice};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

pub const EV_KEY: u16 = 0x01;
pub const EV_SYN: u16 = 0x00;
pub const SYN_DROPPED: u16 = 3;

pub const KEY_MAX: usize = 0x2ff;
pub const KEY_CNT: usize = KEY_MAX + 1;
const KEY_A: usize = 30;
const KEY_Z: usize = 44;
const KEY_1: usize = 2;
const KEY_0: usize = 11;
const KEY_SPACE: usize = 57;
const KEY_ENTER: usize = 28;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct InputEvent {
    pub time: libc::timeval,
    pub type_: u16,
    pub code: u16,
    pub value: i32,
}

pub struct InputReader {
    file: File,
    pub path: PathBuf,
    grabbed: bool,
    can_grab: bool,
    epoll_fd: RawFd,
}

impl InputReader {
    pub fn open(path: &Path) -> Result<Self> {
        let (file, can_grab) = match OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
        {
            Ok(f) => (f, true),
            Err(_) => {
                let f = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(path)
                    .with_context(|| format!("failed to open {}", path.display()))?;
                (f, false)
            }
        };

        let epoll_fd = match unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) } {
            fd if fd >= 0 => fd,
            _ => {
                let err = std::io::Error::last_os_error();
                anyhow::bail!("epoll_create1 failed: {err}");
            }
        };
        let mut ev = libc::epoll_event {
            events: libc::EPOLLIN as u32,
            u64: 0,
        };
        if unsafe { libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_ADD, file.as_raw_fd(), &mut ev) } < 0
        {
            let err = std::io::Error::last_os_error();
            unsafe {
                libc::close(epoll_fd);
            }
            anyhow::bail!("epoll_ctl add failed: {err}");
        }

        Ok(Self {
            file,
            path: path.to_path_buf(),
            grabbed: false,
            can_grab,
            epoll_fd,
        })
    }

    /// Block until data is available on the evdev fd, or timeout elapses.
    /// Returns Ok(true) if data is available, Ok(false) on timeout.
    pub fn wait_for_event(&self, timeout_ms: i32) -> Result<bool> {
        let mut events = [libc::epoll_event { events: 0, u64: 0 }; 1];
        loop {
            let n = unsafe { libc::epoll_wait(self.epoll_fd, events.as_mut_ptr(), 1, timeout_ms) };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == ErrorKind::Interrupted {
                    continue;
                }
                return Err(err).context("epoll_wait failed");
            }
            return Ok(n > 0);
        }
    }

    pub fn grabbed(&self) -> bool {
        self.grabbed
    }

    pub fn set_grabbed(&mut self, grabbed: bool) -> Result<()> {
        if !self.can_grab && grabbed {
            anyhow::bail!("no write permission on evdev device, cannot grab");
        }
        if self.grabbed == grabbed {
            return Ok(());
        }
        ioctl_eviocgrab(self.file.as_raw_fd(), grabbed)?;
        self.grabbed = grabbed;
        Ok(())
    }

    pub fn read_key_bitmap(&mut self) -> Result<[u8; 96]> {
        let mut bits = [0u8; KEY_CNT.div_ceil(8)];
        ioctl_eviocgkey(self.file.as_raw_fd(), &mut bits)?;
        Ok(bits)
    }

    pub fn read_event(&mut self) -> Result<Option<InputEvent>> {
        let mut event = InputEvent::default();
        let ptr = (&mut event as *mut InputEvent).cast::<u8>();
        let size = std::mem::size_of::<InputEvent>();
        let buffer = unsafe { std::slice::from_raw_parts_mut(ptr, size) };
        match self.file.read_exact(buffer) {
            Ok(()) => Ok(Some(event)),
            Err(err) if err.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(err) if err.kind() == ErrorKind::Interrupted => Ok(None),
            Err(err) => Err(err).with_context(|| format!("failed to read {}", self.path.display())),
        }
    }
}

impl Drop for InputReader {
    fn drop(&mut self) {
        if self.grabbed {
            let _ = ioctl_eviocgrab(self.file.as_raw_fd(), false);
        }
        unsafe {
            libc::close(self.epoll_fd);
        }
    }
}

pub fn discover_keyboards() -> (Vec<KeyboardDevice>, Vec<String>) {
    let mut devices = Vec::new();
    let mut warnings = Vec::new();

    let entries = match fs::read_dir("/dev/input") {
        Ok(entries) => entries,
        Err(err) => {
            warnings.push(format!("Cannot read /dev/input: {err}"));
            return (devices, warnings);
        }
    };

    let mut paths = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.starts_with("event"))
        })
        .collect::<Vec<_>>();
    paths.sort();

    for path in paths {
        match inspect_keyboard(&path) {
            Ok(Some(device)) => devices.push(device),
            Ok(None) => {}
            Err(err) => warnings.push(format!("{}: {err:#}", path.display())),
        }
    }

    (devices, warnings)
}

fn inspect_keyboard(path: &Path) -> Result<Option<KeyboardDevice>> {
    let file = OpenOptions::new()
        .read(true)
        .open(path)
        .with_context(|| "permission denied or device is unavailable")?;
    if !has_keyboard_keys(file.as_raw_fd())? {
        return Ok(None);
    }

    let id = read_input_id(file.as_raw_fd()).unwrap_or_default();
    let name = ioctl_string(file.as_raw_fd(), 0x06).unwrap_or_else(|| {
        path.file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("Keyboard")
            .to_owned()
    });
    let phys = ioctl_string(file.as_raw_fd(), 0x07);
    let uniq = ioctl_string(file.as_raw_fd(), 0x08);

    let fingerprint = DeviceFingerprint {
        bustype: nonzero(id.bustype),
        vendor: nonzero(id.vendor),
        product: nonzero(id.product),
        version: nonzero(id.version),
        name: name.clone(),
        phys,
        uniq,
    };
    let device_id = fingerprint.stable_id();
    let is_internal = fingerprint.phys.as_deref().is_some_and(|phys| {
        phys.contains("isa") || phys.contains("i8042") || phys.contains("serio")
    });

    let can_grab = OpenOptions::new().read(true).write(true).open(path).is_ok();

    Ok(Some(KeyboardDevice {
        id: device_id,
        path: path.display().to_string(),
        name,
        fingerprint,
        is_internal,
        connected: true,
        assigned_slot: None,
        locked: false,
        can_grab,
        last_error: None,
    }))
}

fn nonzero(value: u16) -> Option<u16> {
    (value != 0).then_some(value)
}

fn has_keyboard_keys(fd: RawFd) -> Result<bool> {
    let mut bits = vec![0_u8; KEY_CNT.div_ceil(8)];
    ioctl_eviocgbit(fd, EV_KEY, &mut bits)?;
    let has_letters = (KEY_A..=KEY_Z)
        .filter(|code| bit_is_set(&bits, *code))
        .count()
        >= 10;
    let has_numbers = (KEY_1..=KEY_0).any(|code| bit_is_set(&bits, code));
    Ok(has_letters && has_numbers && bit_is_set(&bits, KEY_SPACE) && bit_is_set(&bits, KEY_ENTER))
}

pub fn bit_is_set(bits: &[u8], bit: usize) -> bool {
    bits.get(bit / 8)
        .is_some_and(|byte| byte & (1 << (bit % 8)) != 0)
}

fn read_input_id(fd: RawFd) -> Result<InputId> {
    let mut id = InputId::default();
    let rc = unsafe { libc::ioctl(fd, eviocgid(), &mut id) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error()).context("EVIOCGID failed");
    }
    Ok(id)
}

fn ioctl_string(fd: RawFd, nr: u8) -> Option<String> {
    let mut buf = vec![0_u8; 256];
    let rc = unsafe { libc::ioctl(fd, eviocg_string(nr, buf.len()), buf.as_mut_ptr()) };
    if rc < 0 {
        return None;
    }
    let len = buf.iter().position(|&byte| byte == 0).unwrap_or(buf.len());
    Some(String::from_utf8_lossy(&buf[..len]).trim().to_owned())
}

fn ioctl_eviocgbit(fd: RawFd, ev: u16, buffer: &mut [u8]) -> Result<()> {
    let rc = unsafe { libc::ioctl(fd, eviocgbit(ev, buffer.len()), buffer.as_mut_ptr()) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error()).context("EVIOCGBIT failed");
    }
    Ok(())
}

fn ioctl_eviocgkey(fd: RawFd, buffer: &mut [u8]) -> Result<()> {
    let rc = unsafe { libc::ioctl(fd, eviocgkey(buffer.len()), buffer.as_mut_ptr()) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error()).context("EVIOCGKEY failed");
    }
    Ok(())
}

fn ioctl_eviocgrab(fd: RawFd, grabbed: bool) -> Result<()> {
    let value: libc::c_int = i32::from(grabbed);
    let rc = unsafe { libc::ioctl(fd, eviocgrab(), value) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error()).context("EVIOCGRAB failed");
    }
    Ok(())
}

const IOC_NRBITS: u64 = 8;
const IOC_TYPEBITS: u64 = 8;
const IOC_SIZEBITS: u64 = 14;
const IOC_NRSHIFT: u64 = 0;
const IOC_TYPESHIFT: u64 = IOC_NRSHIFT + IOC_NRBITS;
const IOC_SIZESHIFT: u64 = IOC_TYPESHIFT + IOC_TYPEBITS;
const IOC_DIRSHIFT: u64 = IOC_SIZESHIFT + IOC_SIZEBITS;
const IOC_WRITE: u64 = 1;
const IOC_READ: u64 = 2;

const fn ioc(dir: u64, type_: u64, nr: u64, size: u64) -> libc::c_ulong {
    ((dir << IOC_DIRSHIFT)
        | (type_ << IOC_TYPESHIFT)
        | (nr << IOC_NRSHIFT)
        | (size << IOC_SIZESHIFT)) as libc::c_ulong
}

const fn ior<T>(type_: u8, nr: u8) -> libc::c_ulong {
    ioc(
        IOC_READ,
        type_ as u64,
        nr as u64,
        std::mem::size_of::<T>() as u64,
    )
}

const fn iow<T>(type_: u8, nr: u8) -> libc::c_ulong {
    ioc(
        IOC_WRITE,
        type_ as u64,
        nr as u64,
        std::mem::size_of::<T>() as u64,
    )
}

const fn eviocgid() -> libc::c_ulong {
    ior::<InputId>(b'E', 0x02)
}

const fn eviocgbit(ev: u16, len: usize) -> libc::c_ulong {
    ioc(IOC_READ, b'E' as u64, 0x20 + ev as u64, len as u64)
}

const fn eviocg_string(nr: u8, len: usize) -> libc::c_ulong {
    ioc(IOC_READ, b'E' as u64, nr as u64, len as u64)
}

const fn eviocgkey(len: usize) -> libc::c_ulong {
    ioc(IOC_READ, b'E' as u64, 0x18, len as u64)
}

const fn eviocgrab() -> libc::c_ulong {
    iow::<libc::c_int>(b'E', 0x90)
}
