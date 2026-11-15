//! Local HTTP server on 127.0.0.1: serves the UI and a small JSON API over the registry.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use include_dir::{Dir, include_dir};
use serde_json::{Value, json};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::error::{Error, Result, bad};
use crate::json::int;
use crate::media;
use crate::originals;
use crate::paths;
use crate::protocol::{Frame, LcdFrame, MatrixOptions};
use crate::registry::{Manager, Upload};

static UI: Dir = include_dir!("$CARGO_MANIFEST_DIR/ui");
/// Image and GIF uploads.
const MAX_UPLOAD: usize = 40 * 1024 * 1024;
/// JSON requests; the largest is a full matrix animation, well under 1 MB.
const MAX_JSON: usize = 4 * 1024 * 1024;
/// Requests handled at once; more get 503 instead of a thread each.
const MAX_REQUESTS: usize = 64;
/// Image decodes (uploads and product photos) at once: each can take a few hundred MB while it runs.
const MAX_DECODES: usize = 2;
/// Every /api/ request must carry this header. A web page can only add a custom header to a cross-site
/// request after a CORS preflight, which this server never approves.
const API_HEADER: &str = "X-Halcyon";
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
/// Raise when DISCLAIMER.md changes in substance, so everyone is asked to accept it again.
const DISCLAIMER_VERSION: i64 = 1;

#[derive(Clone)]
enum Media {
    Matrix(Arc<Vec<Frame>>),
    Lcd {
        frames: Arc<Vec<LcdFrame>>,
        parts: Vec<Arc<Vec<LcdFrame>>>, // shared with earlier playlist versions, never copied
    },
}

impl Media {
    fn target(&self) -> &'static str {
        match self {
            Media::Matrix(_) => "matrix",
            Media::Lcd { .. } => "lcd",
        }
    }
}

#[derive(Clone)]
struct Job {
    state: &'static str,
    progress: f64,
    error: Option<String>,
    target: &'static str,
}

pub struct App {
    pub manager: Manager,
    media: Mutex<Vec<(String, Media)>>, // oldest first; prepared files waiting to be sent
    jobs: Mutex<HashMap<String, Job>>,
    port: Mutex<u16>,
    requests: AtomicUsize,
    decodes: AtomicUsize,
}

/// Holds one of `max` places counted by `counter`; the place is given back when dropped.
struct Permit<'a>(&'a AtomicUsize);

