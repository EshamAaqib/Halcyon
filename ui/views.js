/* Keys, Macros, Display and Settings views. Shares globals with app.js ($, api, def, config, deviceId, ...). */

/* ---------- Key names ---------- */
const HID_NAMES = (() => {
  const n = {};
  for (let i = 0; i < 26; i++) n[0x04 + i] = String.fromCharCode(65 + i);
  for (let i = 1; i <= 9; i++) n[0x1d + i] = String(i);
  for (let i = 1; i <= 12; i++) n[0x39 + i] = "F" + i;
  for (let i = 13; i <= 24; i++) n[0x68 + i - 13] = "F" + i;
  for (let i = 1; i <= 9; i++) n[0x58 + i] = "Num " + i;
  Object.assign(n, {
    0x27: "0", 0x28: "Enter", 0x29: "Esc", 0x2a: "Backspace", 0x2b: "Tab", 0x2c: "Space", 0x2d: "-", 0x2e: "=",
    0x2f: "[", 0x30: "]", 0x31: "\\", 0x33: ";", 0x34: "'", 0x35: "`", 0x36: ",", 0x37: ".", 0x38: "/", 0x39: "Caps Lock",
    0x46: "Print Screen", 0x47: "Scroll Lock", 0x48: "Pause", 0x49: "Insert", 0x4a: "Home", 0x4b: "Page Up", 0x4c: "Delete",
    0x4d: "End", 0x4e: "Page Down", 0x4f: "Right", 0x50: "Left", 0x51: "Down", 0x52: "Up", 0x53: "Num Lock",
    0x54: "Num /", 0x55: "Num *", 0x56: "Num -", 0x57: "Num +", 0x58: "Num Enter", 0x62: "Num 0", 0x63: "Num .", 0x65: "Menu",
    0xe0: "Left Ctrl", 0xe1: "Left Shift", 0xe2: "Left Alt", 0xe3: "Left Win", 0xe4: "Right Ctrl", 0xe5: "Right Shift",
    0xe6: "Right Alt", 0xe7: "Right Win", 0xaf: "Fn",
  });
  return n;
})();
const KEY_GROUPS = [
  ["Letters", range(0x04, 0x1d)], ["Numbers", range(0x1e, 0x27)], ["Function keys", range(0x3a, 0x45).concat(range(0x68, 0x73))],
  ["Editing and navigation", [0x29, 0x28, 0x2a, 0x2b, 0x2c, 0x39, 0x49, 0x4c, 0x4a, 0x4d, 0x4b, 0x4e, 0x52, 0x51, 0x50, 0x4f, 0x46, 0x47, 0x48, 0x65]],
  ["Symbols", [0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38]],
  ["Number pad", [0x53, 0x54, 0x55, 0x56, 0x57, 0x58].concat(range(0x59, 0x63))],
  ["Modifiers", range(0xe0, 0xe7)],
];
function range(a, b) { const r = []; for (let i = a; i <= b; i++) r.push(i); return r; }
const CODE_TO_HID = (() => {
  const m = {};
  for (const [hid, code] of Object.entries(HID_TO_CODE)) m[code] = +hid;
  for (let i = 1; i <= 12; i++) m["F" + i] = 0x39 + i;
  for (let i = 0; i <= 9; i++) m["Numpad" + i] = i === 0 ? 0x62 : 0x58 + i;
  Object.assign(m, { PrintScreen: 0x46, ScrollLock: 0x47, Pause: 0x48, Insert: 0x49, End: 0x4d, NumLock: 0x53,
    NumpadDivide: 0x54, NumpadMultiply: 0x55, NumpadSubtract: 0x56, NumpadAdd: 0x57, NumpadEnter: 0x58,
    NumpadDecimal: 0x63, ContextMenu: 0x65, MetaRight: 0xe7 });
  return m;
})();
const MODS = [["Ctrl", 0x01], ["Shift", 0x02], ["Alt", 0x04], ["Win", 0x08]];
const MEDIA = [["Play / Pause", 0xcd], ["Next track", 0xb5], ["Previous track", 0xb6], ["Stop", 0xb7], ["Volume up", 0xe9],
  ["Volume down", 0xea], ["Mute", 0xe2], ["Calculator", 0x192], ["This PC", 0x194], ["Email", 0x18a], ["Media player", 0x183],
  ["Browser home", 0x223], ["Browser search", 0x221], ["Browser back", 0x224], ["Browser forward", 0x225], ["Browser refresh", 0x227]];
