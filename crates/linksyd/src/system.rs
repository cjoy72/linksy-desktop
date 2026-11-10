//! Where things live and what this machine is called, following the XDG conventions
//! (on Windows, its own: the roaming app data, the user's Downloads and Pictures).

#[cfg(windows)]
pub use self::windows::*;
#[cfg(not(windows))]
pub use self::xdg::*;

#[cfg(windows)]
mod windows {
    use std::path::PathBuf;

    use linksy_core::DeviceKind;

    fn profile() -> PathBuf {
        std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
    }

    /// `%APPDATA%\Linksy`: identity and paired devices.
    pub fn data_dir() -> PathBuf {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| profile().join("AppData").join("Roaming")).join("Linksy")
    }

    /// `Downloads\Linksy`, like the phone's Downloads/Linksy.
    pub fn download_dir() -> PathBuf {
        profile().join("Downloads").join("Linksy")
    }

    pub fn pictures_dir() -> PathBuf {
        profile().join("Pictures")
    }

    /// The computer's name, as Windows shows it.
    pub fn device_name() -> String {
        std::env::var("COMPUTERNAME").ok().filter(|n| !n.is_empty()).unwrap_or_else(|| "Windows computer".into())
    }

    /// Open a web link, file or folder with what Windows opens it with.
    pub fn open(target: &std::ffi::OsStr) -> std::io::Result<()> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let target: Vec<u16> = target.encode_wide().chain(Some(0)).collect();
        let verb: Vec<u16> = "open\0".encode_utf16().collect();
        // SAFETY: NUL-terminated strings; no window, parameters or folder
        let result = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
        // Above 32 is success
        if result as isize > 32 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
    }

    /// A machine with a battery is a laptop.
    pub fn device_kind() -> DeviceKind {
        use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        // SAFETY: fills in the struct it's given
        let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
        let ok = unsafe { GetSystemPowerStatus(&mut status) } != 0;
        // 128: no system battery; 255: unknown
        if ok && status.BatteryFlag & 128 == 0 && status.BatteryFlag != 255 { DeviceKind::Laptop } else { DeviceKind::Desktop }
    }
}

#[cfg(not(windows))]
mod xdg {
use std::path::PathBuf;

use linksy_core::DeviceKind;

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(fallback))
}

/// `$XDG_DATA_HOME/linksy`: identity and paired devices.
pub fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("linksy")
}

/// Where received files go by default: a Linksy folder in the user's Downloads (as
/// configured in `user-dirs.dirs`), like the phone's Downloads/Linksy.
pub fn download_dir() -> PathBuf {
    user_dir("XDG_DOWNLOAD_DIR", "Downloads").join("Linksy")
}

/// The user's Pictures folder, where phones' photos are imported.
pub fn pictures_dir() -> PathBuf {
    user_dir("XDG_PICTURES_DIR", "Pictures")
}

fn user_dir(key: &str, fallback: &str) -> PathBuf {
    let config = xdg("XDG_CONFIG_HOME", ".config").join("user-dirs.dirs");
    if let Ok(text) = std::fs::read_to_string(config) {
        for line in text.lines() {
            if let Some(v) = line.strip_prefix(key).and_then(|v| v.strip_prefix('=')) {
                let v = v.trim().trim_matches('"').replace("$HOME", &home().to_string_lossy());
                return PathBuf::from(v);
            }
        }
    }
    home().join(fallback)
}

/// The pretty hostname ("Sam's Laptop") if set, else the plain hostname.
pub fn device_name() -> String {
    if let Ok(info) = std::fs::read_to_string("/etc/machine-info") {
        for line in info.lines() {
            if let Some(v) = line.strip_prefix("PRETTY_HOSTNAME=") {
                let v = v.trim().trim_matches('"');
                if !v.is_empty() {
                    return v.to_string();
                }
            }
        }
    }
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|h| h.trim().to_string())
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "Linux computer".into())
}

/// A machine with a battery is a laptop.
pub fn device_kind() -> DeviceKind {
    let has_battery = std::fs::read_dir("/sys/class/power_supply")
        .map(|entries| entries.flatten().any(|e| e.file_name().to_string_lossy().starts_with("BAT")))
        .unwrap_or(false);
    if has_battery { DeviceKind::Laptop } else { DeviceKind::Desktop }
}
}