impl<'a> Permit<'a> {
    fn take(counter: &'a AtomicUsize, max: usize) -> Option<Permit<'a>> {
        let permit = Permit(counter);
        (counter.fetch_add(1, Ordering::AcqRel) < max).then_some(permit) // dropping it undoes the add
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

enum Reply {
    Json(u16, Value),
    Bytes(u16, &'static str, Vec<u8>),
}

fn ok(v: Value) -> Result<Reply> {
    Ok(Reply::Json(200, v))
}

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "woff2" => "font/woff2",
        "json" => "application/json",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn token() -> String {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).expect("the system random generator is available");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn b64_frames(frames: &[Frame]) -> String {
    let flat: Vec<u8> = frames
        .iter()
        .flat_map(|f| f.iter().flatten().copied())
        .collect();
    B64.encode(flat)
}

impl App {
    pub fn new(manager: Manager) -> Arc<App> {
        Arc::new(App {
            manager,
            media: Mutex::new(Vec::new()),
            jobs: Mutex::new(HashMap::new()),
            port: Mutex::new(0),
            requests: AtomicUsize::new(0),
            decodes: AtomicUsize::new(0),
        })
    }

    /// Bind to 127.0.0.1 (port 0 = any free port) and serve on background threads. Returns the port.
    pub fn serve(self: &Arc<App>, port: u16) -> Result<u16> {
        let server = Server::http(("127.0.0.1", port))
            .map_err(|e| Error::Internal(format!("could not start the local server ({e})")))?;
        let port = server
            .server_addr()
            .to_ip()
            .map(|a| a.port())
            .unwrap_or(port);
        *lock(&self.port) = port;
        let app = self.clone();
        thread::spawn(move || {
            for request in server.incoming_requests() {
                let Some(permit) = Permit::take(&app.requests, MAX_REQUESTS) else {
                    let busy =
                        json!({"ok": false, "error": "too many requests at once; try again"});
                    let _ = request.respond(response(Reply::Json(503, busy)));
                    continue;
                };
                std::mem::forget(permit); // handed over to the thread below, which gives it back
                let worker = app.clone();
                let spawned = thread::Builder::new().spawn(move || {
                    let _permit = Permit(&worker.requests);
                    worker.clone().handle(request);
                });
                if spawned.is_err() {
                    drop(Permit(&app.requests)); // no thread: the request is dropped, its place freed
                }
            }
        });
        Ok(port)
    }

    fn handle(self: Arc<App>, mut req: Request) {
        let url = req.url().to_string();
        let (path, query) = url.split_once('?').unwrap_or((&url, ""));
        let path = path.to_string();
        let query: HashMap<String, String> = query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let reply = if !path.starts_with("/api/") {
            self.static_file(&path)
        } else if !self.local(&req) {
            Reply::Json(
                403,
                json!({"ok": false, "error": "cross-site requests are not allowed"}),
            )
        } else {
            let method = req.method().clone();
            // a bug must answer 500, not drop the connection
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.api(&method, &path, &query, &mut req)
            }))
            .unwrap_or_else(|_| Err(Error::Internal("internal error".into())))
            .unwrap_or_else(|e| Reply::Json(e.status(), json!({"ok": false, "error": e.message()})))
        };
        let _ = req.respond(response(reply)); // the page may have gone away before the answer was sent
    }

    /// Only Halcyon's own page may use the API. Without this any website open in a browser could post to
    /// localhost and remap keys or upload macros (and DNS rebinding could read the replies).
    fn local(&self, req: &Request) -> bool {
        let port = *lock(&self.port);
        allowed(port, |name| {
            req.headers()
                .iter()
                .find(|h| h.field.equiv(name))
                .map(|h| h.value.as_str().to_string())
        })
    }

    fn static_file(&self, path: &str) -> Reply {
        let rel = if path == "/" {
            "index.html"
        } else {
            path.trim_start_matches('/')
        };
        match UI.get_file(rel) {
            Some(f) if !rel.contains("..") => Reply::Bytes(200, mime(rel), f.contents().to_vec()),
            _ => Reply::Bytes(404, "text/plain; charset=utf-8", b"Not found".to_vec()),
        }
    }

    /// The request body, at most `max` bytes. Chunked bodies (no Content-Length) are read up to the limit too.
    fn body(req: &mut Request, max: usize, allow_empty: bool) -> Result<Vec<u8>> {
        let declared = req
            .headers()
            .iter()
            .find(|h| h.field.equiv("Content-Length"))
            .map(|h| h.value.as_str().trim().to_string());
        if declared
            .as_deref()
            .is_some_and(|v| v.parse::<usize>().is_err())
        {
            return bad("invalid Content-Length");
        }
        let too_large = || bad(format!("the request is larger than {} MB", max >> 20));
        let size = req.body_length().unwrap_or(0);
        if size > max {
            return too_large();
        }
        let mut data = Vec::with_capacity(size);
        req.as_reader()
            .take(max as u64 + 1)
            .read_to_end(&mut data)
            .map_err(|_| Error::Bad("the request body could not be read".into()))?;
        if data.len() > max {
            return too_large();
        }
        if !allow_empty && data.is_empty() {
            return bad("the file is empty");
        }
        Ok(data)
    }

    fn json_body(req: &mut Request) -> Result<Value> {
        let data = Self::body(req, MAX_JSON, true)?;
        if data.is_empty() {
            return Ok(json!({}));
        }
        let v: Value =
            serde_json::from_slice(&data).map_err(|e| Error::Bad(format!("invalid JSON ({e})")))?;
        if !v.is_object() {
            return bad("expected a JSON object");
        }
        Ok(v)
    }

    fn api(
        self: &Arc<Self>,
        method: &Method,
        path: &str,
        query: &HashMap<String, String>,
        req: &mut Request,
    ) -> Result<Reply> {
        let parts: Vec<&str> = path.trim_start_matches("/api/").split('/').collect();
        let m = &self.manager;
        match (method, parts.as_slice()) {
            (Method::Get, ["devices"]) => {
                let connected = m.connected();
                let devices: Vec<Value> = m
                    .ids()
                    .iter()
                    .filter_map(|id| {
                        let d = m.def(id).ok()?;
                        let kinds: Vec<Value> = d["connections"].as_array()?.iter().map(|c| c["type"].clone()).collect();
                        Some(json!({"id": id, "vendor": d["vendor"], "name": d["name"], "protocol": d["protocol"], "source": d["source"],
                                    "connections": kinds, "connected": connected.get(id), "has_product": self.product_file(id).is_some()}))
                    })
                    .collect();
                ok(
                    json!({"devices": devices, "problems": m.problems(), "user_folder": paths::user_devices().display().to_string(), "demo": m.demo()}),
                )
            }
            // The user's acknowledgement of DISCLAIMER.md, kept with their settings.
            (Method::Get, ["disclaimer"]) => {
                let accepted = m.state.get("_app", "disclaimer").and_then(|v| v.as_i64())
                    == Some(DISCLAIMER_VERSION);
                ok(json!({"accepted": accepted, "version": DISCLAIMER_VERSION}))
            }
            (Method::Post, ["disclaimer"]) => {
                m.state
                    .put("_app", "disclaimer", json!(DISCLAIMER_VERSION))?;
                ok(json!({"ok": true}))
            }
            (Method::Post, ["reload"]) => {
                m.reload();
                ok(json!({"ok": true, "problems": m.problems()}))
            }
            (Method::Post, ["open-user-folder"]) => {
                let dir = paths::user_devices();
                std::fs::create_dir_all(&dir)?;
                if std::env::var_os("HALCYON_NO_OPEN").is_none() {
                    // tests set HALCYON_NO_OPEN so no Explorer/Finder window pops up
                    open::that_detached(&dir)
                        .map_err(|e| Error::Internal(format!("could not open the folder ({e})")))?;
                }
                ok(json!({"ok": true}))
            }
            (Method::Get, ["devices", id]) => ok(
                json!({"definition": m.definition(id)?, "lighting": m.lighting(id), "config": m.config(id)}),
            ),
            (Method::Get, ["devices", id, "status"]) => ok(m.status(id)?),
            (Method::Get, ["devices", id, "display", "job"]) => {
                m.def(id)?;
                let job = lock(&self.jobs).get(*id).cloned();
                ok(job.map_or_else(|| json!({"state": "idle"}), |j| json!({"state": j.state, "progress": j.progress, "error": j.error, "target": j.target})))
            }
            (Method::Get, ["devices", id, "product"]) => {
                m.def(id)?;
                let f = self
                    .product_file(id)
                    .ok_or_else(|| Error::NotFound("no photo of this keyboard".into()))?;
                Ok(Reply::Bytes(200, "image/png", std::fs::read(f)?))
            }
            (Method::Get, ["devices", id, "originals", which]) => self.original_preview(id, which),
            (Method::Get, ["devices", id, "library"]) => self.library(id, None),
            (Method::Get, ["devices", id, "library", layer]) => {
                let layer = layer
                    .parse::<i64>()
                    .map_err(|_| Error::NotFound("unknown animation".into()))?;
                self.library(id, Some(layer))
            }
            (Method::Post, ["devices", id, "media"]) => {
                m.def(id)?;
                let _permit = self.decode_permit()?;
                let data = Self::body(req, MAX_UPLOAD, false)?;
                self.prepare_media(id, query, &data)
            }
            (Method::Post, ["devices", id, "product"]) => {
                m.def(id)?;
                if m.demo() {
                    // demo sessions leave the user's own files alone, like every other setting
                    return bad("Photos aren't saved in demo mode");
                }
                let _permit = self.decode_permit()?;
                let data = Self::body(req, MAX_UPLOAD, false)?;
                save_product(id, &data)?;
                ok(json!({"ok": true}))
            }
            (Method::Post, ["devices", id, what @ ..]) => {
                m.def(id)?; // an unknown keyboard is reported before anything about the request body
                let data = Self::json_body(req)?;
                let id = *id;
                let done = match what {
                    ["effects", effect, "name"] => {
                        let effect = effect
                            .parse::<i64>()
                            .map_err(|_| Error::NotFound("unknown effect".into()))?;
                        m.name_effect(id, effect, data["name"].as_str().unwrap_or(""))?;
                        return ok(json!({"ok": true}));
                    }
                    ["display"] => return self.start_display(id, &data),
                    ["frames"] => return self.drawn_frames(id, &data),
                    ["lighting"] => m.set_lighting(id, &data)?,
                    ["keymap"] => m.save_keymap(
                        id,
                        data.get("keymap")
                            .ok_or_else(|| Error::Bad("missing keymap".into()))?,
                    )?,
                    ["macros"] => m.save_macros(
                        id,
                        data.get("macros")
                            .ok_or_else(|| Error::Bad("missing macros".into()))?,
                    )?,
                    ["perkey"] => m.save_perkey(
                        id,
                        data.get("colors")
                            .ok_or_else(|| Error::Bad("missing colors".into()))?,
                        data.get("brightness").unwrap_or(&json!(5)),
                    )?,
                    ["settings"] => m.save_settings(
                        id,
                        data.get("settings")
                            .ok_or_else(|| Error::Bad("missing settings".into()))?,
                    )?,
                    ["clock"] => m.sync_clock(id, data.get("slot").unwrap_or(&json!(1)))?,
                    _ => return Err(Error::NotFound("not found".into())),
                };
                ok(
                    json!({"ok": done, "error": if done { Value::Null } else { json!("The keyboard didn't answer. It may be asleep: press a key and try again.") }}),
                )
            }
            _ => Err(Error::NotFound("not found".into())),
        }
    }

    fn decode_permit(&self) -> Result<Permit<'_>> {
        Permit::take(&self.decodes, MAX_DECODES).ok_or_else(|| {
            Error::Busy("another picture is still being converted; try again in a moment".into())
        })
    }