const SHORTCUTS = [["Show desktop", 0x08, 0x07], ["Lock PC", 0x08, 0x0f], ["File Explorer", 0x08, 0x08], ["Copy", 0x01, 0x06],
  ["Paste", 0x01, 0x19], ["Cut", 0x01, 0x1b], ["Undo", 0x01, 0x1d], ["Close window", 0x04, 0x3d], ["Screenshot", 0x0a, 0x16]];
const MOUSE = [["Left click", "mouse", 0x01], ["Right click", "mouse", 0x02], ["Middle click", "mouse", 0x04],
  ["Back button", "mouse", 0x08], ["Forward button", "mouse", 0x10], ["Scroll up", "scroll", 1], ["Scroll down", "scroll", -1]];
const MACRO_MODES = [[0, "Play once"], [1, "Repeat"], [2, "Until pressed again"]];

function modsLabel(mask) { return MODS.filter(([, b]) => mask & b).map(([n]) => n); }
function describe(a) {
  if (!a || a.type === "default") return "Default";
  switch (a.type) {
    case "disabled": return "Disabled";
    case "key": {
      const sc = SHORTCUTS.find(([, m, h]) => m === (a.mods || 0) && h === a.hid);
      if (sc) return sc[0];
      return [...modsLabel(a.mods || 0), HID_NAMES[a.hid] || `Key ${a.hid}`].join(" + ");
    }
    case "consumer": return (MEDIA.find(([, u]) => u === a.usage) || [`Media ${a.usage}`])[0];
    case "mouse": return (MOUSE.find(([, t, v]) => t === "mouse" && v === a.buttons) || ["Mouse"])[0];
    case "scroll": return a.delta > 0 ? "Scroll up" : "Scroll down";
    case "macro": {
      const m = config.macros.find(x => x.id === a.macro);
      const mode = a.mode === 1 ? ` ×${a.count}` : a.mode === 2 ? " (toggle)" : "";
      return m ? `Macro: ${m.name}${mode}` : "Macro (deleted)";
    }
  }
  return a.type;
}
function shortLabel(a) {
  const d = describe(a);
  return d.length > 9 ? d.replace("Macro: ", "M: ").slice(0, 9) + "…" : d;
}
const sameAction = (a, b) => JSON.stringify(a || null) === JSON.stringify(b || null);
function stamp(id, text, err) { const s = $(id); s.textContent = text; s.className = "stamp" + (err ? " err" : ""); }
const now = () => new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

