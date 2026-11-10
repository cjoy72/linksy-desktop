//! The tray icon on Windows: Linksy's whole face there until it has a window of its own. Its
//! menu sends files and the clipboard to each paired device, rings it, pairs a new phone (a QR
//! code to scan), opens the received files, and starts Linksy at sign-in.
//!
//! Windows tray icons belong to a thread with a message loop: the icon and its menu live on a
//! thread of their own, which hears the menu's clicks and the service's news.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use linksy_core::{DeviceInfo, DeviceKind, DeviceView, Event, Node};
use linksy_i18n::t;
use linksy_ipc::Request;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use tray_icon::menu::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::clipboard::Copied;
use crate::history::History;
use crate::lock::Away;
use crate::mirror::Mirror;
use crate::photos::Photos;
use crate::sessions::Sessions;

/// Bigger copied files go as a transfer, as in the apps.
const MAX_CLIPBOARD_FILE: u64 = 10 << 20;
/// Where Windows starts programs at sign-in, for this user.
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Linksy";

/// What the service needs to act on the menu: the same calls the apps make.
#[derive(Clone)]
pub struct Service {
    pub node: Node,
    pub history: History,
    pub sessions: Sessions,
    pub mirror: Mirror,
    pub away: Away,
    pub photos: Photos,
}

impl Service {
    async fn ask(&self, req: Request) -> Result<Value, String> {
        crate::ipc::handle(&self.node, &self.history, &self.sessions, &self.mirror, &self.away, &self.photos, req).await
    }
}

/// What the menu shows.
#[derive(Clone, Default, PartialEq)]
struct Shown {
    /// (device, connected), as the menu lists them
    devices: Vec<(DeviceInfo, bool)>,
    at_sign_in: bool,
}

/// A menu choice, done by the service.
#[derive(Clone)]
enum Action {
    SendFiles(DeviceInfo),
    SendClipboard(DeviceInfo),
    Ring(DeviceInfo),
    Pair,
    OpenDownloads,
    AtSignIn(bool),
    Quit,
}

pub async fn run(service: Service) {
    let (actions, mut todo) = mpsc::unbounded_channel::<Action>();
    let (updates, news) = std_mpsc::channel::<Shown>();
    let first = shown(&service);
    let mut last = first.clone();
    std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || ui(first, news, actions))
        .expect("tray thread");
    let mut events = service.node.subscribe();
    // Forgetting a device sends no event of its own
    let mut every = tokio::time::interval(Duration::from_secs(30));
    loop {
        tokio::select! {
            Some(action) = todo.recv() => {
                let service = service.clone();
                let updates = updates.clone();
                tokio::spawn(async move {
                    act(&service, action).await;
                    // A setting the menu shows may have changed
                    let _ = updates.send(shown(&service));
                });
            }
            event = events.recv() => match event {
                Ok(Event::DeviceFound { .. } | Event::DeviceLost { .. } | Event::Connected { .. } | Event::Disconnected { .. }
                    | Event::Paired { .. } | Event::DeviceUpdated { .. })
                | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Several come together (a device connecting): once they settle
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    let now = shown(&service);
                    if now != last {
                        last = now.clone();
                        if updates.send(now).is_err() {
                            break;
                        }
                    }
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            _ = every.tick() => {
                let now = shown(&service);
                if now != last {
                    last = now.clone();
                    let _ = updates.send(now);
                }
            }
        }
    }
}

fn shown(service: &Service) -> Shown {
    let mut devices: Vec<DeviceView> = service.node.devices().into_iter().filter(|d| d.trusted).collect();
    devices.sort_by(|a, b| b.connected.cmp(&a.connected).then_with(|| a.device.name.to_lowercase().cmp(&b.device.name.to_lowercase())));
    Shown { devices: devices.into_iter().map(|d| (d.device, d.connected)).collect(), at_sign_in: starts_at_sign_in() }
}

