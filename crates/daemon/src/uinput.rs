use anyhow::{Context, Result};
use kbdsplit_shared::{ControllerSlot, ControllerState, GamepadButton};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::{AsRawFd, RawFd};

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0;

const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_Z: u16 = 0x02;
const ABS_RX: u16 = 0x03;
const ABS_RY: u16 = 0x04;
const ABS_RZ: u16 = 0x05;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;
const ABS_CNT: usize = 0x40;

const BTN_SOUTH: u16 = 0x130;
const BTN_EAST: u16 = 0x131;
const BTN_NORTH: u16 = 0x133;
const BTN_WEST: u16 = 0x134;
const BTN_TL: u16 = 0x136;
const BTN_TR: u16 = 0x137;
const BTN_SELECT: u16 = 0x13a;
const BTN_START: u16 = 0x13b;
const BTN_MODE: u16 = 0x13c;
const BTN_THUMBL: u16 = 0x13d;
const BTN_THUMBR: u16 = 0x13e;
const BTN_DPAD_UP: u16 = 0x220;
const BTN_DPAD_DOWN: u16 = 0x221;
const BTN_DPAD_LEFT: u16 = 0x222;
const BTN_DPAD_RIGHT: u16 = 0x223;

#[repr(C)]
#[derive(Clone, Copy)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct UInputUserDev {
    name: [u8; 80],
    id: InputId,
    ff_effects_max: u32,
    absmax: [i32; ABS_CNT],
    absmin: [i32; ABS_CNT],
    absfuzz: [i32; ABS_CNT],
    absflat: [i32; ABS_CNT],
}

fn slot_product_id(slot: ControllerSlot) -> u16 {
    match slot {
        ControllerSlot::One => 0x028e,
        ControllerSlot::Two => 0x028f,
        ControllerSlot::Three => 0x0290,
        ControllerSlot::Four => 0x0291,
    }
}

fn default_dev(slot: ControllerSlot) -> UInputUserDev {
    UInputUserDev {
        name: [0; 80],
        id: InputId {
            bustype: 0x03,
            vendor: 0x045e,
            product: slot_product_id(slot),
            version: slot.index() as u16 + 1,
        },
        ff_effects_max: 0,
        absmax: [0; ABS_CNT],
        absmin: [0; ABS_CNT],
        absfuzz: [0; ABS_CNT],
        absflat: [0; ABS_CNT],
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct InputEvent {
    time: libc::timeval,
    type_: u16,
    code: u16,
    value: i32,
}

const MAX_BATCH_EVENTS: usize = 24;
const EVENT_SIZE: usize = std::mem::size_of::<InputEvent>();

pub struct VirtualGamepad {
    file: File,
    last: ControllerState,
}

fn hat_from_buttons(buttons: u16) -> (i32, i32) {
    let left = buttons & GamepadButton::DpadLeft.bit() != 0;
    let right = buttons & GamepadButton::DpadRight.bit() != 0;
    let up = buttons & GamepadButton::DpadUp.bit() != 0;
    let down = buttons & GamepadButton::DpadDown.bit() != 0;
    let x = match (left, right) {
        (true, false) => -1,
        (false, true) => 1,
        _ => 0,
    };
    let y = match (up, down) {
        (true, false) => -1,
        (false, true) => 1,
        _ => 0,
    };
    (x, y)
}

impl VirtualGamepad {
    pub fn create(slot: ControllerSlot) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/uinput")
            .context("failed to open /dev/uinput")?;

        ioctl_set_bit(file.as_raw_fd(), ui_set_evbit(), EV_KEY)?;
        ioctl_set_bit(file.as_raw_fd(), ui_set_evbit(), EV_ABS)?;
        for button in GamepadButton::ALL {
            ioctl_set_bit(file.as_raw_fd(), ui_set_keybit(), button_code(button))?;
        }
        for axis in [ABS_X, ABS_Y, ABS_RX, ABS_RY, ABS_Z, ABS_RZ, ABS_HAT0X, ABS_HAT0Y] {
            ioctl_set_bit(file.as_raw_fd(), ui_set_absbit(), axis)?;
        }

        let mut dev = default_dev(slot);
        let name = format!("KbdSplit Virtual Xbox Controller {}", slot.number());
        let bytes = name.as_bytes();
        let name_len = bytes.len().min(dev.name.len() - 1);
        dev.name[..name_len].copy_from_slice(&bytes[..name_len]);
        for axis in [ABS_X, ABS_Y, ABS_RX, ABS_RY] {
            dev.absmin[axis as usize] = -32767;
            dev.absmax[axis as usize] = 32767;
            dev.absflat[axis as usize] = 4096;
        }
        for axis in [ABS_Z, ABS_RZ] {
            dev.absmin[axis as usize] = 0;
            dev.absmax[axis as usize] = 255;
        }
        for axis in [ABS_HAT0X, ABS_HAT0Y] {
            dev.absmin[axis as usize] = -1;
            dev.absmax[axis as usize] = 1;
        }

        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&dev as *const UInputUserDev).cast::<u8>(),
                std::mem::size_of::<UInputUserDev>(),
            )
        };
        (&file)
            .write_all(bytes)
            .context("failed to configure uinput device")?;
        ioctl_simple(file.as_raw_fd(), ui_dev_create()).context("UI_DEV_CREATE failed")?;

        Ok(Self {
            file,
            last: ControllerState::default(),
        })
    }

    /// Emit all state changes in a single batched write to /dev/uinput.
    pub fn emit_state(&mut self, state: &ControllerState) -> Result<()> {
        let mut batch: [u8; MAX_BATCH_EVENTS * EVENT_SIZE] = [0; MAX_BATCH_EVENTS * EVENT_SIZE];
        let mut count = 0;

        macro_rules! push_event {
            ($type_:expr, $code:expr, $value:expr) => {{
                let ev = InputEvent {
                    time: libc::timeval { tv_sec: 0, tv_usec: 0 },
                    type_: $type_,
                    code: $code,
                    value: $value,
                };
                let src = unsafe {
                    std::slice::from_raw_parts(
                        (&ev as *const InputEvent).cast::<u8>(),
                        EVENT_SIZE,
                    )
                };
                let offset = count * EVENT_SIZE;
                batch[offset..offset + EVENT_SIZE].copy_from_slice(src);
                count += 1;
            }};
        }

        // Buttons
        for button in GamepadButton::ALL {
            let prev = self.last.button_pressed(button);
            let curr = state.button_pressed(button);
            if prev != curr {
                push_event!(EV_KEY, button_code(button), i32::from(curr));
            }
        }

        // Hat (derived from D-pad buttons)
        let (prev_hx, prev_hy) = hat_from_buttons(self.last.buttons);
        let (cur_hx, cur_hy) = hat_from_buttons(state.buttons);
        if cur_hx != prev_hx {
            push_event!(EV_ABS, ABS_HAT0X, cur_hx);
        }
        if cur_hy != prev_hy {
            push_event!(EV_ABS, ABS_HAT0Y, cur_hy);
        }

        // Axes
        macro_rules! push_axis_if {
            ($code:expr, $prev:expr, $cur:expr) => {
                if $prev != $cur {
                    push_event!(EV_ABS, $code, $cur);
                }
            };
        }
        push_axis_if!(ABS_X, self.last.axes.left_x as i32, state.axes.left_x as i32);
        push_axis_if!(ABS_Y, self.last.axes.left_y as i32, state.axes.left_y as i32);
        push_axis_if!(ABS_RX, self.last.axes.right_x as i32, state.axes.right_x as i32);
        push_axis_if!(ABS_RY, self.last.axes.right_y as i32, state.axes.right_y as i32);
        push_axis_if!(ABS_Z, self.last.axes.left_trigger as i32, state.axes.left_trigger as i32);
        push_axis_if!(ABS_RZ, self.last.axes.right_trigger as i32, state.axes.right_trigger as i32);

        if count > 0 {
            push_event!(EV_SYN, SYN_REPORT, 0);
            let len = count * EVENT_SIZE;
            self.file
                .write_all(&batch[..len])
                .context("failed to emit batched uinput events")?;
        }

        self.last = *state;
        Ok(())
    }
}