/* ---------- Keys ---------- */
VIEWS.keys = (() => {
  let board = [], draft = {}, selected = null, tab = "key", resetArmed = null, capture = null, held = false;
  const dirty = () => !sameAction(draft, config.keymap);

  function load() {
    draft = JSON.parse(JSON.stringify(config.keymap || {}));
    selected = null;
    stopCapture();
    disarmReset();
    stamp("keysStamp", "");
    board = makeBoard($("kbKeys"));
    for (const key of board) {
      const sub = document.createElement("small");
      key.el.querySelector(".cap").appendChild(sub);
      key.el.tabIndex = 0;
      key.el.setAttribute("role", "button");
      key.el.onclick = () => select(key.k.key_index);
      key.el.onkeydown = e => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); select(key.k.key_index); } };
    }
    renderTabs();
    render();
  }

  // "Press a key" takes the next key press as the choice. Escape, or anything that redraws the editor, cancels it.
  function stopCapture() { capture?.(); capture = null; }

  function select(idx) {
    selected = idx;
    const a = draft[idx];
    tab = !a ? "key" : a.type === "consumer" ? "media" : (a.type === "mouse" || a.type === "scroll") ? "mouse" :
      a.type === "macro" ? "macro" : a.type === "disabled" ? "disable" : "key";
    render();
  }

  function setAction(a) {
    const k = def.layout.keys.find(k => k.key_index === selected);
    if (!a || a.type === "default" || (a.type === "key" && !a.mods && a.hid === k.code)) delete draft[selected];
    else draft[selected] = a;
    render();
  }

  function renderTabs() {
    const box = $("keyTabs");
    box.innerHTML = "";
    for (const [id, label] of [["key", "Key"], ["media", "Media"], ["mouse", "Mouse"], ["macro", "Macro"], ["disable", "Off"]]) {
      const b = document.createElement("button");
      b.textContent = label;
      b.dataset.tab = id;
      b.onclick = () => { tab = id; render(); };
      box.appendChild(b);
    }
  }

  function option(label, active, onclick, extra = "") {
    const b = document.createElement("button");
    b.className = "opt" + (active ? " active" : "");
    b.setAttribute("aria-pressed", !!active);
    b.innerHTML = `<span>${escapeHtml(label)}</span>${extra}`;
    b.onclick = onclick;
    return b;
  }

  function renderBody() {
    const body = $("keyTabBody");
    stopCapture();
    body.innerHTML = "";
    const a = draft[selected];
    if (tab === "key") {
      const cur = a && a.type === "key" ? a : null;
      const mods = cur ? cur.mods || 0 : 0;
      const chips = document.createElement("div");
      chips.className = "chips";
      for (const [name, bit] of MODS) {
        const c = option(name, mods & bit, () => {
          const base = cur || { type: "key", hid: def.layout.keys.find(k => k.key_index === selected).code, mods: 0 };
          setAction({ ...base, mods: (base.mods || 0) ^ bit });
        });
        chips.appendChild(c);
      }
      body.insertAdjacentHTML("beforeend", `<div class="lbl">Hold with</div>`);
      body.appendChild(chips);
      body.insertAdjacentHTML("beforeend", `<div class="lbl">Key</div>`);
      const row = document.createElement("div");
      row.className = "pick-row";
      const sel = document.createElement("select");
      sel.className = "input";
      sel.setAttribute("aria-label", "Key");
      sel.innerHTML = KEY_GROUPS.map(([g, list]) => `<optgroup label="${escapeHtml(g)}">${list.map(h => `<option value="${h}">${escapeHtml(HID_NAMES[h])}</option>`).join("")}</optgroup>`).join("");
      sel.value = cur ? cur.hid : def.layout.keys.find(k => k.key_index === selected).code;
      sel.onchange = () => setAction({ type: "key", hid: +sel.value, mods });
      const cap = document.createElement("button");
      cap.className = "btn";
      cap.textContent = "Press a key";
      cap.onclick = () => {
        if (capture) { stopCapture(); return; }
        cap.textContent = "Waiting… (Esc cancels)";
        const h = e => {
          e.preventDefault();
          e.stopPropagation();
          stopCapture();
          // Until the key comes back up, the rest of a chord (the K of Ctrl+K) is not a shortcut either.
          held = true;
          window.addEventListener("keyup", () => { held = false; }, { capture: true, once: true });
          if (e.key === "Escape") return;
          const hid = CODE_TO_HID[e.code];
          if (hid) setAction({ type: "key", hid, mods });
          else toast("That key can't be assigned");
        };
        window.addEventListener("keydown", h, true);
        capture = () => { window.removeEventListener("keydown", h, true); cap.textContent = "Press a key"; };
      };
      row.append(sel, cap);
      body.appendChild(row);
      body.insertAdjacentHTML("beforeend", `<div class="lbl">Shortcuts</div>`);
      const list = document.createElement("div");
      list.className = "opts";
      for (const [name, m, h] of SHORTCUTS) list.appendChild(option(name, cur && cur.mods === m && cur.hid === h, () => setAction({ type: "key", hid: h, mods: m })));
      body.appendChild(list);
    } else if (tab === "media") {
      const list = document.createElement("div");
      list.className = "opts";
      for (const [name, u] of MEDIA) list.appendChild(option(name, a && a.type === "consumer" && a.usage === u, () => setAction({ type: "consumer", usage: u })));
      body.appendChild(list);
    } else if (tab === "mouse") {
      const list = document.createElement("div");
      list.className = "opts";
      for (const [name, t, v] of MOUSE) {
        const act = t === "mouse" ? { type: "mouse", buttons: v } : { type: "scroll", delta: v };
        list.appendChild(option(name, sameAction(a, act), () => setAction(act)));
      }
      body.appendChild(list);
    } else if (tab === "macro") {
      if (!config.macros.length) {
        body.innerHTML = `<p class="hint">No macros yet. Make one in <a href="#macros">Macros</a> first.</p>`;
        return;
      }
      const cur = a && a.type === "macro" ? a : null;
      const list = document.createElement("div");
      list.className = "opts";
      for (const m of config.macros) {
        list.appendChild(option(m.name, cur && cur.macro === m.id, () => setAction({ type: "macro", macro: m.id, mode: cur ? cur.mode : 0, count: cur ? cur.count : 1 }),
          `<small>${m.events.length} steps</small>`));
      }
      body.appendChild(list);
      if (cur) {
        body.insertAdjacentHTML("beforeend", `<div class="lbl">Play</div>`);
        const seg = document.createElement("div");
        seg.className = "seg";
        for (const [v, label] of MACRO_MODES) {
          const b = document.createElement("button");
          b.textContent = label;
          b.className = cur.mode === v ? "active" : "";
          b.setAttribute("aria-pressed", cur.mode === v);
          b.onclick = () => setAction({ ...cur, mode: v });
          seg.appendChild(b);
        }
        body.appendChild(seg);
        if (cur.mode === 1) {
          body.insertAdjacentHTML("beforeend", `<div class="lbl">Times</div>`);
          const n = document.createElement("input");
          n.type = "number"; n.min = 1; n.max = 255; n.value = cur.count || 1; n.className = "input"; n.style.width = "100px";
          n.setAttribute("aria-label", "Times");
          n.onchange = () => setAction({ ...cur, count: Math.max(1, Math.min(255, +n.value || 1)) });
          body.appendChild(n);
        }
      }
    } else if (tab === "disable") {
      body.insertAdjacentHTML("beforeend", `<p class="hint" style="margin-top:0">The key does nothing when pressed.</p>`);
      const list = document.createElement("div");
      list.className = "opts";
      list.appendChild(option("Disable this key", a && a.type === "disabled", () => setAction({ type: "disabled" })));
      body.appendChild(list);
    }
    const reset = document.createElement("button");
    reset.className = "btn";
    reset.style.marginTop = "18px";
    reset.textContent = "Back to default";
    reset.disabled = !a;
    reset.onclick = () => setAction(null);
    body.appendChild(reset);
  }

  function render() {
    for (const key of board) {
      const idx = key.k.key_index;
      const a = draft[idx];
      key.el.classList.toggle("changed", !!a);
      key.el.classList.toggle("selected", idx === selected);
      key.el.setAttribute("aria-pressed", idx === selected);
      key.el.classList.toggle("pending", !sameAction(a, config.keymap[idx]));
      key.el.querySelector("small").textContent = a ? shortLabel(a) : "";
    }
    for (const b of $("keyTabs").children) { b.classList.toggle("active", b.dataset.tab === tab); b.setAttribute("aria-pressed", b.dataset.tab === tab); }
    const k = def.layout.keys.find(k => k.key_index === selected);
    $("keyEditor").classList.toggle("hidden", !k);
    $("keyHint").classList.toggle("hidden", !!k);
    $("keyTitle").textContent = k ? `Key: ${HID_NAMES[k.code] || k.label}` : "No key selected";
    $("keyNow").textContent = k ? (draft[selected] ? describe(draft[selected]) : "Default") : "";
    if (k) keepFocus($("keyTabBody"), renderBody);
    else stopCapture();
    const entries = Object.entries(draft).sort((x, y) => +x[0] - +y[0]);
    $("keysCount").textContent = entries.length ? `${entries.length} changed` : "";
    keepFocus($("keysList"), () => {
      $("keysList").innerHTML = entries.length ? entries.map(([idx, a]) => {
        const kk = def.layout.keys.find(k => k.key_index === +idx);
        return `<li><b>${escapeHtml(kk ? HID_NAMES[kk.code] || kk.label : idx)}</b><span style="flex:1">${escapeHtml(describe(a))}</span>
          <button class="link" data-undo="${escapeHtml(idx)}">Reset</button></li>`;
      }).join("") : `<li><span>Every key does what's printed on it.</span></li>`;
    });
    for (const b of $("keysList").querySelectorAll("[data-undo]")) b.onclick = () => { delete draft[b.dataset.undo]; render(); };
    $("keysSave").disabled = !dirty();
    if (dirty()) stamp("keysStamp", "Not saved yet");
    else if ($("keysStamp").textContent === "Not saved yet") stamp("keysStamp", "");
    $("keysSummary").textContent = k ? `${HID_NAMES[k.code] || k.label}: ${describe(draft[selected])}` : "Click a key to change what it does";
  }

  async function save(map) {
    const id = deviceId;
    stamp("keysStamp", "Saving…");
    const r = await api(`${devPath(id)}/keymap`, { keymap: map });
    if (id !== deviceId) return;  // that answer is about the keyboard used before
    if (r.ok) {
      config.keymap = JSON.parse(JSON.stringify(map));
      draft = JSON.parse(JSON.stringify(map));
      stamp("keysStamp", "Saved " + now());
      render();
      for (const key of board) {  // saved keys flash once on the drawing
        if (map[key.k.key_index]) { key.el.classList.remove("flash"); void key.el.offsetWidth; key.el.classList.add("flash"); }
      }
      return;
    }
    stamp("keysStamp", "Not saved", true);
    toast(r.error);
    render();
  }
  $("keysSave").onclick = () => save(draft);
  function disarmReset() { clearTimeout(resetArmed); resetArmed = null; $("keysReset").textContent = "Reset all keys"; }
  $("keysReset").onclick = () => {
    if (!resetArmed) {
      $("keysReset").textContent = "Click again to reset";
      resetArmed = setTimeout(disarmReset, 3000);
      return;
    }
    disarmReset();
    save({});
  };
  window.addEventListener("blur", () => { held = false; });  // after Alt+Tab the keyup never comes
  return { load, show: render, grabbing: () => !!capture || held, release: stopCapture };
})();

