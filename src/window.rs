//! The native window: the system web view (WebView2 on Windows, WebKit on macOS, WebKitGTK on Linux)
//! showing the local UI.

use std::process::ExitCode;
use std::sync::Arc;

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::window::{Icon, WindowBuilder};
use wry::{WebContext, WebViewBuilder};

use crate::paths;
use crate::server::App;

static ICON: &[u8] = include_bytes!("../assets/logo.png");

fn icon() -> Option<Icon> {
    let img = image::load_from_memory(ICON)
        .ok()?
        .resize(64, 64, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    let (w, h) = img.dimensions();
    Icon::from_rgba(img.into_raw(), w, h).ok()
}

pub fn run(url: &str, app: Arc<App>) -> ExitCode {
    let event_loop = EventLoop::new();
    let window = match WindowBuilder::new()
        .with_title("Halcyon")
        .with_inner_size(LogicalSize::new(1280.0, 860.0))
        .with_min_inner_size(LogicalSize::new(900.0, 640.0))
        .with_window_icon(icon())
        .with_background_color((0, 0, 0, 255))
        .build(&event_loop)
    {
        Ok(w) => w,
        Err(e) => {
            eprintln!("halcyon: could not open a window ({e})");
            return ExitCode::from(1);
        }
    };
    // Keep the web view's own data (cache, local storage) with Halcyon's files, not next to the executable.
    let mut context = WebContext::new(Some(paths::home().join("webview")));
    // The window only ever shows Halcyon's own page: no other sites, pop-ups or downloads.
    let origin = url.to_string(); // "http://127.0.0.1:<port>/"
    let builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(url)
        .with_background_color((0, 0, 0, 255))
        .with_devtools(cfg!(debug_assertions))
        .with_navigation_handler(move |to| to.starts_with(&origin))
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .with_download_started_handler(|_, _| false);
    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(&window);
    // On Linux the web view (WebKitGTK) goes inside the window's GTK container, which also makes it work on Wayland.
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(
            window
                .default_vbox()
                .expect("tao windows on Linux have a GTK box"),
        )
    };
    let webview = match webview {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "halcyon: could not start the web view ({e}). On Windows this needs the Microsoft Edge WebView2 runtime."
            );
            return ExitCode::from(1);
        }
    };
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        let _keep_alive = (&webview, &app);
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *flow = ControlFlow::Exit;
        }
    })
}
