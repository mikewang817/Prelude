//! Minimal termios handling: raw mode for single keypresses and for the
//! width probe. Avoids a dependency for two ioctls.

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Termios {
    c_iflag: u64,
    c_oflag: u64,
    c_cflag: u64,
    c_lflag: u64,
    c_cc: [u8; 20],
    c_ispeed: u64,
    c_ospeed: u64,
}

unsafe extern "C" {
    unsafe fn tcgetattr(fd: i32, t: *mut Termios) -> i32;
    unsafe fn tcsetattr(fd: i32, act: i32, t: *const Termios) -> i32;
}

const ICANON: u64 = 0x0000_0100;
const ECHO: u64 = 0x0000_0008;
const TCSADRAIN: i32 = 1;

pub fn raw_mode() -> Option<Termios> {
    let mut t = std::mem::MaybeUninit::<Termios>::uninit();
    if unsafe { tcgetattr(0, t.as_mut_ptr()) } != 0 {
        return None;
    }
    let saved = unsafe { t.assume_init() };
    let mut raw = saved;
    raw.c_lflag &= !(ICANON | ECHO);
    raw.c_cc[16] = 1; // VMIN
    raw.c_cc[17] = 0; // VTIME
    unsafe { tcsetattr(0, TCSADRAIN, &raw) };
    Some(saved)
}

pub fn restore(saved: Option<Termios>) {
    if let Some(s) = saved {
        unsafe { tcsetattr(0, TCSADRAIN, &s) };
    }
}
