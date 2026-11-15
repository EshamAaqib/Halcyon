const $ = id => document.getElementById(id);
const SWATCHES = ["#00ffff", "#ffffff", "#ff2d55", "#ff7a00", "#ffd60a", "#c6f24e", "#30d158", "#0a84ff", "#ff4fd8"];
const KIND = { off: "Backlight off", static: "Solid colour", animated: "Animated", rainbow: "Rainbow only", directional: "Directional" };
const CONTROL_NAMES = { color: "colour", rainbow: "rainbow", brightness: "brightness", speed: "speed", direction: "direction" };
// HID usage code -> KeyboardEvent.code, so the preview lights the key you press.
const HID_TO_CODE = (() => {
  const m = {};
  for (let i = 0; i < 26; i++) m[0x04 + i] = "Key" + String.fromCharCode(65 + i);
  for (let i = 1; i <= 9; i++) m[0x1d + i] = "Digit" + i;
  Object.assign(m, {
    0x27: "Digit0", 0x28: "Enter", 0x29: "Escape", 0x2a: "Backspace", 0x2b: "Tab", 0x2c: "Space", 0x2d: "Minus", 0x2e: "Equal",
    0x2f: "BracketLeft", 0x30: "BracketRight", 0x31: "Backslash", 0x33: "Semicolon", 0x34: "Quote", 0x35: "Backquote",
    0x36: "Comma", 0x37: "Period", 0x38: "Slash", 0x39: "CapsLock", 0x4a: "Home", 0x4b: "PageUp", 0x4c: "Delete", 0x4d: "End",
    0x4e: "PageDown", 0x4f: "ArrowRight", 0x50: "ArrowLeft", 0x51: "ArrowDown", 0x52: "ArrowUp",
    0xe0: "ControlLeft", 0xe1: "ShiftLeft", 0xe2: "AltLeft", 0xe3: "MetaLeft", 0xe4: "ControlRight", 0xe5: "ShiftRight", 0xe6: "AltRight",
  });
  return m;
})();

let devices = [];
let deviceId = null;
let def = null;
let state = null;
let keys = [];
let sendTimer = null;

// Never throws: a backend that is down, or answers with something that is not JSON, reads as { ok: false, error }.
// Every /api request carries X-Halcyon: a header no other website can make a browser send cross-site.
const API_HEADERS = { "X-Halcyon": "1" };
async function api(path, body) {
  const opts = body === undefined ? { headers: API_HEADERS }
    : body instanceof Blob ? { method: "POST", headers: API_HEADERS, body }  // files go up as they are
    : { method: "POST", headers: { ...API_HEADERS, "Content-Type": "application/json" }, body: JSON.stringify(body) };
  let res;
  try { res = await fetch(path, opts); }
  catch (e) { return { ok: false, error: "Halcyon's backend is not running" }; }
  try {
    const j = await res.json();
    if (!res.ok && j && typeof j === "object") return { ...j, ok: false, error: j.error || `Request failed (${res.status})` };
    return j && typeof j === "object" ? j : { ok: false, error: "Unexpected answer from Halcyon's backend" };
  } catch (e) { return { ok: false, error: `Request failed (${res.status})` }; }
}
// Ids come from definition files: encoded so one can never reach another endpoint.
const devPath = id => "/api/devices/" + encodeURIComponent(id);
// Images from the API are fetched with the header and shown as blob: URLs (an <img src> cannot send headers).
const blobUrls = {};
async function apiImage(path) {
  if (blobUrls[path]) return blobUrls[path];
  try {
    const res = await fetch(path, { headers: API_HEADERS });
    if (!res.ok || !(res.headers.get("Content-Type") || "").startsWith("image/")) return null;
    return (blobUrls[path] = URL.createObjectURL(await res.blob()));
  } catch (e) { return null; }
}
// Only images the server made (data:image/...) may be put in a src.
const safeImage = s => typeof s === "string" && /^data:image\/(png|gif|jpeg|webp);base64,/i.test(s) ? s : null;

function toast(msg) {
  const t = $("toast");
  t.textContent = msg || "Something went wrong";
  t.classList.add("show");
  clearTimeout(t._h);
  t._h = setTimeout(() => t.classList.remove("show"), 3200);
}

// Rebuilding a list drops keyboard focus; put it back on the control in the same position.
function keepFocus(box, rebuild) {
  const stops = "button, input, select, [tabindex]";
  const at = [...box.querySelectorAll(stops)].indexOf(document.activeElement);
  rebuild();
  if (at < 0) return;
  const now = box.querySelectorAll(stops);
  now[Math.min(at, now.length - 1)]?.focus();
}