/// The tray's own thread: the icon, its menu, and Windows' messages for them.
fn ui(first: Shown, news: std_mpsc::Receiver<Shown>, actions: mpsc::UnboundedSender<Action>) {
    let (menu, mut choices) = build_menu(&first);
    let tray = TrayIconBuilder::new()
        .with_icon(icon(connected(&first) > 0))
        .with_tooltip(tooltip(&first))
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(true)
        .build();
    let tray: TrayIcon = match tray {
        Ok(t) => t,
        Err(e) => {
            warn!("no tray icon: {e}");
            return;
        }
    };
    info!("tray icon ready");
    loop {
        pump();
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if let Some(action) = choices.get(&event.id) {
                if actions.send(action.clone()).is_err() {
                    return;
                }
            }
        }
        // A click opens the menu; a double click, the received files
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } = event {
                let _ = actions.send(Action::OpenDownloads);
            }
        }
        match news.try_recv() {
            Ok(mut now) => {
                // Only the newest of several counts
                while let Ok(newer) = news.try_recv() {
                    now = newer;
                }
                let (menu, ids) = build_menu(&now);
                choices = ids;
                tray.set_menu(Some(Box::new(menu)));
                let _ = tray.set_icon(Some(icon(connected(&now) > 0)));
                let _ = tray.set_tooltip(Some(tooltip(&now)));
            }
            Err(std_mpsc::TryRecvError::Empty) => {}
            // The service is stopping
            Err(std_mpsc::TryRecvError::Disconnected) => return,
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// Hand this thread's waiting Windows messages to their windows (the icon's and its menu's).
fn pump() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage};
    // SAFETY: a plain message loop on this thread's own queue
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn connected(s: &Shown) -> usize {
    s.devices.iter().filter(|(_, on)| *on).count()
}

fn tooltip(s: &Shown) -> String {
    let status = match connected(s) {
        0 => t!("No devices connected").to_string(),
        1 => t!("1 device connected").to_string(),
        n => t!("{n} devices connected", n),
    };
    format!("Linksy · {status}")
}

/// The menu, and what each of its items does.
fn build_menu(s: &Shown) -> (Menu, HashMap<MenuId, Action>) {
    let mut ids = HashMap::new();
    let mut n = 0;
    let mut item = |label: &str, enabled: bool, action: Action| {
        n += 1;
        let id = MenuId::new(format!("linksy-{n}"));
        ids.insert(id.clone(), action);
        MenuItem::with_id(id, label, enabled, None)
    };
    let menu = Menu::new();
    let _ = menu.append(&MenuItem::new(tooltip(s).trim_start_matches("Linksy · "), false, None));
    let _ = menu.append(&PredefinedMenuItem::separator());
    for (info, on) in &s.devices {
        let send = item(&t!("Send Files…"), *on, Action::SendFiles(info.clone()));
        let clip = item(&t!("Send Clipboard"), *on, Action::SendClipboard(info.clone()));
        let ring = item(&t!("Ring"), *on, Action::Ring(info.clone()));
        let mut entries: Vec<&dyn IsMenuItem> = vec![&send, &clip];
        let phone = matches!(info.kind, DeviceKind::Phone | DeviceKind::Tablet);
        if phone {
            entries.push(&ring);
        }
        let label = if *on { info.name.clone() } else { t!("{name} (not connected)", name = info.name) };
        let sub = Submenu::with_items(label, true, &entries).expect("device menu");
        let _ = menu.append(&sub);
    }
    if !s.devices.is_empty() {
        let _ = menu.append(&PredefinedMenuItem::separator());
    }
    let _ = menu.append(&item(&t!("Pair a Phone…"), true, Action::Pair));
    let _ = menu.append(&item(&t!("Open Received Files"), true, Action::OpenDownloads));
    n += 1;
    let id = MenuId::new(format!("linksy-{n}"));
    ids.insert(id.clone(), Action::AtSignIn(!s.at_sign_in));
    let _ = menu.append(&CheckMenuItem::with_id(id, t!("Start at Sign-in"), true, s.at_sign_in, None));
    let _ = menu.append(&PredefinedMenuItem::separator());
    n += 1;
    let id = MenuId::new(format!("linksy-{n}"));
    ids.insert(id.clone(), Action::Quit);
    let _ = menu.append(&MenuItem::with_id(id, t!("Quit"), true, None));
    (menu, ids)
}