    /// A photo of the keyboard: the user's own (product-user.png) or one from their vendor software.
    fn product_file(&self, id: &str) -> Option<PathBuf> {
        let spec_file = self.manager.def(id).ok()?["originals"]["product"]["file"]
            .as_str()
            .map(str::to_string);
        ["product-user.png".to_string()]
            .into_iter()
            .chain(spec_file)
            .filter_map(|n| paths::vendor_file(id, &n)) // a definition's name must stay in the folder
            .find(|p| p.is_file())
    }

    fn feature_spec(&self, id: &str, target: &str) -> Result<Value> {
        let feat = self.manager.def(id)?["features"]
            .get(target)
            .cloned()
            .filter(|_| target == "lcd" || target == "matrix");
        feat.ok_or_else(|| Error::Bad(format!("no {target} on this keyboard")))
    }

    fn store(&self, item: Media) -> String {
        let tk = token();
        let mut cache = lock(&self.media);
        let same: Vec<usize> = cache
            .iter()
            .enumerate()
            .filter(|(_, (_, m))| m.target() == item.target())
            .map(|(i, _)| i)
            .collect();
        if same.len() > 3 {
            // keep the last few per display (a boot upload must not evict the matrix design)
            for i in same[..same.len() - 3].iter().rev() {
                cache.remove(*i);
            }
        }
        cache.push((tk.clone(), item));
        tk
    }

