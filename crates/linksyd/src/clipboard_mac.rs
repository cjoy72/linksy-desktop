//! Universal clipboard on macOS: text a paired device shares goes on the clipboard through
//! `pbcopy`, an image or a file through AppleScript (`set the clipboard to…`), and devices
//! that watch this clipboard (`clipboard_watch`) hear every new text copied here, read
//! with `pbpaste` once a second.

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use linksy_core::{Event, Fingerprint, Node};
use linksy_i18n::t;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// How often the clipboard is read for devices watching it.
const POLL: Duration = Duration::from_secs(1);

pub async fn run(node: Node, notifications: bool) {
    let mut events = node.subscribe();
    let mut watchers: HashSet<Fingerprint> = HashSet::new();
    let mut changes: Option<mpsc::UnboundedReceiver<String>> = None;
    // The text last on the clipboard, so what a device sent isn't sent straight back.
    let mut last = String::new();
    loop {
        let event = tokio::select! {
            e = events.recv() => e,
            Some(text) = async { changes.as_mut()?.recv().await } => {
                if text != last {
                    last = text;
                    for to in &watchers {
                        if let Err(e) = node.send_clipboard(to, &last).await {
                            debug!("clipboard to {to} failed: {e}");
                        }
                    }
                }
                continue;
            }
        };
        match event {
            Ok(Event::Clipboard { from, text }) => {
                last = text.clone();
                let from = from.name;
                tokio::task::spawn_blocking(move || {
                    let copied = copy_text(&text);
                    match &copied {
                        Ok(()) => info!("copied clipboard from {from}"),
                        Err(e) => debug!("copy failed: {e}"),
                    }
                    if notifications {
                        let mut preview: String = text.chars().take(120).collect();
                        if preview.len() < text.len() {
                            preview = format!("{}…", preview.trim_end());
                        }
                        let summary = if copied.is_ok() { t!("Copied from {device}", device = from) } else { t!("Clipboard from {device}", device = from) };
                        crate::notify::show(summary, preview);
                    }
                });
            }
            Ok(Event::ClipboardFile { from, path }) => {
                let from = from.name;
                tokio::task::spawn_blocking(move || {
                    let copied = copy_file(&path);
                    match &copied {
                        // Not its name: logs leave out what people share
                        Ok(()) => info!("copied a file from {from}"),
                        Err(e) => warn!("copying a file from {from} failed: {e}"),
                    }
                    if notifications {
                        let body = match (copied.is_ok(), image_type(&path)) {
                            (true, Some(_)) => t!("Image ready to paste").to_string(),
                            (true, None) => t!("{name} ready to paste", name = file_name(&path)),
                            (false, _) => t!("Couldn't copy {name}", name = file_name(&path)),
                        };
                        crate::notify::show(t!("Copied from {device}", device = from), body);
                    }
                });
            }
            Ok(Event::ClipboardWatch { from, on }) => {
                if !on {
                    watchers.remove(&from.id);
                    continue;
                }
                if changes.is_none() {
                    changes = watch();
                }
                if changes.is_some() {
                    info!("{} watches the clipboard", from.name);
                    watchers.insert(from.id);
                }
            }
            Ok(_) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => break,
        }
    }
}

/// `pbpaste` reads the clipboard from any process: watching is always possible here.
pub fn can_watch() -> bool {
    true
}

/// Every new text on the clipboard, read once a second. None if `pbpaste` can't run.
fn watch() -> Option<mpsc::UnboundedReceiver<String>> {
    // What's there already isn't news
    let mut last = pbpaste()?;
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(POLL);
            let Some(now) = pbpaste() else { continue };
            if now == last {
                continue;
            }
            last = now.clone();
            if !now.is_empty() && tx.send(now).is_err() {
                break;
            }
        }
        debug!("clipboard watch ended");
    });
    Some(rx)
}

/// The clipboard's text (empty when it holds something else), or None when it can't be read.
fn pbpaste() -> Option<String> {
    let out = Command::new("pbpaste").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

pub(crate) fn copy_text(text: &str) -> std::io::Result<()> {
    let mut child = Command::new("pbcopy").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    child.stdin.take().expect("piped stdin").write_all(text.as_bytes())?;
    match child.wait()? {
        s if s.success() => Ok(()),
        s => Err(std::io::Error::other(format!("pbcopy exited with {s}"))),
    }
}

/// An image goes on the clipboard as one, so it pastes into a chat or editor; anything else
/// as a file, so it pastes into the Finder.
fn copy_file(path: &Path) -> std::io::Result<()> {
    let file = crate::mac::quote(&path.to_string_lossy());
    let script = match image_type(path) {
        Some("image/png") => format!("set the clipboard to (read (POSIX file {file}) as «class PNGf»)"),
        Some("image/jpeg") => format!("set the clipboard to (read (POSIX file {file}) as JPEG picture)"),
        Some("image/gif") => format!("set the clipboard to (read (POSIX file {file}) as GIF picture)"),
        Some("image/tiff") => format!("set the clipboard to (read (POSIX file {file}) as TIFF picture)"),
        _ => format!("tell application \"Finder\" to set the clipboard to (POSIX file {file})"),
    };
    crate::mac::osascript(&script).map(|_| ())
}

fn image_type(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "tif" | "tiff" => "image/tiff",
        _ => return None,
    })
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_are_told_by_their_extension() {
        assert_eq!(image_type(Path::new("x.PNG")), Some("image/png"));
        assert_eq!(image_type(Path::new("x.pdf")), None);
    }
}
