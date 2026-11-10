# Linksy on Windows (unofficial fork)

This branch runs Linksy's background service on Windows 10 and 11. It is an unofficial port: the
upstream project ([LinkSy-mg/linksy-desktop](https://github.com/LinkSy-mg/linksy-desktop)) is
Linux-only and has no plans for Windows. Like upstream, it is licensed under the GNU AGPL v3.0 or
later.

Your phone needs Linksy for Android from
[linksy-releases](https://github.com/LinkSy-mg/linksy-releases/releases/latest). Nothing on the
phone changes: to it, this is one more computer (`os: windows`).

## What works

| | |
|---|---|
| Pairing | The phone finds the PC on the network (or scan the QR code from **Pair a Phone…** in the tray). The code prompt is a Windows notification with **Pair** / **Decline**. |
| Files | Phone → PC with **Accept / Always Accept / Decline** in a notification, saved to `Downloads\Linksy`. PC → phone from the tray (**Send Files…**) or `linksy send`. |
| Clipboard | Text, pictures and files both ways. The phone can follow the PC's clipboard as it changes. |
| Phone notifications | Shown as Windows notifications with the phone's own button, **Copy code** for one-time codes, and **Mute app**. Swiping one away dismisses it on the phone. |
| Links, ring, battery, calls | "Open on PC", ring the phone from the tray, low-battery and incoming-call notices. |
| Lock when away | Locks Windows (like Win+L) when your phone leaves. |
| Texting, inbox, history | Through the `linksy` command: `linksy texts`, `linksy text`, `linksy inbox`, `linksy history` (`linksy help` for the rest). |
| Tray | Status, a menu per paired device (Send Files…, Send Clipboard, Ring), Pair a Phone…, Open Received Files, Start at Sign-in, Quit. |
| Away from home | Through Linksy's relay, as on Linux. |

## Not yet

These need parts that only exist on Linux so far: screen sharing and remote control, second display,
phone as webcam or microphone, the deck, terminals, driving the phone over USB, the computer's
music and volume on the phone, Wi-Fi Direct transfers, browsing the phone's files in File Explorer,
replying to phone notifications from the PC, and the app window (GTK/Qt). The Windows
service doesn't advertise them, so the phone doesn't offer them.

## Build

You need [Rust](https://rustup.rs) and Visual Studio Build Tools with the C++ workload.

```powershell
cargo build --release -p linksyd -p linksy
```

That gives `target\release\linksyd.exe` (the service and tray; no console window) and
`target\release\linksy.exe` (the command line). Put them in one folder, run `linksyd.exe`, and
turn on **Start at Sign-in** in the tray menu.

The first time it runs, Windows Firewall asks whether Linksy may use the network: allow it on
**private** networks, or the phone can't reach it (UDP port 47200, and mDNS for discovery).

## Where things are

| | |
|---|---|
| Identity, paired devices, settings | `%APPDATA%\Linksy` |
| Received files | `Downloads\Linksy` (change with `linksy settings download-dir …`) |
| Log, crash reports | `%LOCALAPPDATA%\Linksy` |
| The service's local connection | named pipe `\\.\pipe\linksyd-<user>`, only openable by you |

## How the port works

The protocol core (`linksy-core`) was already portable: it also runs inside the Android app. The
Windows work is in the service:

- `linksy-ipc/src/local.rs`: Unix sockets on Linux, per-user named pipes (owner-only ACL, remote
  clients refused) on Windows, behind one `Listener`/`Stream` API.
- `linksyd/src/clipboard/win.rs`: the Win32 clipboard (text, pictures, file lists), watched by its
  sequence number.
- `linksyd/src/notify/win.rs` and `mirror.rs`: toast notifications with buttons, under Linksy's own
  app id.
- `linksyd/src/tray_win.rs`: the tray icon and menu on a thread with its own message loop.
- `linksyd/src/system.rs`: Windows folders, computer name, battery, `ShellExecute`.
- `linksy-core/src/node.rs`: the QUIC socket is made explicitly dual-stack, since Windows makes `[::]`
  IPv6-only by default.
