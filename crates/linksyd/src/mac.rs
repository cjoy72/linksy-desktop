//! macOS: what the service does through the system's own tools (osascript, pbcopy, open,
//! pmset…) where Linux goes through D-Bus and the desktop's. Nothing here needs a window
//! or a bundle: the service runs from a launchd agent, the CLI from a terminal.

use std::process::{Command, Stdio};

/// Run one line of AppleScript and return what it printed.
pub fn osascript(script: &str) -> std::io::Result<String> {
    let out = Command::new("osascript").arg("-e").arg(script).stdin(Stdio::null()).stderr(Stdio::piped()).output()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    } else {
        Err(std::io::Error::other(String::from_utf8_lossy(&out.stderr).trim().to_string()))
    }
}

/// `text` as an AppleScript string literal, so what a device sends can't become script.
pub fn quote(text: &str) -> String {
    let mut s = String::with_capacity(text.len() + 2);
    s.push('"');
    for c in text.chars() {
        match c {
            '\\' => s.push_str("\\\\"),
            '"' => s.push_str("\\\""),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            c => s.push(c),
        }
    }
    s.push('"');
    s
}

/// A banner in Notification Center. macOS files it under the program that asked (Script
/// Editor's name shows on it): the service has no app bundle to own notifications with.
pub fn notify(title: &str, body: &str) -> std::io::Result<()> {
    osascript(&format!("display notification {} with title {}", quote(body), quote(title))).map(|_| ())
}

/// Lock the screen: the display goes to sleep, and with "Require password after sleep or
/// screen saver begins" (on by default) it asks for the password when it wakes. Whether it
/// worked.
pub async fn lock_screen() -> bool {
    tokio::process::Command::new("pmset").arg("displaysleepnow").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().await.is_ok_and(|s| s.success())
}

/// The computer's name as System Settings shows it ("Sam's MacBook Pro"), else its host name.
pub fn computer_name() -> Option<String> {
    for args in [["--get", "ComputerName"], ["--get", "LocalHostName"]] {
        if let Some(name) = output("scutil", &args).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
            return Some(name);
        }
    }
    None
}

/// Whether this Mac runs on a battery (a MacBook).
pub fn has_battery() -> bool {
    output("pmset", &["-g", "batt"]).is_some_and(|s| s.contains("InternalBattery"))
}

/// The interface the default route goes out through ("en0", or a VPN's "utun3").
pub fn default_interface() -> Option<String> {
    let out = output("route", &["-n", "get", "default"])?;
    out.lines().find_map(|l| l.trim().strip_prefix("interface:")).map(|i| i.trim().to_string()).filter(|i| !i.is_empty())
}

/// What `arp -an` lists: every neighbour this computer has heard from.
pub fn arp_table() -> String {
    output("arp", &["-an"]).unwrap_or_default()
}

/// The user's account picture, from the directory service, as a path when it has one.
pub fn account_picture() -> Option<std::path::PathBuf> {
    let user = std::env::var("USER").ok()?;
    let out = output("dscl", &[".", "-read", &format!("/Users/{user}"), "Picture"])?;
    // "Picture:\n /Library/User Pictures/Fun/Smack.heic" or "Picture: /path"
    let path = out.strip_prefix("Picture:").map(str::trim).filter(|p| !p.is_empty())?;
    Some(std::path::PathBuf::from(path))
}

fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_are_quoted_for_applescript() {
        assert_eq!(quote(r#"say "hi" \ bye"#), r#""say \"hi\" \\ bye""#);
        assert_eq!(quote("two\nlines"), r#""two\nlines""#);
    }
}
