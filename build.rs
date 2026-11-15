//! Embed the app icon and version details in the Windows executable.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    // `cfg(windows)` here is the machine building; the resource only belongs in a Windows target.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Halcyon");
        res.set("FileDescription", "Halcyon keyboard control");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the icon ({e}); the app still works");
        }
    }
}
