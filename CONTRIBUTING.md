# Contributing to Halcyon

Three kinds of help matter most:

1. **Hardware reports.** Try a feature on your keyboard and open an issue saying what happened.
2. **New keyboards.** Usually one JSON file, no code.
3. **Fixes and features.** Rust, protocol decoding, UI.

## Working on the code

Install [Rust](https://rustup.rs) and the few system packages listed in [docs/BUILDING.md](docs/BUILDING.md).

```
cargo test                          # protocol bytes, definitions, media and tools; no keyboard needed
cargo run -- --demo                 # the full app with simulated keyboards; nothing is sent to hardware
cargo run -- --demo --browser       # the same in your browser, handy for its developer tools
cargo clippy && cargo fmt           # before a pull request
```

Use `--demo` for all UI work. With a real keyboard connected and no `--demo`, every click is sent to it.

The UI is plain HTML, CSS and JavaScript in `ui/` with no build step; it is compiled into the executable, so
restart `cargo run` after editing it. Its design rule: true black, no decoration, and the only colour on screen
is the keyboard's own light.

## Adding a keyboard

> [!WARNING]
> **Adding and testing keyboards is entirely at your own risk.** A definition decides which USB device Halcyon
> talks to and which commands it sends. A wrong USB interface, effect number or table size could misconfigure a
> keyboard, wipe its stored keymaps, macros or animations, or brick it, and testing may void its warranty.
> **You are solely responsible for any damage to your keyboard or other devices; the Halcyon authors and
> contributors accept no responsibility or liability.** Read [DISCLAIMER.md](DISCLAIMER.md) first, start with
> read-only features (connection status, then lighting), and never test on a keyboard you can't afford to lose.

Halcyon separates **keyboards** from **protocols**:

- A **device definition** (`devices/<id>.json`) describes one keyboard.
- A **protocol family** (`src/protocol/<name>.rs`) is the wire format a group of keyboards share, usually because
  they use the same chip and vendor firmware. You only write one if your keyboard speaks something new.

### 1. Find the USB IDs

Windows: Device Manager → your keyboard → Properties → Details → Hardware Ids shows `VID_xxxx&PID_xxxx`.
macOS: System Information → USB. The control interface is almost always one with a vendor usage page
(`0xFF00` or higher); the vendor software's own folder or a USB viewer can tell you which.

### 2. Write the definition

Copy `devices/leobog-amg65.json`. While testing, keep your file in your own keyboards folder
(**Keyboards → Open keyboards folder**, normally `~/.halcyon/devices/`) and press **Reload**: no rebuild needed.

| Field | Meaning |
|---|---|
| `id` | Unique, lowercase `vendor-model`; must equal the file name |
| `vendor`, `name` | Shown in the app |
| `protocol` | A protocol family, e.g. `hfd` (see `src/protocol/mod.rs`) |
| `connections[]` | One per way the board connects. `type` is a transport of the protocol (`wired`, `dongle`); `vid`, `pid`, `usage_page`, `usage` pick the HID interface; `product` optionally matches the product string; `bulk` names a second interface used for display uploads |
| `features` | What the board can do and over which connections: `lighting`, `perkey`, `keymap`, `macros`, `settings`, `clock`, `matrix` (`width`, `height`, `max_frames`), `lcd` (`width`, `height`, `max_frames`). Leave out what it lacks and the app hides it |
| `lighting.effects[]` | `id` (the number the firmware uses), `name`, `group`, `description`, `controls` (any of `color`, `rainbow`, `brightness`, `speed`, `direction`) and `preview` (how the app animates it: `static`, `animated`, `spectrum`, `rainbow`, `twinkle`, `outward`, `shuttle`, `directional`, `reactive` (ripples from the pressed key), `reactive-key` (only the pressed key), `reactive-off`, `perkey`, `off`) |
| `lighting.groups[]` | Headings for the effect list |
| `layout` | `width`/`height` in key units and `keys[]` with `label`, `x`, `y`, `w`, `code` (HID usage), `key_index` (position in the keymap), `led` (position in the per-key colour table) |
| `originals` | Optional. Files Halcyon may read from the user's own vendor software; see [docs/ORIGINALS.md](docs/ORIGINALS.md) |

Boards that ship the HFD vendor driver (its folder contains `DeviceDriver.exe` and `KeyboardLayout.xml`)
very likely speak `hfd` already. Generate the layout with:

```
halcyon import-layout "path/to/KeyboardLayout.xml" > layout.json
```

### 3. Test and share

Copy the file into `devices/`, run `cargo test` (it validates every definition), try each feature, and open a
pull request that says which connections and features you confirmed on real hardware.

## Writing a protocol family

Add `src/protocol/<name>.rs` with types that implement the `Transport` trait (`src/protocol/mod.rs`); only
`status` and `set_lighting` are required, everything else is optional. List the family in `transports()`,
`open()` and, if it can reject input, the `validate_*` functions in the same file. `hfd.rs` is the reference,
including how to test transports byte for byte with a recording `Port`. Document the wire format in `docs/` the
way `docs/PROTOCOL.md` does.

Rules for protocol work:

- Reverse engineer for compatibility only: describe formats in your own words and code; never copy vendor code.
- Never commit vendor files (firmware, images, databases, installers).
- No firmware flashing. Halcyon sends only the settings commands the vendor software sends.
- Mark anything not yet confirmed on hardware as such.
- `tests/golden/hfd_python.json` records every byte the original prototype sent; the `hfd` transports must keep
  matching it unless a change is confirmed on hardware.

## Pull requests

Keep them focused, match the surrounding code style, and run `cargo test`, `cargo clippy` and `cargo fmt`
first. By contributing you agree that your work is released under the project's MIT licence.