const hex = c => "#" + c.map(v => v.toString(16).padStart(2, "0")).join(""); // taste-skill-ignore: hex colour
const rgb = h => { const n = parseInt(h.slice(1), 16); return [n >> 16 & 255, n >> 8 & 255, n & 255]; };
const effectById = id => def?.lighting.effects.find(e => e.id === id);
const effectName = e => e.name || `Effect ${e.id}`;

/* ---------- Routing ---------- */
const TITLES = { lighting: "Lighting", keys: "Keys", macros: "Macros", display: "Display", settings: "Settings", devices: "Keyboards" };
const VIEWS = {};      // filled by views.js: { name: { load(), show() } }
let config = null;     // keymap, macros, perkey, settings for the selected keyboard
let demoMode = false;
let lastStatus = {};
let view = "lighting";
// A view that is listening for raw key presses (macro recording, "Press a key") says so with grabbing().
const keysGrabbed = () => Object.values(VIEWS).some(v => v.grabbing?.());
const releaseKeys = () => { for (const v of Object.values(VIEWS)) v.release?.(); };
function route() {
  const hash = location.hash.slice(1);
  const v = TITLES[hash] ? hash : "lighting";
  if (v !== view) releaseKeys();  // a recording must not keep swallowing keys on another page
  view = v;
  for (const p of document.querySelectorAll("[data-page]")) p.classList.toggle("hidden", p.dataset.page !== v);
  for (const a of document.querySelectorAll(".nav a")) a.classList.toggle("active", a.dataset.view === v);
  $("viewTitle").textContent = TITLES[v];
  if (v === "devices") renderDevices();
  if (def) VIEWS[v]?.show?.();
  window.moveNavIndicator?.();
}
window.addEventListener("hashchange", route);

/* ---------- Devices & switcher ---------- */
// Returns true when the list of keyboards changed, so callers redraw only then.
async function loadDevices() {
  const r = await api("/api/devices");
  if (!r.devices) return false;
  const changed = JSON.stringify(r.devices) !== JSON.stringify(devices);
  devices = r.devices;
  demoMode = !!r.demo;
  $("folderPath").textContent = "Your devices folder: " + r.user_folder;
  $("problems").innerHTML = (r.problems || []).map(p => `<div class="problem">Could not load ${escapeHtml(p.file)}: ${escapeHtml(p.error)}</div>`).join("");
  if (changed) renderMenu();
  return changed;
}

function connLabel(c) { return c === "dongle" ? "2.4 GHz" : c === "wired" ? "USB cable" : c; }

function renderMenu() {
  const m = $("menu");
  m.innerHTML = "";
  for (const d of devices) {
    const b = document.createElement("button");
    b.className = d.id === deviceId ? "current" : "";
    b.setAttribute("role", "menuitem");
    b.innerHTML = (d.has_product ? `<img class="menu-thumb" data-product="${escapeHtml(d.id)}" alt="">` : "") +
      `<span><b style="font-weight:500">${escapeHtml(d.vendor)} ${escapeHtml(d.name)}</b></span><span class="meta">${d.connected ? escapeHtml(connLabel(d.connected)) : "Not connected"}</span>`;
    b.onclick = () => { closeMenu(); selectDevice(d.id); };
    m.appendChild(b);
  }
  fillProducts(m);
  const hr = document.createElement("hr");
  m.appendChild(hr);
  const add = document.createElement("button");
  add.setAttribute("role", "menuitem");
  add.textContent = "Add a keyboard…";
  add.onclick = () => { closeMenu(); location.hash = "devices"; };
  m.appendChild(add);
}
function closeMenu() { $("menu").classList.remove("open"); $("switchBtn").setAttribute("aria-expanded", "false"); }
$("switchBtn").onclick = e => {
  e.stopPropagation();
  const open = $("menu").classList.toggle("open");
  $("switchBtn").setAttribute("aria-expanded", String(open));
};
document.addEventListener("click", e => { if (!$("menu").contains(e.target)) closeMenu(); });
document.querySelector(".switcher").addEventListener("keydown", e => {
  if (e.key !== "Escape" || !$("menu").classList.contains("open")) return;
  closeMenu();
  $("switchBtn").focus();
});

