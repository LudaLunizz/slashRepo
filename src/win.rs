//! The Windows console calls the exe needs. repo-status.exe is a windowed program, so a double
//! click or the Start Menu shows only the window (no console flashing up). The text modes
//! (`--text`, `--tabs`, `install`, ...) borrow the console of the terminal they were started from.
#![allow(unsafe_code)]

use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE};

/// Print to the terminal that started us. Does nothing when stdout already goes somewhere (a pipe
/// from an agent's hook runner or `| more`) or when there is no terminal.
pub fn attach_parent_console() {
    // SAFETY: GetStdHandle takes a constant and only reads this process's handle table.
    let out = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if out.is_null() || out as isize == -1 {
        // SAFETY: plain call with a constant; failure (no parent console) is harmless and ignored.
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    }
}

/// The local time as HH:MM ("checked 14:05").
pub fn local_hhmm() -> String {
    // SAFETY: GetLocalTime only writes the SYSTEMTIME it is given.
    let t = unsafe {
        let mut t = std::mem::zeroed::<windows_sys::Win32::Foundation::SYSTEMTIME>();
        windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut t);
        t
    };
    format!("{:02}:{:02}", t.wHour, t.wMinute)
}