    fn cached(&self, tk: &str) -> Option<Media> {
        lock(&self.media)
            .iter()
            .find(|(t, _)| t == tk)
            .map(|(_, m)| m.clone())
    }

    fn matrix_reply(&self, frames: Vec<Frame>, delays: Vec<u32>) -> Result<Reply> {
        let pixels = b64_frames(&frames);
        let count = frames.len();
        let tk = self.store(Media::Matrix(Arc::new(frames)));
        let duration: u64 = delays.iter().map(|&d| d as u64).sum();
        ok(
            json!({"ok": true, "token": tk, "frames": count, "duration_ms": duration, "pixels": pixels, "delays": delays}),
        )
    }

    /// An uploaded image/GIF, converted for the display and kept until it is sent.
    fn prepare_media(
        &self,
        id: &str,
        query: &HashMap<String, String>,
        data: &[u8],
    ) -> Result<Reply> {
        let target = query.get("target").map(String::as_str).unwrap_or("lcd");
        let (w, h, max) = display_size(&self.feature_spec(id, target)?);
        let fit = query.get("fit").map(String::as_str).unwrap_or("cover");
        let frames = media::load_frames(data, w, h, fit, max)?;
        if target == "matrix" {
            let delays = frames.iter().map(|f| f.delay_ms).collect();
            return self.matrix_reply(
                frames.iter().map(|f| media::pixels(&f.image)).collect(),
                delays,
            );
        }
        // The screen has one slot, so several files become a playlist that shares its frame budget.
        let mut parts = match query.get("append").and_then(|t| self.cached(t)) {
            Some(Media::Lcd { parts, .. }) => parts,
            _ => Vec::new(),
        };
        parts.push(Arc::new(frames));
        if parts.len() > max {
            return bad("the screen can't hold any more files");
        }
        let per = (max / parts.len()).max(1);
        // Each part is kept at its share, so a long playlist holds at most `max` frames in all. Shares only
        // shrink as files are added, so nothing that is dropped here is needed later.
        let parts: Vec<Arc<Vec<LcdFrame>>> = parts
            .into_iter()
            .map(|p| {
                if p.len() > per {
                    Arc::new(media::thin((*p).clone(), per))
                } else {
                    p
                }
            })
            .collect();
        let combined: Vec<LcdFrame> = parts.iter().flat_map(|p| p.iter().cloned()).collect();
        let preview = media::preview_gif(&combined)?;
        let (count, duration, n_parts) = (
            combined.len(),
            combined.iter().map(|f| f.delay_ms as u64).sum::<u64>(),
            parts.len(),
        );
        let tk = self.store(Media::Lcd {
            frames: Arc::new(combined),
            parts,
        });
        ok(
            json!({"ok": true, "token": tk, "frames": count, "parts": n_parts, "duration_ms": duration, "preview": preview}),
        )
    }