// A small drawing of a keyboard from its layout, for the Keyboards page.
const layoutCache = {};
function boardSvg(layout) {
  if (!Array.isArray(layout?.keys)) return "";
  const u = 10, g = 1.4, main = layout.keys.filter(k => !k.extra);
  const n = v => +v || 0;  // definition values: only numbers reach the markup
  const rects = main.map(k => `<rect x="${n(k.x) * u + g / 2}" y="${n(k.y) * u + g / 2}" width="${Math.max(0, n(k.w) * u - g)}" height="${u - g}" rx="1.6"/>`).join("");
  return `<svg viewBox="0 0 ${n(layout.width) * u} ${n(layout.height) * u}" aria-hidden="true">${rects}</svg>`;
}
let devicesTurn = 0;
async function renderDevices() {
  const grid = $("devGrid"), turn = ++devicesTurn;
  for (const d of devices) {
    if (!(d.id in layoutCache)) layoutCache[d.id] = (await api(devPath(d.id))).definition?.layout || null;
  }
  if (turn !== devicesTurn) return;  // a newer redraw has started
  grid.innerHTML = devices.map(d => `
    <section class="card kbcard ${d.id === deviceId ? "current" : ""}">
      <div class="kbart ${d.has_product ? "photo" : ""}">
        ${d.has_product ? `<img data-product="${escapeHtml(d.id)}" alt="${escapeHtml(d.vendor)} ${escapeHtml(d.name)}">` : boardSvg(layoutCache[d.id])}
        <label class="btn file photo-btn">${d.has_product ? "Change photo" : "Add photo"}<input type="file" accept="image/*" data-photo="${escapeHtml(d.id)}"></label>
      </div>
      <div class="kbmeta">
        <div><h2>${escapeHtml(d.vendor)} ${escapeHtml(d.name)}</h2>
          <p class="hint">${escapeHtml((d.connections || []).map(connLabel).join(" or "))}${d.source === "user" ? " · from your folder" : ""}</p></div>
        <div class="foot-actions">
          ${d.connected ? `<span class="pill ok">Connected · ${escapeHtml(connLabel(d.connected))}</span>` : `<span class="pill">Not connected</span>`}
          ${d.id === deviceId ? `<span class="stamp">In use</span>` : `<button class="btn" data-use="${escapeHtml(d.id)}">Use</button>`}
        </div>
      </div>
    </section>`).join("");
  fillProducts(grid);
  for (const b of grid.querySelectorAll("[data-use]")) b.onclick = () => selectDevice(b.dataset.use);
  for (const inp of grid.querySelectorAll("[data-photo]")) inp.onchange = async () => {
    const f = inp.files[0];
    if (!f) return;
    const r = await api(`${devPath(inp.dataset.photo)}/product`, f);
    if (!r.ok) { toast(r.error); return; }
    photoVersion = Date.now();
    for (const k of Object.keys(blobUrls)) if (k.includes("/product?")) { URL.revokeObjectURL(blobUrls[k]); delete blobUrls[k]; }
    await loadDevices();
    renderMenu();
    renderDevices();
    renderSwitcher();
  };
}

let photoVersion = Date.now();
const productUrl = id => `${devPath(id)}/product?v=${photoVersion}`;
// Fills each <img data-product="id"> under root with that keyboard's photo.
function fillProducts(root) {
  for (const img of root.querySelectorAll("img[data-product]")) {
    apiImage(productUrl(img.dataset.product)).then(u => { if (u) img.src = u; else img.remove(); });
  }
}
function renderSwitcher() {
  const d = devices.find(x => x.id === deviceId);
  const thumb = $("devThumb");
  thumb.classList.add("hidden");
  if (d?.has_product) apiImage(productUrl(d.id)).then(u => { if (u && d.id === deviceId) { thumb.src = u; thumb.classList.remove("hidden"); } });
}
$("openFolder").onclick = async () => { const r = await api("/api/open-user-folder", {}); if (!r.ok) toast(r.error); };
$("reload").onclick = async () => {
  const r = await api("/api/reload", {});
  if (!r.ok) { toast(r.error); return; }
  for (const id of Object.keys(layoutCache)) delete layoutCache[id];  // a definition may have a new layout
  await loadDevices();
  renderDevices();
  toast(r.problems?.length ? `Reloaded with ${r.problems.length} problem(s), see the list above` : "Keyboards reloaded");
};

