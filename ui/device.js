/* The keyboard drawn as the real product: displays on a raised top bar, keys below.
   Every drawing of the keyboard shows the same display content, so a GIF chosen on the Display page
   also plays on the Lighting and Keys pages. */

const screens = {
  matrix: { pixels: null, delays: [], label: "" },   // Uint8Array of frames * 315 * 3
  lcd: { src: null, label: "" },                      // image URL (animated GIF)
};
const matrixViews = new Set();
const lcdViews = new Set();
// Drop the views whose element has left the page. The draw loop does this too, but it pauses while the window is hidden.
function pruneViews() {
  for (const v of matrixViews) if (!v.c.isConnected) matrixViews.delete(v);
  for (const img of lcdViews) if (!img.isConnected) lcdViews.delete(img);
}

// An LED matrix drawn as round LEDs that light up in their colour.
class MatrixView {
  constructor(canvas, w, h, src = null) {
    this.c = canvas;
    this.src = src;  // its own animation (style thumbnails); null = whatever the keyboard's matrix shows
    this.w = w;
    this.h = h;
    this.frame = 0;
    this.next = 0;
    this.shown = undefined;  // the pixels and frame now on the canvas
    this.ctx = canvas.getContext("2d");
    canvas.style.aspectRatio = `${w} / ${h}`;
    matrixViews.add(this);
  }
  resize() {
    const r = this.c.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    if (!r.width) return false;
    const cw = Math.round(r.width * dpr), ch = Math.round(r.width * dpr * this.h / this.w);
    if (this.c.width !== cw || this.c.height !== ch) {
      this.c.width = cw;
      this.c.height = ch;
      this.shown = undefined;  // resizing a canvas clears it
    }
    return true;
  }
  draw(now) {
    if (!this.resize()) return;
    const src = this.src || screens.matrix;
    const { ctx, w, h } = this;
    const cell = this.c.width / w;
    const n = w * h;
    const frames = src.pixels ? Math.floor(src.pixels.length / (n * 3)) : 0;
    if (frames && now >= this.next) {
      if (this.next) this.frame = (this.frame + 1) % frames;  // the first draw keeps the chosen start frame
      this.next = now + (src.delays[this.frame] || 80);
    }
    // Most ticks change nothing; repaint only for a new frame, new pixels or a new size.
    if (this.shown === src.pixels && this.shownFrame === this.frame) return;
    this.shown = src.pixels;
    this.shownFrame = this.frame;
    ctx.fillStyle = "#050505";
    ctx.fillRect(0, 0, this.c.width, this.c.height);
    const o = (this.frame % Math.max(1, frames)) * n * 3;
    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; x++) {
        const i = o + (y * w + x) * 3;
        const r = frames ? src.pixels[i] : 0, g = frames ? src.pixels[i + 1] : 0, b = frames ? src.pixels[i + 2] : 0;
        const cx = (x + 0.5) * cell, cy = (y + 0.5) * cell;
        const lit = r + g + b > 24;
        if (lit) {  // light spill around a lit LED
          ctx.fillStyle = `rgba(${r},${g},${b},0.22)`;
          ctx.beginPath(); ctx.arc(cx, cy, cell * 0.62, 0, 7); ctx.fill();
        }
        ctx.fillStyle = lit ? `rgb(${r},${g},${b})` : "#161616";
        ctx.beginPath(); ctx.arc(cx, cy, cell * 0.36, 0, 7); ctx.fill();
      }
    }
  }
}

// Draw the product around a keyboard element: adds the display bar above the keys when the keyboard has one.
function makeDevice(kb) {
  const board = kb.parentElement;
  board.querySelector(".dev-top")?.remove();
  const f = def.features || {};
  board.classList.toggle("device", !!(f.matrix || f.lcd));
  pruneViews();  // screens of boards drawn before
  if (!f.matrix && !f.lcd) return;
  const top = document.createElement("div");
  top.className = "dev-top" + (f.matrix ? "" : " no-matrix");
  if (f.matrix) {
    const m = document.createElement("div");
    m.className = "dev-matrix";
    const c = document.createElement("canvas");
    c.setAttribute("role", "img");
    c.setAttribute("aria-label", "LED matrix preview");
    m.appendChild(c);
    top.appendChild(m);
    new MatrixView(c, f.matrix.width, f.matrix.height);
  }
  if (f.lcd) {
    const l = document.createElement("div");
    l.className = "dev-lcd";
    l.style.aspectRatio = `${f.lcd.width} / ${f.lcd.height}`;
    const img = document.createElement("img");
    img.alt = "Screen preview";
    l.appendChild(img);
    top.appendChild(l);
    lcdViews.add(img);
    if (screens.lcd.src) img.src = screens.lcd.src;
    else drawClock(l);
  }
  const knob = document.createElement("div");
  knob.className = "dev-knob";
  top.appendChild(knob);
  board.insertBefore(top, kb);
}

// Until a screen image is chosen, the screen shows the clock, as the keyboard does after a clock sync.
function drawClock(box) {
  box.classList.add("clock");
  box.dataset.time = new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}
setInterval(() => {
  for (const box of document.querySelectorAll(".dev-lcd.clock")) drawClock(box);
}, 15000);

function setScreen(target, data) {
  if (target === "matrix") {
    screens.matrix = data;
    for (const v of matrixViews) if (!v.src) { v.frame = 0; v.next = 0; }  // views with their own animation play on
  } else {
    screens.lcd = data;
    for (const img of lcdViews) {
      if (!img.isConnected) { lcdViews.delete(img); continue; }
      if (data.src) { img.src = data.src; img.parentElement.classList.remove("clock"); }
      else { img.removeAttribute("src"); drawClock(img.parentElement); }
    }
  }
}

function b64ToBytes(s) {
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

// Show the keyboard's original boot animation until the user picks something else.
async function loadDefaultScreens() {
  const m = def.features?.matrix, id = deviceId;
  if (!m) return;
  const r = def.originals?.boot ? await api(`${devPath(id)}/originals/boot`) : { ok: false };
  if (id !== deviceId || screens.matrix.userSet) return;
  let px = null;
  try { if (r.ok) px = b64ToBytes(r.pixels); } catch (e) {}
  if (px) {
    setScreen("matrix", { pixels: px, delays: Array.isArray(r.delays) ? r.delays : [], label: "Original boot animation" });
    // Boot animations fade in from black; start the preview on the brightest frame so it never looks off.
    const size = m.width * m.height * 3;
    let best = 0, bestSum = -1;
    for (let f = 0; f * size < px.length; f++) {
      let s = 0;
      for (let i = f * size; i < (f + 1) * size; i++) s += px[i];
      if (s > bestSum) { bestSum = s; best = f; }
    }
    for (const v of matrixViews) if (!v.src) v.frame = best;
  } else {
    // No original to show: play one of Halcyon's own styles rather than leaving the matrix dark.
    const st = HALCYON_STYLES.find(s => s.name === "Northern lights");
    const made = st.make(m.width, m.height);
    setScreen("matrix", { pixels: made.pixels, delays: Array(made.frames).fill(made.delay), label: st.name });
  }
  VIEWS.display?.status?.();  // the Display page names what the matrix shows
}

(function loop(now) {
  requestAnimationFrame(loop);
  for (const v of matrixViews) {
    if (!v.c.isConnected) { matrixViews.delete(v); continue; }
    v.draw(now);
  }
})(performance.now());
