/* Display page: LED matrix (image, pixel painting, scrolling text) and the screen. */

// A 5-pixel-tall font for the matrix. "#" = LED on.
const FONT = {
  A: [".#.", "#.#", "###", "#.#", "#.#"], B: ["##.", "#.#", "##.", "#.#", "##."], C: [".##", "#..", "#..", "#..", ".##"],
  D: ["##.", "#.#", "#.#", "#.#", "##."], E: ["###", "#..", "##.", "#..", "###"], F: ["###", "#..", "##.", "#..", "#.."],
  G: [".##", "#..", "#.#", "#.#", ".##"], H: ["#.#", "#.#", "###", "#.#", "#.#"], I: ["###", ".#.", ".#.", ".#.", "###"],
  J: ["..#", "..#", "..#", "#.#", ".#."], K: ["#.#", "#.#", "##.", "#.#", "#.#"], L: ["#..", "#..", "#..", "#..", "###"],
  M: ["#...#", "##.##", "#.#.#", "#...#", "#...#"], N: ["#..#", "##.#", "#.##", "#..#", "#..#"], O: [".#.", "#.#", "#.#", "#.#", ".#."],
  P: ["##.", "#.#", "##.", "#..", "#.."], Q: [".#.", "#.#", "#.#", "##.", ".##"], R: ["##.", "#.#", "##.", "#.#", "#.#"],
  S: [".##", "#..", ".#.", "..#", "##."], T: ["###", ".#.", ".#.", ".#.", ".#."], U: ["#.#", "#.#", "#.#", "#.#", "###"],
  V: ["#.#", "#.#", "#.#", "#.#", ".#."], W: ["#...#", "#...#", "#.#.#", "##.##", "#...#"], X: ["#.#", "#.#", ".#.", "#.#", "#.#"],
  Y: ["#.#", "#.#", ".#.", ".#.", ".#."], Z: ["###", "..#", ".#.", "#..", "###"],
  0: ["###", "#.#", "#.#", "#.#", "###"], 1: [".#.", "##.", ".#.", ".#.", "###"], 2: ["##.", "..#", ".#.", "#..", "###"],
  3: ["##.", "..#", ".#.", "..#", "##."], 4: ["#.#", "#.#", "###", "..#", "..#"], 5: ["###", "#..", "##.", "..#", "##."],
  6: [".##", "#..", "###", "#.#", "###"], 7: ["###", "..#", ".#.", ".#.", ".#."], 8: ["###", "#.#", "###", "#.#", "###"],
  9: ["###", "#.#", "###", "..#", "##."], " ": ["..", "..", "..", "..", ".."], "!": ["#", "#", "#", ".", "#"],
  "?": ["##.", "..#", ".#.", "...", ".#."], ".": [".", ".", ".", ".", "#"], ",": ["..", "..", "..", ".#", "#."],
  ":": [".", "#", ".", "#", "."], "-": ["...", "...", "###", "...", "..."], "+": ["...", ".#.", "###", ".#.", "..."],
  "'": ["#", "#", ".", ".", "."], "/": ["..#", "..#", ".#.", "#..", "#.."], "<": ["..#", ".#.", "#..", ".#.", "..#"],
  ">": ["#..", ".#.", "..#", ".#.", "#.."], "=": ["...", "###", "...", "###", "..."], "♥": [".#.#.", "#####", "#####", ".###.", "..#.."],
};

