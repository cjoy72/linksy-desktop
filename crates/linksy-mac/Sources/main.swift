// Linksy's menu bar app for macOS: a front end to the running `linksyd`, like the Linux
// tray (crates/linksyd/src/tray.rs) but as its own small app, since a launchd agent has no
// menu bar of its own.
//
//   linksy-mac                  run (normally from Linksy.app, at login)
//   linksy-mac --login-item     start Linksy.app at login from now on (macOS 13+), and exit
//   linksy-mac --no-login-item  stop starting it at login, and exit

import AppKit
import ServiceManagement

let arguments = Set(CommandLine.arguments.dropFirst())
if arguments.contains("--login-item") || arguments.contains("--no-login-item") {
    let register = arguments.contains("--login-item")
    do {
        if register {
            try SMAppService.mainApp.register()
            print("Linksy will start at login (System Settings › General › Login Items).")
        } else {
            try SMAppService.mainApp.unregister()
            print("Linksy no longer starts at login.")
        }
        exit(0)
    } catch {
        FileHandle.standardError.write(Data("Couldn't \(register ? "add" : "remove") the login item: \(error.localizedDescription)\n".utf8))
        exit(1)
    }
}

// Opened twice (at login, then by hand): the first one is enough
if let bundle = Bundle.main.bundleIdentifier, NSRunningApplication.runningApplications(withBundleIdentifier: bundle).count > 1 {
    exit(0)
}

signal(SIGPIPE, SIG_IGN)
let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let delegate = AppDelegate()
app.delegate = delegate
app.run()