let selecting = 0;
async function selectDevice(id) {
  const turn = ++selecting;
  const r = await api(`${devPath(id)}`);
  if (turn !== selecting) return;  // another keyboard was chosen while this one was loading
  if (!r.definition) { toast(r.error || "That keyboard could not be opened"); return; }
  clearTimeout(sendTimer);  // a change still waiting to be sent belongs to the previous keyboard
  deviceId = id;
  try { localStorage.setItem("halcyon.device", id); } catch (e) {}
  def = r.definition;
  def.lighting = { effects: [], ...def.lighting };
  for (const e of def.lighting.effects) if (!Array.isArray(e.controls)) e.controls = [];
  state = { effect: 0, color: [0, 255, 255], brightness: 5, speed: 3, direction: 0, rainbow: false, ...r.lighting };
  // A missing or null part of the saved config reads as empty, never as null.
  const c = r.config || {};
  config = { ...c, keymap: c.keymap || {},
    macros: (Array.isArray(c.macros) ? c.macros : []).filter(m => m && Array.isArray(m.events)), perkey: c.perkey || {}, settings: c.settings || {} };
  const lit = def.lighting.effects.find(e => e.preview !== "off") || def.lighting.effects[0];
  if (!effectById(state.effect)) state.effect = lit?.id;
  lastOn = lit?.id;
  hoverEffect = null;
  presses = [];
  $("stamp").textContent = "";
  const d = devices.find(x => x.id === id) || def;
  $("devName").textContent = [d.vendor, d.name].filter(Boolean).join(" ") || id;
  renderMenu();
  renderSwitcher();
  $("power").classList.toggle("hidden", !def.lighting.effects.some(e => e.preview === "off"));
  if (view === "devices") renderDevices();
  // Screens first: the boards drawn below show them, and they must not show the previous keyboard's.
  screens.matrix = { pixels: null, delays: [] };
  screens.lcd = { src: null };
  buildBoard();
  sweepStart = performance.now();  // the board "powers on" when a keyboard is opened
  buildEffects();
  buildTune();
  render();
  for (const v of Object.values(VIEWS)) v.load?.();
  loadDefaultScreens();
  VIEWS[view]?.show?.();
  pollStatus();
}

/* ---------- Status ---------- */
async function pollStatus() {
  const id = deviceId;
  if (!id || !accepted) return;
  let s = await api(`${devPath(id)}/status`);
  if (id !== deviceId) return;  // the keyboard was switched while this was on its way
  if (s.ok === false && !("connection" in s)) s = { connection: null, error: "Halcyon's backend stopped" };
  lastStatus = s;
  VIEWS.display?.status?.(s);
  if (s.busy) { $("statusText").textContent = "Sending to the keyboard…"; return; }
  const dot = $("dot");
  if (!s.connection) {
    dot.className = "dot err";
    $("statusText").textContent = s.error || "Not connected";
    $("devVendor").textContent = "Not connected";
  } else if (!s.linked) {
    dot.className = "dot warn";
    $("statusText").textContent = `${connLabel(s.connection)} receiver found · keyboard asleep or off`;
    $("devVendor").textContent = connLabel(s.connection);
  } else {
    dot.className = "dot ok";
    $("statusText").textContent = "Connected · " + connLabel(s.connection);
    $("devVendor").textContent = connLabel(s.connection);
  }
  const hasBatt = s.linked && typeof s.battery === "number" && s.connection !== "wired";
  $("batt").classList.toggle("hidden", !hasBatt);
  if (hasBatt) {
    $("battBar").style.width = Math.max(4, Math.min(100, s.battery)) + "%";
    $("battBar").classList.toggle("low", s.battery <= 20);
    $("battText").textContent = s.battery + "%";
  }
}

/* ---------- Keyboard render ---------- */
function buildBoard() {
  keys = makeBoard($("kb"));
  const g = document.createElement("canvas");  // the keys' light spilling onto the case
  g.className = "kb-glow";
  g.width = 160;
  g.height = 50;
  g.setAttribute("aria-hidden", "true");
  $("kb").prepend(g);
  glowCtx = g.getContext("2d");
  for (const key of keys) {
    key.el.setAttribute("role", "button");
    key.el.setAttribute("aria-label", "Paint " + (key.k.label || "key"));
    key.el.onclick = () => paintKey(key.k.led);
    key.el.onkeydown = e => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); paintKey(key.k.led); } };
  }
}

/* ---------- Per-key painting ---------- */
const isPerkey = () => !!effectById(state.effect)?.perkey;
function paintKey(led) {
  if (!isPerkey()) return;
  config.perkey[led] = [...state.color];
  update({});
}
$("paintFill").onclick = () => {
  for (const k of def.layout.keys) if (!k.extra) config.perkey[k.led] = [...state.color];
  update({});
};
$("paintClear").onclick = () => { config.perkey = {}; update({}); };