VIEWS.display = (() => {
  const W = () => def.features.matrix.width, H = () => def.features.matrix.height;
  const N = () => W() * H();
  const maxFrames = () => def.features.matrix.max_frames || 260;
  const PAINT = ["#ff2d55", "#ff7a00", "#ffd60a", "#c6f24e", "#00ffff", "#0a84ff", "#a35cff", "#ff4fd8", "#ffffff"];
  const opt = { tab: "styles", fit: "cover", speed: 10, bright: 25, both: false, lcdFit: "cover", slot: 1,
                txtColor: "#00ffff", txtStyle: "scroll", paint: "#00ffff" };
  let token = { matrix: null, lcd: null }, files = {}, frames = [], cur = 0, polling = null;
  let painting = false, txtFrames = null, chosenStyle = null, stylesBuilt = false;

  /* --- small helpers --- */
  function segment(el, items, key, after) {
    el.innerHTML = "";
    for (const [v, label] of items) {
      const b = document.createElement("button");
      b.textContent = label;
      b.dataset.v = v;
      b.onclick = () => { opt[key] = v; render(); after?.(); };
      el.appendChild(b);
    }
  }
  function swatchRow(el, key, after, withRainbow) {
    el.innerHTML = "";
    const list = withRainbow ? [...PAINT, "rainbow"] : PAINT;
    for (const c of list) {
      const b = document.createElement("button");
      b.className = "sw" + (c === "rainbow" ? " rainbow-sw" : "");
      b.dataset.c = c;
      b.title = c === "rainbow" ? "Rainbow" : c;
      b.setAttribute("aria-label", c === "rainbow" ? "Rainbow" : "Colour " + c);
      if (c !== "rainbow") b.style.background = c;
      b.onclick = () => { opt[key] = c; render(); after?.(); };
      el.appendChild(b);
    }
  }
  const toB64 = bytes => { let s = ""; for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000)); return btoa(s); };
  // The keyboard stores one playback speed (it ignores per-frame times); it sends 0x66 - speed as a frame
  // interval. Previews assume ~10 ms per unit, which matches how slow the low settings feel on the hardware.
  const frameMs = () => Math.max(16, (0x66 - opt.speed) * 10);
  const joinFrames = list => { const out = new Uint8Array(list.length * N() * 3); list.forEach((f, i) => out.set(f, i * N() * 3)); return out; };

  /* --- build once per keyboard --- */
  function load() {
    const f = def.features || {};
    // Nothing of the previous keyboard's carries over: its files, tokens, drawing or a running upload.
    clearInterval(polling);
    polling = null;
    for (const bar of document.querySelectorAll("[data-progress]")) bar.classList.add("hidden");
    for (const st of document.querySelectorAll("[data-stamp]")) st.textContent = "";
    stamp("clockStamp", "");
    files = {};
    txtFrames = null;
    $("lcdMore").classList.add("hidden");
    if (!f.matrix) opt.both = $("lcdBoth").checked = false;
    // Only the displays this keyboard has are offered.
    for (const id of ["mxCard", "bootCard", "lcdBothRow"]) $(id).classList.toggle("hidden", !f.matrix);
    $("lcdCard").classList.toggle("hidden", !f.lcd);
    $("clockSync").classList.toggle("hidden", !f.clock);
    $("noDisplay").classList.toggle("hidden", !!(f.matrix || f.lcd));
    $("mxSize").textContent = f.matrix ? `${W()} × ${H()} LEDs` : "";
    $("lcdSize").textContent = f.lcd ? `${f.lcd.width} × ${f.lcd.height}` : "";
    if (f.matrix) grid.style.aspectRatio = `${W()} / ${H()}`;
    makeBoard($("kbDisplay"));
    frames = f.matrix ? [new Uint8Array(N() * 3)] : [];
    cur = 0;
    token = { matrix: null, lcd: null };
    segment($("mxTabs"), [["styles", "Styles"], ["image", "Image or GIF"], ["draw", "Draw"], ["text", "Text"]], "tab", tabChanged);
    stylesBuilt = false;
    chosenStyle = null;
    $("stylesOurs").innerHTML = $("stylesLib").innerHTML = "";
    pruneViews();  // their previews stop with them
    segment($("mxFit"), [["cover", "Fill"], ["contain", "Fit inside"]], "fit", () => files.matrix && prepare("matrix", files.matrix));
    $("mxSpeed").value = opt.speed;
    segment($("mxBright"), [[10, "10%"], [25, "25%"], [50, "50%"], [100, "Max"]], "bright");
    segment($("lcdFit"), [["cover", "Fill"], ["contain", "Fit inside"]], "lcdFit", () => files.lcd && prepare("lcd", files.lcd));
    segment($("txtStyle"), [["scroll", "Scroll"], ["static", "Still"]], "txtStyle", makeText);
    swatchRow($("pixSwatches"), "paint");
    swatchRow($("txtSwatches"), "txtColor", makeText, true);
    $("bootRestore").classList.toggle("hidden", !def.originals?.boot);
    loadBoot();
    $("lcdRestore").classList.toggle("hidden", !def.originals?.screen);
    render();
  }

  function tabChanged() {
    if (opt.tab === "draw") { drawGrid(); drawThumbs(); previewDraw(); }
    if (opt.tab === "text") makeText();
    if (opt.tab === "styles") { buildStyles(); if (chosenStyle) showStyle(chosenStyle); }
    if (opt.tab === "image" && token.matrix && files.matrix) prepare("matrix", files.matrix);
  }

  /* --- image / GIF --- */
  for (const target of ["matrix", "lcd"]) {
    // Clearing the input lets the same file be chosen again.
    document.querySelector(`[data-file="${target}"]`).onchange = e => { if (e.target.files[0]) prepare(target, e.target.files[0]); e.target.value = ""; };
    const drop = document.querySelector(`[data-drop="${target}"]`);
    drop.ondragover = e => { e.preventDefault(); drop.classList.add("over"); };
    drop.ondragleave = () => drop.classList.remove("over");
    drop.ondrop = e => { e.preventDefault(); drop.classList.remove("over"); if (e.dataTransfer.files[0]) prepare(target, e.dataTransfer.files[0]); };
    document.querySelector(`[data-send="${target}"]`).onclick = () => send(target);
  }
  // A file dropped beside a drop zone would otherwise replace the app with that file.
  window.addEventListener("dragover", e => e.preventDefault());
  window.addEventListener("drop", e => e.preventDefault());

  async function prepare(target, file, append = false) {
    const id = deviceId;
    files[target] = file;
    const st = document.querySelector(`[data-stamp="${target}"]`);
    st.className = "stamp"; st.textContent = "Preparing…";
    const fit = target === "matrix" ? opt.fit : opt.lcdFit;
    const more = append && token.lcd ? `&append=${encodeURIComponent(token.lcd)}` : "";
    const r = await api(`${devPath(id)}/media?target=${target}&fit=${fit}${more}`, file);
    if (id !== deviceId) return;  // the keyboard was switched while the file was being prepared
    if (!r.ok) { st.className = "stamp err"; st.textContent = "Can't use this file"; toast(r.error); return; }
    token[target] = r.token;
    if (target === "matrix") { setScreen("matrix", { pixels: b64ToBytes(r.pixels), delays: r.delays.map(frameMs), label: file.name, userSet: true }); }
    else {
      setScreen("lcd", { src: safeImage(r.preview), label: file.name });
      if (opt.both) { opt.tab = "image"; prepare("matrix", file); }  // same file on the matrix too
    }
    st.textContent = `${r.frames} frame${r.frames > 1 ? "s" : ""} · ${(r.duration_ms / 1000).toFixed(1)} s`;
    if (target === "lcd") {
      $("lcdMore").classList.remove("hidden");
      $("lcdParts").textContent = r.parts > 1 ? `${r.parts} in the playlist, played one after another` : "";
    }
    render();
  }

  /* --- pixel painting --- */
  const grid = $("pixGrid");
  function cellAt(e) {
    const r = grid.getBoundingClientRect();
    const x = Math.floor((e.clientX - r.left) / r.width * W()), y = Math.floor((e.clientY - r.top) / r.height * H());
    return x >= 0 && y >= 0 && x < W() && y < H() ? y * W() + x : -1;
  }
  function paintAt(e) {
    const i = cellAt(e);
    if (i < 0) return;
    const erase = $("pixErase").checked || e.buttons === 2;
    const [r, g, b] = erase ? [0, 0, 0] : rgb(opt.paint);
    frames[cur].set([r, g, b], i * 3);
    drawGrid();
    previewDraw();
  }
  grid.onpointerdown = e => { painting = true; grid.setPointerCapture(e.pointerId); paintAt(e); };
  grid.onpointermove = e => painting && paintAt(e);
  grid.onpointerup = grid.onpointercancel = () => { painting = false; drawThumbs(); };
  grid.oncontextmenu = e => e.preventDefault();

  function drawGrid() {
    if (!def.features?.matrix) return;
    const r = grid.getBoundingClientRect();
    if (!r.width) return;
    const dpr = window.devicePixelRatio || 1;
    grid.width = Math.round(r.width * dpr);
    grid.height = Math.round(r.width * dpr * H() / W());
    const ctx = grid.getContext("2d"), cell = grid.width / W(), f = frames[cur];
    ctx.fillStyle = "#000";
    ctx.fillRect(0, 0, grid.width, grid.height);
    for (let i = 0; i < N(); i++) {
      const x = i % W(), y = Math.floor(i / W());
      const c = f.subarray(i * 3, i * 3 + 3);
      const lit = c[0] + c[1] + c[2] > 0;
      ctx.fillStyle = lit ? `rgb(${c[0]},${c[1]},${c[2]})` : "#1a1a1a";
      ctx.beginPath();
      ctx.roundRect(x * cell + cell * 0.08, y * cell + cell * 0.08, cell * 0.84, cell * 0.84, cell * 0.22);
      ctx.fill();
    }
  }
  function drawThumbs() {
    keepFocus($("pixFrames"), buildThumbs);
  }
  function buildThumbs() {
    const box = $("pixFrames");
    box.innerHTML = "";
    frames.forEach((f, n) => {
      const c = document.createElement("canvas");
      c.width = W(); c.height = H();
      c.style.width = W() * 2 + "px"; c.style.height = H() * 2 + "px";
      const ctx = c.getContext("2d"), img = ctx.createImageData(W(), H());
      for (let i = 0; i < N(); i++) img.data.set([f[i * 3], f[i * 3 + 1], f[i * 3 + 2], 255], i * 4);
      ctx.putImageData(img, 0, 0);
      const b = document.createElement("button");
      b.className = "thumb" + (n === cur ? " active" : "");
      b.title = `Frame ${n + 1}`;
      b.setAttribute("aria-label", `Frame ${n + 1}`);
      b.appendChild(c);
      b.onclick = () => { cur = n; drawGrid(); drawThumbs(); };
      box.appendChild(b);
    });
    $("pixDel").disabled = frames.length < 2;
  }
  function previewDraw() {
    setScreen("matrix", { pixels: joinFrames(frames), delays: frames.map(frameMs), label: "Your drawing", userSet: true });
  }
  $("pixFill").onclick = () => { const c = rgb(opt.paint); for (let i = 0; i < N(); i++) frames[cur].set(c, i * 3); drawGrid(); drawThumbs(); previewDraw(); };
  $("pixClear").onclick = () => { frames[cur].fill(0); drawGrid(); drawThumbs(); previewDraw(); };
  $("pixAdd").onclick = () => { if (frames.length >= maxFrames()) return; frames.splice(cur + 1, 0, new Uint8Array(N() * 3)); cur++; drawGrid(); drawThumbs(); previewDraw(); };
  $("pixDup").onclick = () => { if (frames.length >= maxFrames()) return; frames.splice(cur + 1, 0, frames[cur].slice()); cur++; drawGrid(); drawThumbs(); previewDraw(); };
  $("pixDel").onclick = () => { if (frames.length < 2) return; frames.splice(cur, 1); cur = Math.max(0, cur - 1); drawGrid(); drawThumbs(); previewDraw(); };
  window.addEventListener("resize", () => opt.tab === "draw" && drawGrid());

  /* --- scrolling text --- */
  function textColumns(text) {
    const cols = [];
    for (const ch of text.toUpperCase()) {
      const g = FONT[ch] || FONT["?"];
      for (let x = 0; x < g[0].length; x++) cols.push(g.map(row => row[x] === "#"));
      cols.push([false, false, false, false, false]);
    }
    return cols;
  }
  function makeText() {
    if (!def.features?.matrix) return;
    const cols = textColumns($("txtInput").value || " ");
    const w = W(), h = H(), list = [];
    const color = i => opt.txtColor === "rainbow"
      ? hsl((i * 9) % 360, 1, 0.55).map(Math.round) : rgb(opt.txtColor);
    const frameAt = offset => {
      const f = new Uint8Array(N() * 3);
      for (let x = 0; x < w; x++) {
        const col = cols[x - offset];
        if (!col) continue;
        for (let y = 0; y < Math.min(h, 5); y++) if (col[y]) f.set(color(x - offset), (y * w + x) * 3);
      }
      return f;
    };
    if (opt.txtStyle === "static" && cols.length <= w) list.push(frameAt(Math.floor((w - cols.length) / 2)));
    else {
      const span = w + cols.length, step = Math.max(1, Math.ceil(span / maxFrames()));
      for (let o = w; o > -cols.length; o -= step) list.push(frameAt(o));
    }
    txtFrames = list;
    setScreen("matrix", { pixels: joinFrames(list), delays: list.map(frameMs), label: `“${$("txtInput").value}”`, userSet: true });
    render();
  }
  $("txtInput").oninput = makeText;

  /* --- sending --- */
  async function send(target) {
    if (!gateOpen()) return;
    const id = deviceId;
    let tk = token[target];
    if (target === "matrix" && opt.tab !== "image") {
      const list = opt.tab === "draw" ? frames : txtFrames;
      const px = opt.tab === "styles" ? chosenStyle.pixels : joinFrames(list);
      const count = px.length / (N() * 3);
      const r = await api(`${devPath(id)}/frames`, { pixels: toB64(px), delays: Array.from({ length: count }, frameMs) });
      if (!r.ok) { toast(r.error); return; }
      if (id !== deviceId) return;  // the keyboard was switched: this belongs to the previous one
      tk = r.token;
    }
    const body = target === "matrix"
      ? { token: tk, speed: opt.speed, brightness: opt.bright }
      : { token: tk, slot: opt.slot, ...(opt.both && token.matrix
          ? { also: token.matrix, speed: opt.speed, brightness: opt.bright } : {}) };
    const r = await api(`${devPath(id)}/display`, body);
    if (!r.ok) { toast(r.error); return; }
    if (id === deviceId) watch(target);
  }
  async function restoreScreen() {
    if (!gateOpen()) return;
    const id = deviceId;
    const r = await api(`${devPath(id)}/display`, { restore: "screen" });
    if (!r.ok) { toast(r.error); return; }
    if (id !== deviceId) return;
    const p = await api(`${devPath(id)}/originals/screen`);
    if (p.ok && id === deviceId) setScreen("lcd", { src: safeImage(p.preview), label: "Original screen" });
    if (id === deviceId) watch("lcd");
  }

  /* --- boot animation: its own card, separate from what the matrix shows day to day --- */
  const bootSrc = { pixels: null, delays: [] };
  let bootView = null;
  const bootKey = () => `halcyon.boot.${deviceId}`;
  function setBootPreview(pixels, label) {
    bootSrc.pixels = pixels;
    bootSrc.delays = Array.from({ length: pixels.length / (N() * 3) }, frameMs);
    $("bootLabel").textContent = label;
    if (bootView) { bootView.frame = 0; bootView.next = 0; }
  }
  async function loadBoot() {
    const id = deviceId;
    bootSrc.pixels = null;  // until this keyboard's own is known
    $("bootLabel").textContent = "unknown";
    if (bootView) matrixViews.delete(bootView);  // the matrix may have another size now, or none
    bootView = null;
    if (!def.features?.matrix) return;
    bootView = new MatrixView($("bootCanvas"), W(), H(), bootSrc);
    try {
      const saved = JSON.parse(localStorage.getItem(bootKey()));
      if (saved?.pixels) return setBootPreview(b64ToBytes(saved.pixels), String(saved.label || "your design"));
    } catch (e) {}
    if (def.originals?.boot) {
      const r = await api(`${devPath(id)}/originals/boot`);
      if (r.ok && id === deviceId) { try { setBootPreview(b64ToBytes(r.pixels), "the original"); } catch (e) {} }
    }
  }
  $("bootSet").onclick = async () => {
    if (!gateOpen() || !screens.matrix.pixels) return;
    const id = deviceId;
    const max = 200 * N() * 3;  // the keyboard keeps up to 200 boot frames
    const px = screens.matrix.pixels.length > max ? screens.matrix.pixels.slice(0, max) : screens.matrix.pixels;
    const r = await api(`${devPath(id)}/frames`, { pixels: toB64(px), delays: Array.from({ length: px.length / (N() * 3) }, frameMs) });
    if (!r.ok) { toast(r.error); return; }
    if (id !== deviceId) return;
    const d = await api(`${devPath(id)}/display`, { token: r.token, boot: true, speed: opt.speed, brightness: opt.bright });
    if (!d.ok) { toast(d.error); return; }
    if (id !== deviceId) return;
    const label = screens.matrix.label || "your design";
    setBootPreview(px, label);
    try { localStorage.setItem(bootKey(), JSON.stringify({ pixels: toB64(px), label })); } catch (e) {}
    watch("boot");
  };
  $("bootRestore").onclick = async () => {
    if (!gateOpen()) return;
    const id = deviceId;
    const r = await api(`${devPath(id)}/display`, { restore: "boot" });
    if (!r.ok) { toast(r.error); return; }
    if (id !== deviceId) return;
    try { localStorage.removeItem(bootKey()); } catch (e) {}
    loadBoot();
    watch("boot");
  };
  $("lcdRestore").onclick = restoreScreen;
  $("lcdAdd").onchange = e => { if (e.target.files[0]) prepare("lcd", e.target.files[0], true); e.target.value = ""; };

  function watch(target) {
    const bar = document.querySelector(`[data-progress="${target}"]`);
    const st = document.querySelector(`[data-stamp="${target}"]`);
    // One upload runs at a time, so only its own bar shows.
    for (const b of document.querySelectorAll("[data-progress]")) b.classList.toggle("hidden", b !== bar);
    clearInterval(polling);
    const id = deviceId;
    const timer = polling = setInterval(async () => {
      const j = await api(`${devPath(id)}/display/job`);
      if (timer !== polling) return;  // a newer upload, or another keyboard, has taken over
      const pct = Math.round((j.progress || 0) * 100);
      bar.firstElementChild.style.width = pct + "%";
      st.className = "stamp";
      if (j.state === "running") st.textContent = `Sending… ${pct}%`;
      if (j.state === "done") { clearInterval(timer); st.textContent = "On the keyboard " + now(); setTimeout(() => timer === polling && bar.classList.add("hidden"), 1500); }
      if (j.state === "error" || j.ok === false) { clearInterval(timer); st.className = "stamp err"; st.textContent = "Failed"; bar.classList.add("hidden"); toast(j.error); }
    }, 400);
  }

  $("clockSync").onclick = async () => {
    if (!gateOpen()) return;
    const id = deviceId;
    stamp("clockStamp", "Syncing…");
    const r = await api(`${devPath(id)}/clock`, { slot: opt.slot });
    if (id !== deviceId) return;
    if (r.ok) stamp("clockStamp", "Clock synced " + now()); else { stamp("clockStamp", "Not synced", true); toast(r.error); }
  };
  $("lcdBoth").onchange = () => {
    opt.both = $("lcdBoth").checked;
    if (opt.both && files.lcd) { opt.tab = "image"; prepare("matrix", files.lcd); }
    render();
  };
  $("mxSpeed").oninput = () => {
    opt.speed = +$("mxSpeed").value;
    if (screens.matrix.userSet) screens.matrix.delays = screens.matrix.delays.map(frameMs);
    render();
  };

  /* --- render --- */
  const cableOk = () => demoMode || lastStatus.connection === "wired";
  function render() {
    if (!def?.features?.matrix && !def?.features?.lcd) return;
    for (const [id, key] of [["mxTabs", "tab"], ["mxFit", "fit"], ["mxBright", "bright"],
                             ["lcdFit", "lcdFit"], ["txtStyle", "txtStyle"]]) {
      for (const b of $(id).children) { b.classList.toggle("active", String(opt[key]) === b.dataset.v); b.setAttribute("aria-pressed", String(opt[key]) === b.dataset.v); }
    }
    for (const [id, key] of [["pixSwatches", "paint"], ["txtSwatches", "txtColor"]])
      for (const b of $(id).children) { b.classList.toggle("active", b.dataset.c === opt[key]); b.setAttribute("aria-pressed", b.dataset.c === opt[key]); }
    for (const body of document.querySelectorAll(".mx-body")) body.classList.toggle("hidden", body.dataset.tab !== opt.tab);
    $("mxSpeedOut").textContent = opt.speed;
    const matrixReady = opt.tab === "image" ? !!token.matrix : opt.tab === "draw" ? true : opt.tab === "styles" ? !!chosenStyle : !!txtFrames;
    const mb = document.querySelector('[data-send="matrix"]'), lb = document.querySelector('[data-send="lcd"]');
    mb.disabled = !matrixReady || !cableOk();
    lb.disabled = !token.lcd || (opt.both && !token.matrix) || !cableOk();
    lb.textContent = opt.both ? "Send to both" : "Send to keyboard";
    $("bootRestore").disabled = $("lcdRestore").disabled = !cableOk();
    $("bootSet").disabled = !screens.matrix.userSet || !cableOk();
    $("bootSet").title = screens.matrix.userSet ? "" : "Pick a style, image, drawing or text for the matrix first";
    $("cableNotice").classList.toggle("hidden", cableOk());
    const parts = [screens.matrix.label && `Matrix: ${screens.matrix.label}`, screens.lcd.label && `Screen: ${screens.lcd.label}`].filter(Boolean);
    $("dispNow").textContent = parts.length ? "· " + parts.join(" · ") : "· what the displays will show";
  }
  function show() {
    render();
    if (!def.features?.matrix) return;
    if (opt.tab === "draw") { drawGrid(); drawThumbs(); }
    if (opt.tab === "text" && !txtFrames) makeText();
    if (opt.tab === "styles") buildStyles();
  }

  /* --- built-in styles --- */
  function styleCard(box, name, getStyle) {
    const w = W(), h = H();
    const b = document.createElement("button");
    b.className = "style";
    b.innerHTML = `<canvas aria-hidden="true"></canvas><span>${escapeHtml(name)}</span>`;
    box.appendChild(b);
    let style = null;
    const ready = getStyle().catch(() => null).then(st => {
      if (!st) { b.remove(); return; }
      if (!b.isConnected) return;  // the gallery was rebuilt while this was loading
      style = { ...st, name };
      new MatrixView(b.querySelector("canvas"), w, h, { pixels: st.pixels, delays: Array(st.frames).fill(st.delay || 60) });
    });
    b.onclick = async () => {
      await ready;
      if (!style) return;
      chosenStyle = style;
      // Each style comes with a speed and brightness that suit it; the sliders stay free to change.
      opt.speed = Math.max(1, Math.min(100, 102 - Math.round((style.delay || 60) / 10)));
      if (style.bright) opt.bright = style.bright;
      $("mxSpeed").value = opt.speed;
      for (const x of document.querySelectorAll(".style")) x.classList.toggle("active", x === b);
      showStyle(style);
    };
  }
  function showStyle(st) {
    setScreen("matrix", { pixels: st.pixels, delays: Array.from({ length: st.frames }, frameMs), label: st.name, userSet: true });
    render();
  }
  async function buildStyles() {
    if (stylesBuilt || !def.features?.matrix) return;
    stylesBuilt = true;
    const id = deviceId;
    const ours = $("stylesOurs"), lib = $("stylesLib");
    ours.innerHTML = lib.innerHTML = "";
    pruneViews();
    for (const st of HALCYON_STYLES) styleCard(ours, st.name, async () => ({ ...st.make(W(), H()), bright: st.bright }));
    const r = await api(`${devPath(id)}/library`);
    if (id !== deviceId) return;  // these would be another keyboard's animations
    const items = r.ok && Array.isArray(r.items) ? r.items : [];
    $("stylesLibWrap").classList.toggle("hidden", !items.length);
    for (const it of items) {
      styleCard(lib, it.name, async () => {
        const d = await api(`${devPath(id)}/library/${encodeURIComponent(it.layer)}`);
        return d.ok ? { pixels: b64ToBytes(d.pixels), frames: d.frames, delay: 60 } : null;
      });
    }
  }
  // For the command palette: open the Styles tab and choose one of Halcyon's styles by name.
  async function pickStyle(name) {
    if (!def.features?.matrix) return;
    opt.tab = "styles";
    render();
    await buildStyles();
    [...$("stylesOurs").children].find(b => b.textContent === name)?.click();
  }
  return { load, show, status: render, pickStyle };
})();