impl Drop for VirtualGamepad {
    fn drop(&mut self) {
        if let Err(err) = ioctl_simple(self.file.as_raw_fd(), ui_dev_destroy()) {
            // ENODEV means the kernel already cleaned up on FD close — expected.
            if let Some(errno) = err.downcast_ref::<std::io::Error>()
                .and_then(|e| e.raw_os_error())
                && errno != libc::ENODEV
            {
                tracing::warn!("UI_DEV_DESTROY failed: {err:?}");
            }
        }
    }
}

fn button_code(button: GamepadButton) -> u16 {
    match button {
        GamepadButton::South => BTN_SOUTH,
        GamepadButton::East => BTN_EAST,
        GamepadButton::West => BTN_WEST,
        GamepadButton::North => BTN_NORTH,
        GamepadButton::LeftShoulder => BTN_TL,
        GamepadButton::RightShoulder => BTN_TR,
        GamepadButton::Select => BTN_SELECT,
        GamepadButton::Start => BTN_START,
        GamepadButton::Guide => BTN_MODE,
        GamepadButton::LeftThumb => BTN_THUMBL,
        GamepadButton::RightThumb => BTN_THUMBR,
        GamepadButton::DpadUp => BTN_DPAD_UP,
        GamepadButton::DpadDown => BTN_DPAD_DOWN,
        GamepadButton::DpadLeft => BTN_DPAD_LEFT,
        GamepadButton::DpadRight => BTN_DPAD_RIGHT,
    }
}

fn ioctl_set_bit(fd: RawFd, request: libc::c_ulong, bit: u16) -> Result<()> {
    let rc = unsafe { libc::ioctl(fd, request, bit as libc::c_int) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error()).context("uinput capability ioctl failed");
    }
    Ok(())
}

fn ioctl_simple(fd: RawFd, request: libc::c_ulong) -> Result<()> {
    let rc = unsafe { libc::ioctl(fd, request) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error()).context("uinput ioctl failed");
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
    ((dir << IOC_DIRSHIFT)
        | (type_ << IOC_TYPESHIFT)
        | (nr << IOC_NRSHIFT)
        | (size << IOC_SIZESHIFT)) as libc::c_ulong
}

const fn io(type_: u8, nr: u8) -> libc::c_ulong {
    ioc(IOC_NONE, type_ as u64, nr as u64, 0)
}

const fn iow_int(type_: u8, nr: u8) -> libc::c_ulong {
    ioc(
        IOC_WRITE,
        type_ as u64,
        nr as u64,
        std::mem::size_of::<libc::c_int>() as u64,
    )
}

const fn ui_dev_create() -> libc::c_ulong {
    io(b'U', 1)
}

const fn ui_dev_destroy() -> libc::c_ulong {
    io(b'U', 2)
}

const fn ui_set_evbit() -> libc::c_ulong {
    iow_int(b'U', 100)
}

const fn ui_set_keybit() -> libc::c_ulong {
    iow_int(b'U', 101)
}

const fn ui_set_absbit() -> libc::c_ulong {
    iow_int(b'U', 103)
}
