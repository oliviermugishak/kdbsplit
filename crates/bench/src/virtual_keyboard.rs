use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::{AsRawFd, RawFd};

const EV_KEY: u16 = 0x01;
const EV_SYN: u16 = 0x00;
const SYN_REPORT: u16 = 0;

#[allow(dead_code)]
pub struct VirtualKeyboard {
    file: File,
}

impl VirtualKeyboard {
    pub fn create() -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/uinput")
            .context("failed to open /dev/uinput")?;

        let fd = file.as_raw_fd();
        ioctl_set_bit(fd, ui_set_evbit(), EV_KEY)?;
        for code in 1..=111 {
            ioctl_set_bit(fd, ui_set_keybit(), code)?;
        }

        let mut dev = UInputUserDev::default();
        let name = b"KbdSplit Benchmark Keyboard\0";
        dev.name[..name.len().min(79)].copy_from_slice(&name[..name.len().min(79)]);
        dev.id.bustype = 0x03;
        dev.id.vendor = 0x045e;
        dev.id.product = 0x0999;
        dev.id.version = 1;

        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&dev as *const UInputUserDev).cast::<u8>(),
                std::mem::size_of::<UInputUserDev>(),
            )
        };
        (&file).write_all(bytes).context("failed to configure uinput")?;
        ioctl_simple(fd, ui_dev_create()).context("UI_DEV_CREATE failed")?;

        Ok(Self { file })
    }

    pub fn press_and_release(&mut self, code: u16) -> Result<()> {
        self.emit(EV_KEY, code, 1)?;
        self.emit(EV_SYN, SYN_REPORT, 0)?;
        self.emit(EV_KEY, code, 0)?;
        self.emit(EV_SYN, SYN_REPORT, 0)?;
        Ok(())
    }

    fn emit(&mut self, type_: u16, code: u16, value: i32) -> Result<()> {
        let event = InputEvent {
            time: libc::timeval { tv_sec: 0, tv_usec: 0 },
            type_,
            code,
            value,
        };
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&event as *const InputEvent).cast::<u8>(),
                std::mem::size_of::<InputEvent>(),
            )
        };
        self.file.write_all(bytes).context("uinput write failed")
    }

}

impl Drop for VirtualKeyboard {
    fn drop(&mut self) {
        let _ = unsafe { libc::ioctl(self.file.as_raw_fd(), ui_dev_destroy()) };
    }
}

#[repr(C)]
struct UInputUserDev {
    name: [u8; 80],
    id: InputId,
    ff_effects_max: u32,
    absmax: [i32; 64],
    absmin: [i32; 64],
    absfuzz: [i32; 64],
    absflat: [i32; 64],
}

impl Default for UInputUserDev {
    fn default() -> Self {
        Self {
            name: [0; 80],
            id: InputId::default(),
            ff_effects_max: 0,
            absmax: [0; 64],
            absmin: [0; 64],
            absfuzz: [0; 64],
            absflat: [0; 64],
        }
    }
}

#[repr(C)]
#[derive(Default)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
#[derive(Default)]
struct InputEvent {
    time: libc::timeval,
    type_: u16,
    code: u16,
    value: i32,
}

fn ioctl_set_bit(fd: RawFd, request: libc::c_ulong, bit: u16) -> Result<()> {
    let rc = unsafe { libc::ioctl(fd, request, bit as libc::c_int) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error()).context("ioctl_set_bit");
    }
    Ok(())
}

fn ioctl_simple(fd: RawFd, request: libc::c_ulong) -> Result<()> {
    let rc = unsafe { libc::ioctl(fd, request) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error()).context("ioctl");
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
const IOC_NONE: u64 = 0;
const IOC_WRITE: u64 = 1;

const fn ioc(dir: u64, type_: u64, nr: u64, size: u64) -> libc::c_ulong {
    ((dir << IOC_DIRSHIFT) | (type_ << IOC_TYPESHIFT) | (nr << IOC_NRSHIFT) | (size << IOC_SIZESHIFT)) as libc::c_ulong
}

const fn io(type_: u8, nr: u8) -> libc::c_ulong {
    ioc(IOC_NONE, type_ as u64, nr as u64, 0)
}

const fn iow_int(type_: u8, nr: u8) -> libc::c_ulong {
    ioc(IOC_WRITE, type_ as u64, nr as u64, std::mem::size_of::<libc::c_int>() as u64)
}

const fn ui_dev_create() -> libc::c_ulong { io(b'U', 1) }
const fn ui_dev_destroy() -> libc::c_ulong { io(b'U', 2) }
const fn ui_set_evbit() -> libc::c_ulong { iow_int(b'U', 100) }
const fn ui_set_keybit() -> libc::c_ulong { iow_int(b'U', 101) }