/// The tray picture: filled while a device is connected, hollow otherwise, in the taskbar's
/// colour (white on the usual dark one).
fn icon(connected: bool) -> Icon {
    let png: &[u8] = match (connected, light_taskbar()) {
        (true, false) => include_bytes!("../../../packaging/windows/tray-dark.png"),
        (false, false) => include_bytes!("../../../packaging/windows/tray-idle-dark.png"),
        (true, true) => include_bytes!("../../../packaging/windows/tray-light.png"),
        (false, true) => include_bytes!("../../../packaging/windows/tray-idle-light.png"),
    };
    let picture = image::load_from_memory_with_format(png, image::ImageFormat::Png).expect("tray icon").to_rgba8();
    let (w, h) = picture.dimensions();
    Icon::from_rgba(picture.into_raw(), w, h).expect("tray icon")
}

/// Whether the taskbar is light (Settings › Personalization › Colors).
fn light_taskbar() -> bool {
    read_dword(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "SystemUsesLightTheme") == Some(1)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn read_dword(key: &str, value: &str) -> Option<u32> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let (key, value) = (wide(key), wide(value));
    let mut data = 0u32;
    let mut size = 4u32;
    // SAFETY: NUL-terminated names, and a u32 of the size given for the value
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&raw mut data).cast(), &mut size) };
    (status == 0).then_some(data)
}

/// Whether Windows starts this service at sign-in.
fn starts_at_sign_in() -> bool {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
    let (key, value) = (wide(RUN_KEY), wide(RUN_VALUE));
    // SAFETY: asks only whether the value is there (no buffer)
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
    status == 0
}

/// Start this service at sign-in, or stop doing so.
fn set_at_sign_in(on: bool) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW};
    let (key, value) = (wide(RUN_KEY), wide(RUN_VALUE));
    let status = if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let command = wide(&format!("\"{}\"", exe.display()));
        // SAFETY: NUL-terminated strings; the data is the command's UTF-16 with its NUL
        unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), REG_SZ, command.as_ptr().cast(), (command.len() * 2) as u32) }
    } else {
        // SAFETY: NUL-terminated names
        match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) } {
            2 => 0, // already gone
            s => s,
        }
    };
    if status == 0 { Ok(()) } else { Err(format!("Windows said no (error {status})")) }
}

async fn act(service: &Service, action: Action) {
    let (who, result) = match action {
        Action::Quit => (None, service.ask(Request::Quit).await),
        Action::SendFiles(d) => {
            let title = t!("Send to {name}", name = d.name);
            let picked = tokio::task::spawn_blocking(move || rfd::FileDialog::new().set_title(title).pick_files()).await.ok().flatten();
            match picked {
                Some(paths) if !paths.is_empty() => (Some(d.name), service.ask(Request::Send { to: d.id.to_string(), paths }).await),
                _ => return,
            }
        }
        Action::SendClipboard(d) => {
            let sent = send_clipboard(service, &d).await;
            if sent.is_ok() {
                crate::notify::show(d.name.clone(), t!("Clipboard sent to {name}", name = d.name));
            }
            (Some(d.name), sent)
        }
        Action::Ring(d) => (Some(d.name.clone()), service.ask(Request::Ring { to: d.id.to_string(), on: true }).await),
        Action::Pair => (None, pair(service).await),
        Action::OpenDownloads => {
            let dir = service.node.settings().download_dir;
            let _ = std::fs::create_dir_all(&dir);
            (None, crate::system::open(dir.as_os_str()).map(|()| Value::Null).map_err(|e| e.to_string()))
        }
        Action::AtSignIn(on) => (None, set_at_sign_in(on).map(|()| Value::Null)),
    };
    // Nowhere else to say it went wrong: a notification
    if let Err(e) = result {
        crate::notify::show(who.unwrap_or_else(|| "Linksy".into()), e);
    }
}