/* ---------- Macros ---------- */
VIEWS.macros = (() => {
  let draft = [], current = null, recording = false, last = 0;
  const down = new Set();  // keys pressed during this recording and not yet released
  const CAP = () => def.features?.macros?.capacity_events || 650;
  const used = () => draft.reduce((n, m) => n + m.events.length + 2, 0);

  function load() {
    stopRec();
    draft = JSON.parse(JSON.stringify(config.macros || []));
    current = draft[0]?.id || null;
    stamp("macroStamp", "");
    render();
  }
  const cur = () => draft.find(m => m.id === current);

  function eventLabel([kind, v]) {
    if (kind === "delay") return null;
    const arrow = kind.endsWith("down") ? "↓" : "↑";
    const name = kind.startsWith("mouse") ? (MOUSE.find(([, t, b]) => t === "mouse" && b === v) || ["Mouse"])[0] : HID_NAMES[v] || `Key ${v}`;
    return `<span class="arrow">${arrow}</span>${escapeHtml(name)}`;
  }

  function render() {
    const u = used();
    $("macroCap").textContent = `${u} of ${CAP()} steps used`;
    $("macroMeter").style.width = Math.min(100, (u / CAP()) * 100) + "%";
    $("macroMeter").classList.toggle("over", u > CAP());
    keepFocus($("macroList"), () => {
      $("macroList").innerHTML = draft.length ? draft.map(m => {
        const keysUsing = Object.values(config.keymap).filter(a => a?.type === "macro" && a.macro === m.id).length;
        return `<button class="mrow ${m.id === current ? "active" : ""}" data-id="${escapeHtml(m.id)}"><b>${escapeHtml(m.name)}</b>
          <span>${m.events.filter(e => e[0] !== "delay").length} actions · ${keysUsing ? `on ${keysUsing} key${keysUsing > 1 ? "s" : ""}` : "not on a key"}</span></button>`;
      }).join("") : `<p class="hint">No macros yet.</p>`;
    });
    for (const b of $("macroList").querySelectorAll(".mrow")) b.onclick = () => { stopRec(); current = b.dataset.id; render(); };
    const m = cur();
    $("macroEdBody").classList.toggle("hidden", !m);
    $("macroEdHint").textContent = m ? "" : "Choose or create a macro";
    if (!m) return;
    if (document.activeElement !== $("macroName")) $("macroName").value = m.name;
    $("macroRec").textContent = recording ? "Stop recording" : (m.events.length ? "Record again" : "Record");
    $("macroRec").classList.toggle("rec", recording);
    $("macroRecState").textContent = recording ? "Type the sequence now · Esc stops" : "";
    const list = $("macroEvents");
    keepFocus(list, () => {
      list.innerHTML = m.events.length ? m.events.map((e, i) => e[0] === "delay"
        ? `<li class="delay"><span>Wait</span><input type="number" min="0" max="65535" value="${+e[1] || 0}" data-i="${i}" aria-label="Delay in ms"><span>ms</span><button class="x" data-del="${i}" aria-label="Remove">×</button></li>`
        : `<li>${eventLabel(e)}<button class="x" data-del="${i}" aria-label="Remove">×</button></li>`).join("")
        : `<li class="empty">Press Record, then type the keys you want this macro to send.</li>`;
    });
    for (const inp of list.querySelectorAll("input")) inp.onchange = () => { m.events[+inp.dataset.i][1] = Math.max(0, Math.min(65535, +inp.value || 0)); dirtyStamp(); };
    for (const b of list.querySelectorAll("[data-del]")) b.onclick = () => { m.events.splice(+b.dataset.del, 1); dirtyStamp(); render(); };
  }
  const dirtyStamp = () => stamp("macroStamp", sameAction(draft, config.macros) ? "" : "Not saved yet");

  function onKey(e) {
    if (!recording || e.target === $("macroName")) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.repeat) return;
    // Escape ends the recording (keyboard users have no other way out: every key is being recorded).
    if (e.key === "Escape" && e.type === "keydown") { stopRec(); render(); $("macroRec").focus(); return; }
    const hid = CODE_TO_HID[e.code];
    if (!hid) return;
    const m = cur();
    if (!m) return;
    // A key-up whose key went down before the recording started (the Enter that started it) is not recorded.
    if (e.type === "keyup" && !down.has(hid)) return;
    if (e.type === "keydown") down.add(hid); else down.delete(hid);
    const t = performance.now();
    if (m.events.length && $("macroDelays").checked) m.events.push(["delay", Math.min(65535, Math.round(t - last))]);
    last = t;
    m.events.push([e.type === "keydown" ? "down" : "up", hid]);
    dirtyStamp();
    render();
    $("macroEvents").scrollTop = 1e6;
  }
  window.addEventListener("keydown", onKey, true);
  window.addEventListener("keyup", onKey, true);
  function stopRec() {
    if (recording) {
      // Keys still held when the recording stops are released, or the keyboard would hold them down forever.
      const m = cur();
      if (m) for (const hid of down) m.events.push(["up", hid]);
    }
    down.clear();
    recording = false;
  }

  $("macroRec").onclick = () => {
    const m = cur();
    if (!m) return;
    if (recording) stopRec();
    else { m.events = []; down.clear(); last = performance.now(); recording = true; $("macroRec").blur(); }
    dirtyStamp();
    render();
  };
  $("macroName").oninput = () => { if (!cur()) return; cur().name = $("macroName").value || "Untitled"; dirtyStamp(); render(); };
  $("macroNew").onclick = () => {
    stopRec();
    const m = { id: "m" + Date.now().toString(36), name: `Macro ${draft.length + 1}`, events: [] };
    draft.push(m);
    current = m.id;
    dirtyStamp();
    render();
    $("macroName").focus();
    $("macroName").select();
  };
  $("macroAddDelay").onclick = () => { if (!cur()) return; cur().events.push(["delay", 50]); dirtyStamp(); render(); };
  $("macroDelete").onclick = () => {
    stopRec();
    draft = draft.filter(m => m.id !== current);
    current = draft[0]?.id || null;
    dirtyStamp();
    render();
  };
  $("macroSave").onclick = async () => {
    stopRec();
    if (used() > CAP()) { toast("These macros are too long for the keyboard's memory"); return; }
    stamp("macroStamp", "Saving…");
    const id = deviceId, sent = draft;
    const r = await api(`${devPath(id)}/macros`, { macros: sent });
    if (id !== deviceId) return;  // that answer is about the keyboard used before
    if (r.ok) { config.macros = JSON.parse(JSON.stringify(sent)); stamp("macroStamp", "Saved " + now()); VIEWS.keys.show(); }
    else { stamp("macroStamp", "Not saved", true); toast(r.error); }
    render();
  };
  return { load, show: render, grabbing: () => recording, release: () => { if (recording) { stopRec(); render(); } } };
})();