// Draw the keyboard from its layout into `kb`; returns [{el, x, y, code, k}] for each key.
function makeBoard(kb) {
  kb.innerHTML = "";
  const keys = [];
  const L = def.layout;
  kb.style.aspectRatio = `${L.width} / ${L.height}`;
  const gap = 0.12;
  for (const k of L.keys) {
    if (k.extra) continue;
    const el = document.createElement("div");
    el.className = "key";
    el.style.left = `${(k.x / L.width) * 100}%`;
    el.style.top = `${(k.y / L.height) * 100}%`;
    el.style.width = `calc(${(k.w / L.width) * 100}% - ${(gap / L.width) * 100}%)`;
    el.style.height = `${((1 - gap) / L.height) * 100}%`;
    const cap = document.createElement("div");
    cap.className = "cap";
    const legend = document.createElement("span");
    legend.textContent = k.label;
    cap.appendChild(legend);
    el.appendChild(cap);
    kb.appendChild(el);
    keys.push({ el, x: (k.x + k.w / 2) / L.width, y: (k.y + 0.5) / L.height, code: HID_TO_CODE[k.code], k });
  }
  makeDevice(kb);
  return keys;
}

function hsl(h, s, l) {
  const a = s * Math.min(l, 1 - l);
  const f = n => { const k = (n + h / 30) % 12; return l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1)); };
  return [f(0) * 255, f(8) * 255, f(4) * 255];
}

// Approximate preview of each effect; the real firmware animation may differ in detail.
let presses = [];
let lastOn = 1;
let sweepStart = -1e9;
let hoverEffect = null;
let glowCtx = null;
function drawGlow(vals) {
  if (!glowCtx || !glowCtx.canvas.isConnected) return;
  const c = glowCtx.canvas, L = def.layout, rh = c.height / L.height;
  glowCtx.clearRect(0, 0, c.width, c.height);
  keys.forEach((k, i) => {
    glowCtx.fillStyle = `rgb(${vals[i * 3] | 0},${vals[i * 3 + 1] | 0},${vals[i * 3 + 2] | 0})`;
    const wpx = k.k.w / L.width * c.width;
    glowCtx.fillRect(k.x * c.width - wpx / 2, k.y * c.height - rh / 2, wpx, rh);
  });
}
window.addEventListener("keyup", e => { keys.find(k => k.code === e.code)?.el.classList.remove("down"); });
window.addEventListener("blur", () => { for (const k of keys) k.el.classList.remove("down"); });  // keyup never arrives after Alt+Tab
const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;
$("power").onclick = () => {
  const offEffect = def.lighting.effects.find(e => e.preview === "off");
  const isOff = effectById(state.effect)?.preview === "off";
  update({ effect: isOff ? lastOn : offEffect.id });
};
const seed = i => { const x = Math.sin(i * 127.1) * 43758.5453; return x - Math.floor(x); };
function frame(now) {
  requestAnimationFrame(frame);
  if (def && state && view === "lighting") {
    const e = effectById(hoverEffect ?? state.effect);  // hovering an effect previews it without sending
    const kind = e ? e.preview : "static";
    const can = c => !!e && e.controls.includes(c);
    const level = can("brightness") ? state.brightness / (def.lighting.brightness_max || 5) : 1;
    const glow = [];
    const speed = 0.4 + (state.speed / (def.lighting.speed_max || 5)) * 1.8;
    const t = now / 1000 * speed;
    const aspect = def.layout.height / def.layout.width;
    presses = presses.filter(p => now - p.t < 1400);
    keys.forEach((k, n) => {
      let c = state.color, i = 1;
      const pos = [k.x, 1 - k.x, 1 - k.y, k.y][state.direction] ?? k.x;
      const rainbowHue = ((k.x * 300) - t * 50) % 360 + 360;
      let rainbow = state.rainbow && can("rainbow");
      switch (kind) {
        case "off": i = 0; break;
        case "perkey": c = config.perkey[k.k.led] || [0, 0, 0]; break;
        case "animated": i = 0.15 + 0.85 * (0.5 + 0.5 * Math.sin(t * 2.2)); break;
        case "spectrum": c = hsl((t * 45) % 360, 1, 0.55); break;
        case "rainbow": rainbow = true; break;
        case "twinkle": i = 0.12 + 0.88 * Math.pow(Math.max(0, Math.sin(t * 1.6 + seed(n) * 40)), 8); break;
        case "outward": {
          const d = Math.hypot(k.x - 0.5, (k.y - 0.5) * aspect);
          i = 0.15 + 0.85 * Math.pow(0.5 + 0.5 * Math.sin(d * 18 - t * 4), 2);
          break;
        }
        case "shuttle": { const band = 0.5 + 0.55 * Math.sin(t * 1.4); i = 0.08 + 0.92 * Math.exp(-((k.x - band) ** 2) / 0.006); break; }
        case "directional": i = 0.15 + 0.85 * Math.pow(0.5 + 0.5 * Math.sin(pos * 10 - t * 3.5), 2); break;
        case "reactive":
        case "reactive-key":
        case "reactive-off": {
          let hit = 0;
          for (const p of presses) {
            const age = (now - p.t) / 1000;
            const d = Math.hypot(k.x - p.x, (k.y - p.y) * aspect);
            const ring = kind === "reactive" ? Math.exp(-((d - age * 0.9) ** 2) / 0.002) * Math.max(0, 1 - age / 1.2) : 0;
            const self = d < 0.01 ? Math.max(0, 1 - age / 0.8) : 0;
            hit = Math.max(hit, ring, self);
          }
          i = kind === "reactive-off" ? 1 - hit : hit;
          break;
        }
      }
      if (rainbow) c = hsl(kind === "directional" ? ((pos * 360) - t * 60) % 360 + 360 : rainbowHue, 1, 0.55);
      i *= level;
      // Switching effect: the new light sweeps across the board with a bright leading edge.
      let edgeGlow = 0;
      if (!reduceMotion) {
        const edge = (now - sweepStart) / 520 * 1.25 - k.x;
        if (edge < 0) i *= 0.12;
        else if (edge < 0.1) edgeGlow = 1 - edge / 0.1;
      }
      // Any key you press gives a short flash, whatever the effect.
      if (!kind.startsWith("reactive")) {
        for (const p of presses) {
          if (Math.hypot(k.x - p.x, k.y - p.y) < 0.01) edgeGlow = Math.max(edgeGlow, 0.7 * Math.max(0, 1 - (now - p.t) / 450));
        }
      }
      if (edgeGlow) c = c.map(v => v + (255 - v) * edgeGlow * 0.6);
      const a = Math.min(1, 0.05 + i * 0.85 + edgeGlow * 0.5);
      const mix = (from, amt) => c.map(v => Math.round(from + (v - from) * amt)).join(", ");
      k.el.style.setProperty("--k", mix(26, a));
      k.el.style.setProperty("--g", mix(110, Math.min(1, 0.2 + a * 0.8)));
      const ga = Math.max(0, a - 0.05);
      glow.push(c[0] * ga, c[1] * ga, c[2] * ga);
    });
    drawGlow(glow);
  }
}