    /// Frames drawn in the app (pixel editor, text banner, styles): base64 RGB, width x height x 3 bytes each.
    fn drawn_frames(&self, id: &str, data: &Value) -> Result<Reply> {
        let feat = self.manager.def(id)?["features"]
            .get("matrix")
            .cloned()
            .ok_or_else(|| Error::Bad("this keyboard has no LED matrix".into()))?;
        let (w, h, max) = display_size(&feat);
        let n = w as usize * h as usize;
        let raw = B64
            .decode(data["pixels"].as_str().unwrap_or(""))
            .map_err(|_| Error::Bad("frame data is not base64".into()))?;
        if n == 0 || raw.is_empty() || raw.len() % (n * 3) != 0 {
            return bad("frame data has the wrong size");
        }
        let frames: Vec<Frame> = raw
            .chunks(n * 3)
            .take(max)
            .map(|f| f.chunks(3).map(|c| [c[0], c[1], c[2]]).collect())
            .collect();
        let mut delays: Vec<u32> = data["delays"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|d| int(d, "delay").map(|v| v.clamp(0, 60_000) as u32))
            .collect::<Result<_>>()?;
        delays.resize(frames.len(), 100);
        self.matrix_reply(frames, delays)
    }

    fn original(&self, id: &str, which: &str) -> Result<Option<Upload>> {
        let d = self.manager.def(id)?;
        let spec = &d["originals"][which];
        if !spec.is_object() {
            return Ok(None);
        }
        match which {
            "boot" => Ok(originals::boot_animation(id, spec)?
                .filter(|b| !b.frames.is_empty())
                .map(|b| {
                    // no file, or no frames in it, would blank the display, so both count as missing
                    Upload::Matrix(
                        Arc::new(b.frames),
                        MatrixOptions {
                            speed: b.speed,
                            boot: true,
                            brightness: b.brightness,
                        },
                    )
                })),
            "screen" => {
                let (Some(data), Ok(feat)) = (
                    originals::screen_file(id, spec),
                    self.feature_spec(id, "lcd"),
                ) else {
                    return Ok(None);
                };
                // a damaged vendor file is a problem on this computer, not bad input from the page
                let (w, h, max) = display_size(&feat);
                let frames = media::load_frames(&data, w, h, "cover", max).map_err(|e| {
                    Error::Internal(format!("the original screen file can't be read ({e})"))
                })?;
                Ok(Some(Upload::Lcd(
                    Arc::new(frames),
                    spec["slot"].as_u64().unwrap_or(1).min(255) as u8,
                )))
            }
            _ => Ok(None),
        }
    }

