//! Linksy's menu bar app for macOS lives in `Sources/*.swift` beside this file; `build.rs`
//! compiles it with `swiftc` into `target/<profile>/Linksy.app`. There is no Rust in the
//! app: this crate exists so `cargo build -p linksy-mac` builds it like the rest of the
//! workspace. See `README.md` in this folder.