// Type on the real keyboard and the preview reacts, which is how the typing effects are seen.
window.addEventListener("keydown", e => {
  if (view !== "lighting" || e.target.tagName === "INPUT" || e.repeat) return;
  const k = keys.find(k => k.code === e.code);
  if (k) { presses.push({ x: k.x, y: k.y, t: performance.now() }); k.el.classList.add("down"); }
});

/* ---------- Effects ---------- */
const PENCIL = `<svg width="13" height="13" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.4"><path d="M9.5 2.5l2 2L5 11H3V9z"/></svg>`;
function buildEffects() {
  const box = $("effects");
  box.innerHTML = "";
  hoverEffect = null;  // the tile under the pointer is about to go, and with it its mouseleave
  const named = def.lighting.groups || [];
  // Effects whose group the definition does not list (or that have none) still get a place, after the others.
  const groups = [...named, { name: "", rest: true }];
  for (const g of groups) {
    if (g.id === "off") continue;  // Off is the power button under the keyboard
    const list = def.lighting.effects.filter(e => g.rest
      ? !named.some(n => n.id === e.group) && !(e.preview === "off" && named.length) : e.group === g.id);
    if (!list.length) continue;
    const sec = document.createElement("div");
    sec.className = "fx-group";
    if (g.name) sec.innerHTML = `<h3>${escapeHtml(g.name)}</h3>`;
    const grid = document.createElement("div");
    grid.className = "effects";
    for (const e of list) {
      const b = document.createElement("div");
      b.className = "fx";
      b.dataset.id = e.id;
      b.tabIndex = 0;
      b.setAttribute("role", "button");
      b.title = e.description || "";
      b.innerHTML = `<span class="name ${e.name ? "" : "unnamed"}">${escapeHtml(effectName(e))}</span>` +
        `<span class="kind">${escapeHtml(e.description || "")}</span><span class="swatch"></span>` +
        (!e.name || e.named_by_user ? `<button class="rename" title="Name this effect" aria-label="Name ${escapeHtml(effectName(e))}">${PENCIL}</button>` : "");
      b.onclick = ev => { if (!ev.target.closest(".rename") && ev.target.tagName !== "INPUT") update({ effect: e.id }); };
      b.onmouseenter = () => { if (e.id !== state.effect) { hoverEffect = e.id; sweepStart = performance.now(); } };
      b.onmouseleave = () => { hoverEffect = null; };
      b.onkeydown = ev => { if ((ev.key === "Enter" || ev.key === " ") && ev.target === b) { ev.preventDefault(); update({ effect: e.id }); } };
      const rn = b.querySelector(".rename");
      if (rn) rn.onclick = ev => { ev.stopPropagation(); startRename(b, e); };
      grid.appendChild(b);
    }
    sec.appendChild(grid);
    box.appendChild(sec);
  }
}
function escapeHtml(s) { return String(s ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c])); }

