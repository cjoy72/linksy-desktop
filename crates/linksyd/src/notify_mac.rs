//! Notifications on macOS, through Notification Center (osascript's `display notification`):
//! banners with no buttons. A request (pairing, files, a session) is answered in the
//! terminal with `linksy watch`, which the banner says.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use linksy_core::{CallState, Direction, Event, Fingerprint, Node, SessionKind, Uuid};
use linksy_i18n::t;
use tokio::sync::broadcast::error::RecvError;
use tracing::debug;

/// Battery level, in percent, at which a device's low battery is announced (once).
const LOW_BATTERY: u8 = 15;

/// A request was answered somewhere: nothing to take down, banners carry no buttons.
pub(crate) fn answered(_request: Uuid) {}

pub async fn run(node: Node) {
    let mut events = node.subscribe();
    // Incoming transfers: remember who sent what so completion can say it.
    let mut incoming: HashMap<Uuid, (String, bool)> = HashMap::new();
    // Devices already warned about a low battery; cleared once they charge or recover.
    let mut warned: HashSet<Fingerprint> = HashSet::new();
    let how = || t!("Answer in a terminal: linksy watch").to_string();
    loop {
        let event = match events.recv().await {
            Ok(e) => e,
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => break,
        };
        match event {
            Event::PairingRequest { device, code, .. } => {
                let body = t!("Check that {device} shows the code {code}.", device = device.name, code = spaced(&code));
                show(t!("Pair with {device}?", device = device.name), format!("{body}\n{}", how()));
            }
            Event::TransferRequest { from, files, total, .. } => {
                let what = match files.as_slice() {
                    [one] => display_name(&one.path),
                    many => t!("{n} items", n = many.len()),
                };
                show(t!("{device} wants to send you files", device = from.name), format!("{what} · {}\n{}", human_size(total), how()));
            }
            Event::SessionRequest { from, kind, .. } => {
                let summary = match kind {
                    SessionKind::Control => t!("{device} wants to control your screen", device = from.name),
                    SessionKind::Camera => t!("{device} wants to use your camera", device = from.name),
                    SessionKind::Display => t!("{device} wants to use this screen as a second display", device = from.name),
                    SessionKind::Microphone => t!("{device} wants to use your microphone", device = from.name),
                    SessionKind::Input => t!("{device} wants to use your mouse and keyboard", device = from.name),
                    SessionKind::Screen => t!("{device} wants to see your screen", device = from.name),
                    SessionKind::Terminal => t!("{device} wants a terminal on this computer", device = from.name),
                    // Never asked of a computer: the core answers that this one can't
                    SessionKind::Sound => continue,
                };
                show(summary, how());
            }
            Event::TransferStarted { transfer, peer, direction: Direction::Incoming, import, .. } => {
                incoming.insert(transfer, (peer.name, import));
            }
            Event::TransferComplete { transfer, saved } => match incoming.remove(&transfer) {
                Some((from, true)) if !saved.is_empty() => {
                    let summary = if saved.len() == 1 { t!("Imported 1 photo from {device}", device = from) } else { t!("Imported {n} photos from {device}", n = saved.len(), device = from) };
                    show(summary, saved[0].parent().map(|p| p.display().to_string()).unwrap_or_default());
                }
                Some((from, false)) => {
                    let summary = match saved.as_slice() {
                        [one] => t!("Received {name}", name = file_name(one)),
                        many => t!("Received {n} items", n = many.len()),
                    };
                    show(summary, t!("From {device}", device = from));
                }
                _ => {}
            },
            Event::TransferFailed { transfer, reason } => {
                if let Some((from, _)) = incoming.remove(&transfer)
                    && reason != "cancelled"
                {
                    show(t!("Transfer from {device} failed", device = from), capitalize(&reason));
                }
            }
            Event::Paired { device } => {
                show(t!("Paired with {device}", device = device.name), t!("You can now share files and more.").into());
            }
            Event::Notification { from, title, body } => {
                let body = if body.is_empty() || !body.contains(&from.name) { format!("{body}\n{}", t!("From {device}", device = from.name)) } else { body };
                show(title, body.trim().to_string());
            }
            Event::Battery { from, battery } if battery.level <= LOW_BATTERY && !battery.charging => {
                if warned.insert(from.id) {
                    show(t!("{device} is low on battery", device = from.name), t!("{level}% left", level = battery.level));
                }
            }
            Event::Battery { from, battery } if battery.charging || battery.level > LOW_BATTERY + 5 => {
                warned.remove(&from.id);
            }
            Event::CallState { from, state: CallState::Ringing, contact } => {
                let summary = match contact {
                    Some(c) => t!("Call from {contact}", contact = c),
                    None => t!("Incoming call").to_string(),
                };
                show(summary, t!("On {device}", device = from.name));
            }
            Event::Ring { from, on: true } => {
                show(t!("{device} is looking for this computer", device = from.name), t!("Here it is!").into());
                // Notification Center's sound is the user's choice; a beep is heard anyway
                tokio::task::spawn_blocking(|| {
                    let _ = crate::mac::osascript("beep 3");
                });
            }
            _ => {}
        }
    }
}

pub(crate) fn show(summary: String, body: String) {
    tokio::task::spawn_blocking(move || {
        if let Err(e) = crate::mac::notify(&summary, &body) {
            debug!("notification failed: {e}");
        }
    });
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn display_name(rel: &str) -> String {
    rel.rsplit('/').next().unwrap_or(rel).to_string()
}

fn spaced(code: &str) -> String {
    if code.len() == 6 { format!("{} {}", &code[..3], &code[3..]) } else { code.to_string() }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1000.0 && unit < UNITS.len() - 1 {
        v /= 1000.0;
        unit += 1;
    }
    if v < 10.0 { format!("{v:.1} {}", UNITS[unit]) } else { format!("{v:.0} {}", UNITS[unit]) }
}