    fn original_preview(&self, id: &str, which: &str) -> Result<Reply> {
        match self.original(id, which)? {
            // Previews show the colours at full strength; the keyboard gets them dimmed like the vendor app does.
            Some(Upload::Matrix(frames, _)) => ok(
                json!({"ok": true, "frames": frames.len(), "pixels": b64_frames(&frames), "delays": vec![60; frames.len()]}),
            ),
            Some(Upload::Lcd(frames, _)) => ok(
                json!({"ok": true, "frames": frames.len(), "preview": media::preview_gif(&frames)?}),
            ),
            None => ok(json!({"ok": false})),
        }
    }

    /// The vendor's built-in matrix animations, from the user's own vendor software.
    fn library(&self, id: &str, layer: Option<i64>) -> Result<Reply> {
        let d = self.manager.def(id)?;
        let (spec, feat) = (&d["originals"]["library"], &d["features"]["matrix"]);
        if !spec.is_object() || !feat.is_object() {
            return ok(json!({"ok": false, "items": []}));
        }
        let Some(layer) = layer else {
            let boot = spec["boot_layer"].as_i64().unwrap_or(1);
            let items: Vec<Value> = originals::library(id, spec)?
                .into_iter()
                .filter(|e| e["layer"].as_i64() != Some(boot))
                .collect();
            return ok(json!({"ok": true, "items": items}));
        };
        let (w, h, max) = display_size(feat);
        let Some(frames) = originals::library_frames(id, spec, layer, w as usize, h as usize)?
        else {
            return ok(json!({"ok": false}));
        };
        let frames = media::thin(frames, max); // thin evenly so the whole animation still plays
        ok(
            json!({"ok": true, "frames": frames.len(), "pixels": b64_frames(&frames), "delays": vec![60; frames.len()]}),
        )
    }