function startRename(tile, e) {
  const name = tile.querySelector(".name");
  const input = document.createElement("input");
  input.value = e.name || "";
  input.placeholder = `Effect ${e.id}`;
  input.maxLength = 40;
  input.setAttribute("aria-label", "Effect name");
  name.replaceWith(input);
  input.focus();
  input.select();
  let done = false;
  const finish = async save => {
    if (done) return;
    done = true;
    const focused = document.activeElement === input;
    if (save && input.value.trim() !== (e.name || "")) {
      const r = await api(`${devPath(deviceId)}/effects/${encodeURIComponent(e.id)}/name`, { name: input.value });
      if (r.ok) { e.name = input.value.trim() || null; e.named_by_user = !!e.name; }  // stays renamable
      else toast(r.error || "The name was not saved");
    }
    buildEffects();
    render();
    if (focused) [...$("effects").querySelectorAll(".fx")].find(t => t.dataset.id === String(e.id))?.focus();
  };
  input.onkeydown = ev => { if (ev.key === "Enter") finish(true); if (ev.key === "Escape") finish(false); };
  input.onblur = () => finish(true);
}

/* ---------- Tune ---------- */
function has(control) { const e = effectById(state.effect); return !!e && e.controls.includes(control); }

function buildSeg(id, items, key) {
  const box = $(id);
  box.innerHTML = "";
  for (const it of items) {
    const b = document.createElement("button");
    b.textContent = it.label;
    b.dataset.v = it.value;
    if (it.title) { b.title = it.title; b.setAttribute("aria-label", it.title); }
    b.onclick = () => update({ [key]: it.value });
    box.appendChild(b);
  }
}

function buildTune() {
  for (const s of $("swatches").querySelectorAll(".sw")) s.remove();
  for (const c of SWATCHES) {
    const s = document.createElement("button");
    s.className = "sw";
    s.style.background = c;
    s.dataset.c = c;
    s.title = c.toUpperCase();
    s.setAttribute("aria-label", "Colour " + c);
    s.onclick = () => update({ color: rgb(c), rainbow: false });
    $("swatches").insertBefore(s, $("custom"));
  }
  $("picker").oninput = e => update({ color: rgb(e.target.value), rainbow: false });
  $("rainbow").onchange = e => update({ rainbow: e.target.checked });
  const bmax = def.lighting.brightness_max || 5, smax = def.lighting.speed_max || 5;
  buildSeg("brightness", Array.from({ length: bmax + 1 }, (_, i) => ({ value: i, label: i === 0 ? "Off" : String(i) })), "brightness");
  buildSeg("speed", Array.from({ length: smax + 1 }, (_, i) => ({ value: i, label: String(i) })), "speed");
  const ARROWS = { Right: "→", Left: "←", Up: "↑", Down: "↓" };
  buildSeg("direction", (def.lighting.directions || []).map(d => ({ value: d.value, label: ARROWS[d.name] || d.name, title: d.name })), "direction");
}

