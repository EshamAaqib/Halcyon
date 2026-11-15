//! Halcyon: open-source control software for RGB keyboards.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod error;
mod hid;
mod json;
mod media;
mod originals;
mod paths;
mod protocol;
mod registry;
mod server;
mod tools;
mod window;

use std::path::Path;
use std::process::ExitCode;

const HELP: &str = "\
Halcyon: open-source control software for RGB keyboards

Usage:
  halcyon                       open the app
  halcyon --browser [--port N]  serve the app to your web browser instead of a window
  halcyon --demo                simulate every defined keyboard; nothing is sent to hardware
  halcyon --headless [--port N] only run the local server and print its address (for tests and CI)
  halcyon import-layout <KeyboardLayout.xml>
                                print a definition \"layout\" block from an HFD vendor layout file
  halcyon import-originals <keyboard id> <vendor app folder>
                                copy the product photo and factory animations from your own vendor software
  halcyon --version | --help

Settings live in ~/.halcyon (set HALCYON_HOME to move them).";

fn main() -> ExitCode {
    attach_console();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    match args.first().map(String::as_str) {
        Some("--help" | "-h") => {
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Some("--version" | "-V") => {
            println!("Halcyon {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some("import-layout") => {
            let Some(file) = args.get(1) else {
                return usage();
            };
            return match tools::import_layout(Path::new(file)) {
                Ok(layout) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&layout).unwrap_or_default()
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e),
            };
        }
        Some("import-originals") => {
            let (Some(id), Some(dir)) = (args.get(1), args.get(2)) else {
                return usage();
            };
            return match tools::import_originals(id, Path::new(dir)) {
                Ok((copied, missing)) => {
                    copied.iter().for_each(|c| println!("  copied      {c}"));
                    missing.iter().for_each(|m| println!("  not found   {m}"));
                    println!("Done. Files are in {}", paths::vendor(id).display());
                    if missing.is_empty() {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::from(1)
                    }
                }
                Err(e) => fail(&e),
            };
        }
        Some(a) if !a.starts_with("--") => return usage(),
        _ => {}
    }

    let manager = registry::Manager::new(flag("--demo"));
    let app = server::App::new(manager);
    if flag("--browser") || flag("--headless") {
        let default = if flag("--headless") { 0 } else { 8765 };
        // only the port can be chosen: the server always listens on 127.0.0.1
        let port = match args.iter().position(|a| a == "--port") {
            None => default,
            Some(i) => match args.get(i + 1).and_then(|p| p.parse::<u16>().ok()) {
                Some(p) => p,
                None => return usage(),
            },
        };
        let port = match app.serve(port) {
            Ok(p) => p,
            Err(e) => return fail(&e),
        };
        let url = format!("http://127.0.0.1:{port}/");
        println!("Halcyon running at {url}  (Ctrl+C to quit)");
        if flag("--browser") {
            let _ = open::that_detached(&url);
        }
        loop {
            std::thread::park();
        }
    }
    let port = match app.serve(0) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    window::run(&format!("http://127.0.0.1:{port}/"), app)
}

fn usage() -> ExitCode {
    eprintln!("{HELP}");
    ExitCode::from(2)
}

fn fail(e: &error::Error) -> ExitCode {
    eprintln!("halcyon: {e}");
    ExitCode::from(1)
}

/// The release build has no console window of its own; when started from a terminal, print there.
#[cfg(windows)]
fn attach_console() {
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // SAFETY: a plain Win32 call with no pointers; failing (no parent console) is harmless.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_console() {}