    fn start_display(self: &Arc<Self>, id: &str, data: &Value) -> Result<Reply> {
        self.manager.def(id)?;
        let byte = |key: &str, default: i64, hi: i64| -> Result<u8> {
            let v = int(data.get(key).unwrap_or(&json!(default)), key)?;
            if !(0..=hi).contains(&v) {
                return bad(format!("{key} must be between 0 and {hi}"));
            }
            Ok(v as u8)
        };
        let mut steps = Vec::new();
        if data.get("restore").is_some_and(crate::json::truthy) {
            let which = data["restore"].as_str().unwrap_or("");
            let up = self.original(id, which)?.ok_or_else(|| {
                Error::NotFound("the original isn't available on this computer".into())
            })?;
            steps.push(up);
        } else {
            // One or more prepared files: {"token": ...} plus optionally {"also": token} to fill both displays at once.
            for key in ["token", "also"] {
                let Some(tk) = data[key].as_str().filter(|s| !s.is_empty()) else {
                    continue;
                };
                match self
                    .cached(tk)
                    .ok_or_else(|| Error::Bad("choose a file first".into()))?
                {
                    Media::Matrix(frames) => {
                        let opts = MatrixOptions {
                            speed: byte("speed", 10, 255)?,
                            boot: data.get("boot").is_some_and(crate::json::truthy),
                            brightness: byte("brightness", 25, 100)?,
                        };
                        steps.push(Upload::Matrix(frames, opts));
                    }
                    Media::Lcd { frames, .. } => {
                        steps.push(Upload::Lcd(frames, byte("slot", 1, 255)?))
                    }
                }
            }
            if steps.is_empty() {
                return bad("choose a file first");
            }
        }
        let first = match steps[0] {
            Upload::Matrix(..) => "matrix",
            Upload::Lcd(..) => "lcd",
        };
        {
            let mut jobs = lock(&self.jobs); // two requests arriving together must not both start an upload
            if jobs.get(id).is_some_and(|j| j.state == "running") {
                return Err(Error::Busy("an upload is already running".into()));
            }
            jobs.insert(
                id.to_string(),
                Job {
                    state: "running",
                    progress: 0.0,
                    error: None,
                    target: first,
                },
            );
        }
        let (app, id) = (self.clone(), id.to_string());
        thread::spawn(move || app.run_job(&id, steps));
        ok(json!({"ok": true}))
    }

    fn run_job(&self, id: &str, steps: Vec<Upload>) {
        let n = steps.len();
        let set = |f: &dyn Fn(&mut Job)| {
            if let Some(j) = lock(&self.jobs).get_mut(id) {
                f(j);
            }
        };
        for (i, step) in steps.iter().enumerate() {
            let target = match step {
                Upload::Matrix(..) => "matrix",
                Upload::Lcd(..) => "lcd",
            };
            set(&|j| j.target = target);
            let mut progress = |p: f64| set(&|j| j.progress = (i as f64 + p) / n as f64);
            // whatever went wrong, the job must not stay "running": that would block every later upload
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.manager.upload_display(id, step, &mut progress)
            }))
            .unwrap_or_else(|_| Err(Error::Internal("the upload stopped unexpectedly".into())));
            if let Err(e) = result {
                let msg = e.message().to_string();
                set(&|j| {
                    j.state = "error";
                    j.error = Some(msg.clone());
                });
                return;
            }
        }
        set(&|j| {
            j.state = "done";
            j.progress = 1.0;
        });
    }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header")
}

/// (width, height, max_frames) of a display feature; `check_definition` keeps these small and non-zero.
fn display_size(feat: &Value) -> (u32, u32, usize) {
    let get = |k: &str, default: u64| feat[k].as_u64().unwrap_or(default).min(4096);
    (
        get("width", 0) as u32,
        get("height", 0) as u32,
        get("max_frames", 1) as usize,
    )
}

