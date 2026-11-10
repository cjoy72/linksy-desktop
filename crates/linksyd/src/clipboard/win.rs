//! The Windows clipboard, through Win32: any program may read and set it, and a change shows
//! as a new clipboard sequence number, so watching needs no window or helper tool.

use std::path::{Path, PathBuf};
use std::time::Duration;

use clipboard_win::{Clipboard, Getter, Setter, formats, raw};
use tokio::sync::mpsc;
use tracing::debug;

use super::{Copied, image_type};

/// How long to keep trying while another program holds the clipboard open.
const ATTEMPTS: usize = 10;

fn open() -> std::io::Result<Clipboard> {
    Clipboard::new_attempts(ATTEMPTS).map_err(|e| std::io::Error::other(format!("the clipboard is busy: {e}")))
}

fn err(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

/// Windows always lets a program see clipboard changes.
pub fn probe_watch() -> bool {
    true
}

pub fn can_paste() -> bool {
    true
}

/// Every new text on the clipboard: checked twice a second, by its sequence number, which
/// costs nothing until it changes.
pub fn watch() -> Option<mpsc::UnboundedReceiver<String>> {
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let mut seen = raw::seq_num();
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let now = raw::seq_num();
            if now == seen {
                continue;
            }
            seen = now;
            // The program taking it over may still be setting it up
            std::thread::sleep(Duration::from_millis(50));
            let text = open().ok().and_then(|_c| {
                let mut s = String::new();
                formats::Unicode.read_clipboard(&mut s).ok().map(|_| s)
            });
            if let Some(text) = text.filter(|t| !t.is_empty())
                && tx.send(text).is_err()
            {
                break;
            }
        }
        debug!("clipboard watch ended");
    });
    Some(rx)
}

/// Reads the clipboard once, for Send Clipboard: files first, then a picture, then text.
pub fn read_local() -> Option<Copied> {
    let _clip = open().ok()?;
    let mut files: Vec<String> = Vec::new();
    if formats::FileList.read_clipboard(&mut files).is_ok() && !files.is_empty() {
        return Some(Copied::Files(files.into_iter().map(PathBuf::from).collect()));
    }
    // Browsers and editors put a PNG of their own; everything else a bitmap
    if let Some(png) = formats::RawData(png_format()?).read_clipboard_vec() {
        return Some(Copied::Png(png));
    }
    let mut bmp = Vec::new();
    if formats::Bitmap.read_clipboard(&mut bmp).is_ok()
        && let Some(png) = bmp_to_png(&bmp)
    {
        return Some(Copied::Png(png));
    }
    let mut text = String::new();
    formats::Unicode.read_clipboard(&mut text).ok()?;
    (!text.is_empty()).then_some(Copied::Text(text))
}

trait ReadVec {
    fn read_clipboard_vec(&self) -> Option<Vec<u8>>;
}

impl ReadVec for formats::RawData {
    fn read_clipboard_vec(&self) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        self.read_clipboard(&mut out).ok().filter(|n| *n > 0).map(|_| out)
    }
}

fn png_format() -> Option<u32> {
    raw::register_format("PNG").map(|f| f.get())
}

pub(crate) fn copy_text(text: &str) -> std::io::Result<()> {
    let _clip = open()?;
    formats::Unicode.write_clipboard(&text).map_err(err)
}

/// An image goes on the clipboard as one (a bitmap, and a PNG for programs that take it),
/// so it pastes into a chat or editor; anything else as a file, for File Explorer.
pub fn copy_file(path: &Path) -> std::io::Result<()> {
    if image_type(path).is_some()
        && let Ok(bytes) = std::fs::read(path)
        && let Ok(picture) = image::load_from_memory(&bytes)
    {
        let mut bmp = std::io::Cursor::new(Vec::new());
        picture.to_rgba8().write_to(&mut bmp, image::ImageFormat::Bmp).map_err(err)?;
        let mut png = std::io::Cursor::new(Vec::new());
        picture.write_to(&mut png, image::ImageFormat::Png).map_err(err)?;
        let _clip = open()?;
        raw::empty().map_err(err)?;
        formats::Bitmap.write_clipboard(&bmp.into_inner()).map_err(err)?;
        if let Some(f) = png_format() {
            let _ = raw::set_without_clear(f, &png.into_inner());
        }
        return Ok(());
    }
    let full = std::path::absolute(path)?;
    let _clip = open()?;
    formats::FileList.write_clipboard(&[full.to_string_lossy().as_ref()]).map_err(err)
}

fn bmp_to_png(bmp: &[u8]) -> Option<Vec<u8>> {
    let picture = image::load_from_memory_with_format(bmp, image::ImageFormat::Bmp).ok()?;
    let mut png = std::io::Cursor::new(Vec::new());
    picture.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}