/* ---------- Settings ---------- */
VIEWS.settings = (() => {
  let s = {}, timer = null;
  const SLEEP = [[0, "Never"], [1, "1 min"], [2, "5 min"], [3, "30 min"]];
  const box = $("sleepSeg");
  for (const [v, label] of SLEEP) {
    const b = document.createElement("button");
    b.textContent = label;
    b.dataset.v = v;
    b.onclick = () => { s.sleep = v; change(); };
    box.appendChild(b);
  }
  for (const inp of document.querySelectorAll("[data-set]")) inp.onchange = () => { s[inp.dataset.set] = inp.checked; change(); };

  function load() {
    clearTimeout(timer);  // an unsent change belongs to the previous keyboard
    s = { sleep: 0, ...(config.settings || {}) };
    stamp("setStamp", "");
    render();
  }
  function render() {
    for (const inp of document.querySelectorAll("[data-set]")) {
      inp.checked = !!s[inp.dataset.set];
      const sub = inp.closest(".sub");
      if (sub) { inp.disabled = !s.game_mode; sub.classList.toggle("off", !s.game_mode); }
    }
    for (const b of box.children) { b.classList.toggle("active", +b.dataset.v === (s.sleep || 0)); b.setAttribute("aria-pressed", +b.dataset.v === (s.sleep || 0)); }
  }
  function change() {
    render();
    stamp("setStamp", "Saving…");
    clearTimeout(timer);
    const id = deviceId;
    timer = setTimeout(async () => {
      const sent = { ...s };
      const r = await api(`${devPath(id)}/settings`, { settings: sent });
      if (id !== deviceId) return;  // that answer is about the keyboard used before
      if (r.ok) { config.settings = sent; stamp("setStamp", "Saved " + now()); }
      else { stamp("setStamp", "Not saved", true); toast(r.error); }
    }, 300);
  }
  return { load, show: render };
})();