/// The API check on its own, for tests: `get` returns a request header by name.
fn allowed(port: u16, get: impl Fn(&'static str) -> Option<String>) -> bool {
    let host = get("Host").unwrap_or_default().trim().to_ascii_lowercase();
    let mut hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    if port == 80 {
        hosts.extend(["127.0.0.1".to_string(), "localhost".to_string()]);
    }
    // Browsers label requests made by another site; only a missing label (older engines) or our own is fine.
    let site_ok = get("Sec-Fetch-Site").is_none_or(|s| matches!(s.trim(), "same-origin" | "none"));
    hosts.contains(&host)
        && get("Origin").is_none_or(|o| o.trim().eq_ignore_ascii_case(&format!("http://{host}")))
        && get(API_HEADER).is_some_and(|v| v.trim() == "1")
        && site_ok
}

fn response(reply: Reply) -> Response<Cursor<Vec<u8>>> {
    let (code, ty, body) = match reply {
        Reply::Json(code, v) => (code, "application/json", v.to_string().into_bytes()),
        Reply::Bytes(code, ty, body) => (code, ty, body),
    };
    let mut r = Response::from_data(body)
        .with_status_code(code)
        .with_header(header("Content-Type", ty))
        .with_header(header("Cache-Control", "no-cache"))
        .with_header(header("X-Content-Type-Options", "nosniff"))
        .with_header(header("Referrer-Policy", "no-referrer"))
        .with_header(header("X-Frame-Options", "DENY"))
        .with_header(header("Cross-Origin-Opener-Policy", "same-origin"))
        .with_header(header("Cross-Origin-Resource-Policy", "same-origin"));
    if ty.starts_with("text/html") {
        r.add_header(header("Content-Security-Policy", CSP));
    }
    r
}

/// Store a photo the user chose, scaled down to at most 1600 px.
fn save_product(id: &str, data: &[u8]) -> Result<()> {
    let img = media::load_image(data)?;
    let img = if img.width() > 1600 || img.height() > 1600 {
        img.thumbnail(1600, 1600)
    } else {
        img
    };
    let dir = paths::vendor(id);
    std::fs::create_dir_all(&dir)?;
    let mut png = Vec::new();
    img.to_rgba8()
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| Error::Internal(e.to_string()))?;
    std::fs::write(dir.join("product-user.png"), png)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(port: u16, headers: &[(&str, &str)]) -> bool {
        allowed(port, |name| {
            headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.to_string())
        })
    }

    #[test]
    fn api_only_answers_its_own_page() {
        let own = [("Host", "127.0.0.1:8765"), ("X-Halcyon", "1")];
        assert!(check(8765, &own));
        assert!(check(
            8765,
            &[("host", "LOCALHOST:8765"), ("x-halcyon", "1")]
        ));
        assert!(check(
            8765,
            &[
                own[0],
                own[1],
                ("Origin", "http://127.0.0.1:8765"),
                ("Sec-Fetch-Site", "same-origin")
            ]
        ));
        // the custom header is required (a cross-site page cannot add it without a preflight)
        assert!(!check(8765, &own[..1]));
        assert!(!check(8765, &[own[0], ("X-Halcyon", "0")]));
        for host in [
            "",
            "127.0.0.1",
            "127.0.0.1:1",
            "evil.example:8765",
            "[::1]:8765",
            "127.0.0.1.:8765",
        ] {
            assert!(!check(8765, &[("Host", host), own[1]]), "Host {host}");
        }
        assert!(!check(8765, &[own[1]]), "missing Host");
        for origin in [
            "null",
            "http://evil.example",
            "https://127.0.0.1:8765",
            "http://localhost:8765",
        ] {
            assert!(
                !check(8765, &[own[0], own[1], ("Origin", origin)]),
                "Origin {origin}"
            );
        }
        for site in ["cross-site", "same-site"] {
            assert!(
                !check(8765, &[own[0], own[1], ("Sec-Fetch-Site", site)]),
                "{site}"
            );
        }
        assert!(check(80, &[("Host", "localhost"), own[1]]));
    }

    #[test]
    fn every_response_carries_the_security_headers() {
        let has = |r: &Response<Cursor<Vec<u8>>>, name: &'static str| {
            r.headers().iter().any(|h| h.field.equiv(name))
        };
        let html = response(Reply::Bytes(200, "text/html; charset=utf-8", vec![]));
        let api = response(Reply::Json(403, json!({})));
        for name in [
            "X-Content-Type-Options",
            "Referrer-Policy",
            "X-Frame-Options",
            "Cross-Origin-Opener-Policy",
            "Cross-Origin-Resource-Policy",
        ] {
            assert!(has(&html, name) && has(&api, name), "{name}");
        }
        assert!(has(&html, "Content-Security-Policy") && !has(&api, "Content-Security-Policy"));
        assert!(!has(&api, "Access-Control-Allow-Origin"));
    }

    #[test]
    fn permits_are_bounded_and_returned() {
        let n = AtomicUsize::new(0);
        let a = Permit::take(&n, 2).unwrap();
        let _b = Permit::take(&n, 2).unwrap();
        assert!(Permit::take(&n, 2).is_none());
        drop(a);
        assert!(Permit::take(&n, 2).is_some());
    }
}
