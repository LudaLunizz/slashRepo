//! The Windows console calls the exe needs. repo-status.exe is a windowed program, so a double
//! click or the Start Menu shows only the window (no console flashing up). The text modes
//! (`--text`, `--tabs`, `install`, ...) borrow the console of the terminal they were started from.
#![allow(unsafe_code)]

use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindow};

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

/// UTF-16, NUL-terminated.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// True when another Repo Status window is already open in this session; that window is brought
/// to the front, so a second `/repo` shows the open one instead of building another (the Linux
/// build does the same through GApplication). The first caller keeps a named mutex until the
/// process ends.
pub fn show_open_window(title: &str) -> bool {
    let name = wide("Local\\slashRepo-window");
    // SAFETY: a null security descriptor and a NUL-terminated name; the handle is deliberately
    // never closed, so the mutex lives as long as this process.
    let (handle, error) = unsafe {
        let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        (h, GetLastError())
    };
    if handle.is_null() || error != ERROR_ALREADY_EXISTS {
        return false;
    }
    let title = wide(title);
    // SAFETY: FindWindowW only reads the NUL-terminated title; the handle it returns is checked.
    unsafe {
        let window = FindWindowW(std::ptr::null(), title.as_ptr());
        if !window.is_null() {
            ShowWindow(window, SW_RESTORE);
            SetForegroundWindow(window);
        }
    }
    true
}