/// An invitation for a phone to scan: its QR code as a picture, opened in the photo viewer.
async fn pair(service: &Service) -> Result<Value, String> {
    let uri = match service.ask(Request::CreateInvite).await? {
        Value::String(uri) => uri,
        _ => return Err("no invitation came back".into()),
    };
    let dir = linksy_ipc::runtime_dir().unwrap_or_else(std::env::temp_dir);
    let path = dir.join("Pair with Linksy.png");
    qr_png(&uri, &path)?;
    crate::system::open(path.as_os_str()).map_err(|e| e.to_string())?;
    crate::notify::show(t!("Pair a Phone").into(), t!("In Linksy on your phone, scan the code that just opened.").into());
    Ok(Value::Null)
}

/// `text` as a QR code picture: black on white, with the quiet zone around it.
fn qr_png(text: &str, path: &Path) -> Result<(), String> {
    let code = qrcode::QrCode::new(text.as_bytes()).map_err(|e| e.to_string())?;
    let width = code.width();
    let (quiet, scale) = (4, 12);
    let side = ((width + 2 * quiet) * scale) as u32;
    let colors = code.to_colors();
    let picture = image::GrayImage::from_fn(side, side, |x, y| {
        let (cx, cy) = ((x as usize / scale).wrapping_sub(quiet), (y as usize / scale).wrapping_sub(quiet));
        let dark = cx < width && cy < width && colors[cy * width + cx] == qrcode::Color::Dark;
        image::Luma([if dark { 0 } else { 255 }])
    });
    picture.save(path).map_err(|e| e.to_string())
}

/// As the app's Send Clipboard: one small copied file or an image onto the device's
/// clipboard, more files as a transfer, else the text.
async fn send_clipboard(service: &Service, d: &DeviceInfo) -> Result<Value, String> {
    let to = d.id.to_string();
    let copied = tokio::task::spawn_blocking(crate::clipboard::read_local).await.ok().flatten();
    match copied {
        Some(Copied::Text(text)) => service.ask(Request::Clipboard { to, text }).await,
        Some(Copied::Files(paths)) => {
            let small = paths.len() == 1 && std::fs::metadata(&paths[0]).is_ok_and(|m| m.is_file() && m.len() <= MAX_CLIPBOARD_FILE);
            if small && can(d, "clipboard_file") {
                service.ask(Request::ClipboardFile { to, path: paths.into_iter().next().unwrap() }).await
            } else {
                service.ask(Request::Send { to, paths }).await
            }
        }
        Some(Copied::Png(png)) => {
            if !can(d, "clipboard_file") {
                return Err(t!("{name} can't take images on its clipboard yet", name = d.name));
            }
            let path: PathBuf = linksy_ipc::runtime_dir().unwrap_or_else(std::env::temp_dir).join("Clipboard image.png");
            std::fs::write(&path, png).map_err(|_| t!("Couldn't read the image on the clipboard").to_string())?;
            service.ask(Request::ClipboardFile { to, path }).await
        }
        None => Err(t!("There's nothing to send on the clipboard").to_string()),
    }
}

/// Whether a device can do something: what it says, or for an older phone that says
/// nothing, all but what came later (as the apps decide).
fn can(info: &DeviceInfo, feature: &str) -> bool {
    if !info.features.is_empty() {
        return info.features.iter().any(|f| f == feature);
    }
    !(info.os == "android" && ["microphone", "locate"].contains(&feature))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_qr_code_picture_is_square_and_black_and_white() {
        let dir = std::env::temp_dir().join(format!("linksy-qr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("qr.png");
        qr_png("linksy://pair?x=1", &path).unwrap();
        let picture = image::open(&path).unwrap().to_luma8();
        assert_eq!(picture.width(), picture.height());
        assert!(picture.pixels().all(|p| p.0[0] == 0 || p.0[0] == 255));
        // The quiet zone is white
        assert_eq!(picture.get_pixel(0, 0).0[0], 255);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_menu_has_an_entry_per_device_and_its_own() {
        let (_, ids) = build_menu(&Shown::default());
        // Pair, Open Received Files, Start at Sign-in, Quit
        assert_eq!(ids.len(), 4);
        debug!("menu ok");
    }
}
