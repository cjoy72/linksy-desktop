//! Windows toast notifications. Linksy registers its own app id for them (under the user's
//! classes, as unpackaged apps do), so they show as "Linksy" with its buttons; a button
//! pressed while the service runs comes back to it.

use std::path::Path;
use std::sync::Once;

use tauri_winrt_notification::{Duration, Scenario, Toast};
use tracing::debug;

use super::DESKTOP_ENTRY;

/// Register the app id toasts are shown under: its name, and its icon when one is installed
/// beside the service.
fn register() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
        let key = wide(&format!(r"Software\Classes\AppUserModelId\{DESKTOP_ENTRY}"));
        let mut values = vec![("DisplayName", "Linksy".to_string())];
        if let Some(icon) = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.join("linksy.png"))).filter(|p| p.is_file()) {
            values.push(("IconUri", icon.to_string_lossy().into_owned()));
        }
        for (name, value) in values {
            let (name, value) = (wide(name), wide(&value));
            // SAFETY: NUL-terminated strings; the data is the value's UTF-16 with its NUL
            let status = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_SZ, value.as_ptr().cast(), (value.len() * 2) as u32) };
            if status != 0 {
                debug!("couldn't register the notification app id: error {status}");
            }
        }
    });
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn toast(summary: &str, body: &str) -> Toast {
    register();
    let mut lines = body.splitn(2, '\n');
    let mut t = Toast::new(DESKTOP_ENTRY).title(summary);
    if let Some(first) = lines.next().filter(|l| !l.is_empty()) {
        t = t.text1(first);
    }
    if let Some(rest) = lines.next() {
        t = t.text2(rest);
    }
    t
}

pub fn show(summary: &str, body: &str) {
    if let Err(e) = toast(summary, body).show() {
        debug!("notification failed: {e}");
    }
}

/// A notice that stays until answered, with `buttons` (label, action). `on_action` hears the
/// button's action, or "default" for a click on the notice itself.
pub fn prompt(summary: &str, body: &str, buttons: &[(&str, &str)], urgent: bool, on_action: impl Fn(&str) + Send + 'static) -> Result<(), String> {
    let mut t = toast(summary, body);
    for (label, action) in buttons {
        t = t.add_button(label, action);
    }
    if urgent {
        // A reminder stays on screen until it's answered
        t = t.scenario(Scenario::Reminder).duration(Duration::Long);
    }
    t.on_activated(move |action| {
        on_action(action.as_deref().unwrap_or("default"));
        Ok(())
    })
    .show()
    .map_err(|e| e.to_string())
}

/// A phone's notification: its app's icon, buttons, and `dismissed` when the user swipes
/// it away (not when it times out into the notification center).
pub fn mirror(
    summary: &str,
    body: &str,
    icon: Option<&Path>,
    buttons: &[(String, String)],
    on_action: impl Fn(&str) + Send + 'static,
    dismissed: impl Fn() + Send + 'static,
) -> Result<(), String> {
    use tauri_winrt_notification::{IconCrop, ToastDismissalReason};
    let mut t = toast(summary, body);
    if let Some(icon) = icon {
        t = t.icon(icon, IconCrop::Circular, "");
    }
    for (label, action) in buttons {
        t = t.add_button(label, action);
    }
    t.on_activated(move |action| {
        on_action(action.as_deref().unwrap_or("default"));
        Ok(())
    })
    .on_dismissed(move |reason| {
        if reason == Some(ToastDismissalReason::UserCanceled) {
            dismissed();
        }
        Ok(())
    })
    .show()
    .map_err(|e| e.to_string())
}

/// Open a file or folder as File Explorer would.
pub fn open(path: &Path) {
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

/// Show a file selected in its folder.
pub fn reveal(path: &Path) {
    let mut select = std::ffi::OsString::from("/select,");
    select.push(path);
    let _ = std::process::Command::new("explorer").arg(select).spawn();
}
