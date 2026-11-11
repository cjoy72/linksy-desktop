# linksy-mac

Linksy's menu bar app for macOS: a front end to the running `linksyd` over its local socket,
like the Linux tray (`crates/linksyd/src/tray.rs`) but as its own small app, since a launchd
agent has no menu bar of its own.

It's Swift against AppKit (`Sources/*.swift`), and `build.rs` compiles it with the system's
`swiftc` into `target/<profile>/Linksy.app`: Xcode's command line tools are enough, no Xcode
project and no Homebrew. Swift was picked over Rust with `objc2` because a status item, a
menu that is rebuilt on every open, an open panel and a QR window are a few hundred lines of
AppKit, with nothing to pull in. The crate has no Rust code of its own; it exists so
`cargo build --release -p linksy-mac` builds the app like the rest of the workspace. On other
systems the build script does nothing.

```sh
cargo build --release -p linksy-mac   # target/release/Linksy.app
open target/release/Linksy.app        # run it against the installed service
scripts/install-macos.sh              # install it with the service, as a login item
```

- `Client.swift`: the socket protocol (newline-delimited JSON; `crates/linksy-ipc`), with
  reconnecting every 3 s while the service is down.
- `Model.swift`: devices and pending requests, from the service's JSON.
- `AppDelegate.swift`: the status item and menu, Send Files / Send Clipboard / Ring,
  Accept / Decline, Start the Service, Quit.
- `Invite.swift`: the Pair a Device window with the QR code from `create_invite`.
- `main.swift`: start-up, and `--login-item` / `--no-login-item` (SMAppService).

`LINKSY_SOCKET` points it at another service, as for the CLI.
