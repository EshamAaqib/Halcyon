# Building Halcyon

Halcyon is a single Rust executable: the interface (`ui/`) and the keyboard definitions (`devices/`) are
compiled into it.

## Requirements

- [Rust](https://rustup.rs) 1.88 or newer
- **Windows**: the Microsoft C++ Build Tools (the Rust installer offers them). The WebView2 runtime it uses at
  run time is part of Windows 10 and 11.
- **macOS**: the Xcode command-line tools (`xcode-select --install`).
- **Linux** (Debian/Ubuntu names): `sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libudev-dev`.
  Fedora: `webkit2gtk4.1-devel gtk3-devel systemd-devel`. Arch: `webkit2gtk-4.1 gtk3`.

## Build

```
cargo build --release
```

| System | Result |
|---|---|
| Windows | `target/release/halcyon.exe`, one self-contained file with the icon embedded |
| macOS, Linux | `target/release/halcyon` |

To check a build without a keyboard: `halcyon --demo`.

## Packages

[cargo-bundle](https://github.com/burtonageo/cargo-bundle) turns the build into a macOS app or a Debian package,
using the `[package.metadata.bundle]` section of `Cargo.toml`:

```
cargo install cargo-bundle
cargo bundle --release          # Halcyon.app on macOS, a .deb on Linux
```

The macOS app is unsigned, so open it the first time with right-click → Open.

## Linux: keyboard access

Linux only lets root talk to HID devices by default. Install the udev rule once, then replug the keyboard:

```
sudo cp packaging/linux/70-halcyon.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger
```

When you add a keyboard, add its USB ids to that file too.

## Releases

Pushing a tag like `v1.0.0` runs `.github/workflows/release.yml`. It tests and builds on GitHub's machines
and attaches four files to the release: the Windows executable, macOS apps for Apple Silicon and Intel, and a
Linux archive with the udev rule. You can also start it by hand from the Actions tab to get test builds
without tagging.
