# Security

Halcyon talks to hardware, runs a local web server and reads files other people write (keyboard definitions,
images), so it is built to treat all of that as untrusted. This page describes what it protects against and
how to report a problem.

## Reporting a vulnerability

Please report privately through GitHub's **Security → Report a vulnerability** on this repository, not in a
public issue. Include what you did, what happened, and the Halcyon version (`halcyon --version`).

## What Halcyon protects against

**Websites attacking the local server.** The window shows a page served by Halcyon on `127.0.0.1` only (never
on the network). Every API request must:

- carry a custom `X-Halcyon` header, which other websites cannot add without the server's permission (the
  server answers no CORS preflight), so cross-site requests fail even if another check were bypassed;
- name Halcyon's own address in `Host`, which defeats DNS rebinding;
- come from Halcyon's own page if it has an `Origin` or `Sec-Fetch-Site` header.

Every response sets `X-Frame-Options: DENY` and `frame-ancestors 'none'` (no clickjacking through hidden frames),
`nosniff`, `no-referrer` and cross-origin isolation headers. The page runs under a strict Content Security
Policy: only its own scripts, no inline script, no external connections.

**The window being steered elsewhere.** The native window can only navigate to Halcyon's own page; new windows
and downloads are refused.

**Malicious files.** Images and GIFs are size-checked before decoding (dimensions, total pixels and memory are
capped), so a tiny crafted file cannot exhaust memory. Requests, conversions, request bodies and cached uploads
are all bounded.

**Malicious keyboard definitions.** Definitions are data, never code. They are validated when loaded: file names
in `originals` must be plain names and paths must stay inside the vendor folder, ids are restricted to safe
characters, and display sizes are limited. At worst, a hostile definition can make Halcyon send its normal
keyboard commands to the USB device the definition names. Only install definitions from sources you trust.

**Mistakes by the app itself.** Nothing is sent to a keyboard until the user has accepted the
[disclaimer](DISCLAIMER.md). Halcyon contains no firmware flashing, and sends only the settings commands the
vendor software sends; a test requires those bytes to stay identical to the version confirmed on hardware.
Saved settings are written atomically, and a damaged settings file is kept aside rather than overwritten.

## Supply chain

- Dependencies are locked (`Cargo.lock`, `--locked` in CI) and scanned against the RustSec advisory database on
  every change and weekly (`cargo audit --deny warnings`). Reviewed exceptions, with reasons, are in
  `.cargo/audit.toml`.
- GitHub Actions are pinned to exact commits; workflows are read-only except the step that publishes a release.
- Dependabot proposes dependency and Actions updates.
- Every release includes `SHA256SUMS.txt`. Check a download with `sha256sum -c SHA256SUMS.txt` (macOS:
  `shasum -a 256 -c`, Windows: `Get-FileHash`).
- Release builds are not yet code-signed, so Windows SmartScreen and macOS Gatekeeper will warn on first run.

## Out of scope

Programs already running on your computer under your account can talk to any USB device you can, with or
without Halcyon. Halcyon does not try to defend against them.