function render() {
  const e = def && effectById(state.effect);
  if (!e) return;
  const shown = state.rainbow && has("rainbow") ? [198, 120, 255] : state.color;
  document.documentElement.style.setProperty("--light", shown.join(", "));
  for (const t of document.querySelectorAll(".fx")) {
    t.classList.toggle("active", t.dataset.id === String(state.effect));
    t.setAttribute("aria-pressed", t.dataset.id === String(state.effect));
  }

  $("nowName").textContent = effectName(e);
  const off = e.preview === "off";
  if (!off) lastOn = state.effect;
  $("power").textContent = off ? "Turn backlight on" : "Turn backlight off";
  $("power").classList.toggle("primary", off);
  const typing = e.group === "typing";
  $("nowDesc").textContent = "· " + (e.description || KIND[e.preview] || "") + (typing ? ". Type something to see it here and on your keyboard." : "");
  $("tuneFor").textContent = effectName(e);

  for (const f of document.querySelectorAll("[data-control]")) f.classList.toggle("hidden", !has(f.dataset.control));
  for (const k of keys) { k.el.tabIndex = isPerkey() ? 0 : -1; k.el.setAttribute("aria-disabled", !isPerkey()); }  // keys are controls only while painting
  const missing = Object.keys(CONTROL_NAMES).filter(c => !has(c)).map(c => CONTROL_NAMES[c]);
  $("locked").classList.toggle("hidden", e.preview !== "off" && !missing.length);
  $("locked").textContent = e.preview === "off"
    ? "The backlight is off. Pick any effect to turn it back on."
    : `This effect has no ${missing.join(", ").replace(/, ([^,]*)$/, " or $1")} setting.`;
  $("locked").style.borderTop = e.controls.length ? "" : "0";
  $("locked").style.paddingTop = e.controls.length ? "" : "0";

  const h = hex(state.color);
  for (const s of $("swatches").querySelectorAll(".sw")) { s.classList.toggle("active", !state.rainbow && s.dataset.c === h); s.setAttribute("aria-pressed", !state.rainbow && s.dataset.c === h); }
  $("custom").classList.toggle("active", !state.rainbow && !SWATCHES.includes(h));
  $("picker").value = h;
  $("hex").textContent = state.rainbow && has("rainbow") ? "Rainbow" : h.toUpperCase();
  $("rainbow").checked = state.rainbow;
  for (const [id, key] of [["brightness", "brightness"], ["speed", "speed"], ["direction", "direction"]])
    for (const b of $(id).children) { b.classList.toggle("active", +b.dataset.v === state[key]); b.setAttribute("aria-pressed", +b.dataset.v === state[key]); }
  $("brightnessOut").textContent = state.brightness === 0 ? "Off" : `${state.brightness} of ${def.lighting.brightness_max || 5}`;
  $("speedOut").textContent = `${state.speed} of ${def.lighting.speed_max || 5}`;
  const dn = (def.lighting.directions || []).find(d => d.value === state.direction);
  $("directionOut").textContent = dn ? dn.name : "";
}

function update(patch) {
  if ("effect" in patch && patch.effect !== state.effect && hoverEffect !== patch.effect) sweepStart = performance.now();
  if ("effect" in patch) hoverEffect = null;
  Object.assign(state, patch);
  render();
  $("stamp").className = "stamp";
  $("stamp").textContent = "Sending…";
  clearTimeout(sendTimer);
  sendTimer = setTimeout(send, isPerkey() ? 450 : 120);  // per-key uploads are ~23 packets; batch quick clicks
}

async function send() {
  if (!gateOpen()) return;  // nothing goes to a keyboard before the disclaimer is accepted
  const id = deviceId;
  const r = isPerkey()
    ? await api(`${devPath(id)}/perkey`, { colors: config.perkey, brightness: state.brightness })
    : await api(`${devPath(id)}/lighting`, state);
  if (id !== deviceId) return;  // the keyboard was switched while this was on its way
  if (r.ok) {
    $("stamp").className = "stamp";
    $("stamp").textContent = "Applied " + new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  } else {
    $("stamp").className = "stamp err";
    $("stamp").textContent = "Not applied";
    toast(r.error || "The keyboard did not accept that");
  }
}

/* ---------- Start ---------- */
// Nothing is sent to a keyboard until the user has accepted the risk (DISCLAIMER.md); asked once.
let accepted = false;
const gateOpen = () => accepted;
async function acceptDisclaimer() {
  const d = await api("/api/disclaimer");
  if (d.accepted) { accepted = true; return; }
  const gate = $("gate");
  gate.classList.remove("hidden");
  document.querySelector(".shell").inert = true;  // nothing behind the gate can be reached, by mouse or Tab
  $("gateCheck").onchange = () => { $("gateOk").disabled = !$("gateCheck").checked; };
  $("gateCheck").focus();
  await new Promise(resolve => {
    $("gateOk").onclick = async () => {
      $("gateOk").disabled = true;
      const r = await api("/api/disclaimer", {});
      if (!r.ok) { toast(r.error || "Your answer could not be saved"); $("gateOk").disabled = !$("gateCheck").checked; return; }
      gate.classList.add("hidden");
      document.querySelector(".shell").inert = false;
      resolve();
    };
  });
  accepted = true;
}

(async function init() {
  requestAnimationFrame(frame);
  await loadDevices();               // lists keyboards only; nothing is sent to them yet
  if (demoMode) accepted = true;  // nothing reaches a device in demo mode
  else await acceptDisclaimer();
  renderMenu();
  route();
  let saved = null;
  try { saved = localStorage.getItem("halcyon.device"); } catch (e) {}
  const pick = devices.find(d => d.id === saved && d.connected) || devices.find(d => d.connected) || devices.find(d => d.id === saved) || devices[0];
  if (pick) await selectDevice(pick.id);
  else $("statusText").textContent = "No keyboards found";
  setInterval(pollStatus, 5000);
  // Redrawing only on a change keeps an open menu, a focused button or a file dialog alive.
  setInterval(async () => { if (await loadDevices() && view === "devices") renderDevices(); }, 15000);
})();
