/* App-wide polish: sliding nav indicator, the keyboard tilting toward the pointer, and a Ctrl+K command palette. */

/* ---------- Sliding nav indicator ---------- */
(() => {
  const side = document.querySelector(".sidebar");
  const ind = document.createElement("div");
  ind.className = "nav-ind";
  side.prepend(ind);
  window.moveNavIndicator = () => {
    const a = document.querySelector(".nav a.active");
    if (!a) return;
    ind.style.transform = `translate(${a.offsetLeft}px, ${a.offsetTop}px)`;
    ind.style.width = a.offsetWidth + "px";
    ind.style.height = a.offsetHeight + "px";
  };
  window.addEventListener("resize", window.moveNavIndicator);
  window.moveNavIndicator();
})();

/* ---------- The keyboard leans toward the pointer ---------- */
if (!matchMedia("(prefers-reduced-motion: reduce)").matches) {
  for (const stage of document.querySelectorAll(".stage")) {
    const board = stage.querySelector(".board");
    stage.addEventListener("pointermove", e => {
      const r = stage.getBoundingClientRect();
      const nx = (e.clientX - r.left) / r.width * 2 - 1, ny = (e.clientY - r.top) / r.height * 2 - 1;
      board.style.setProperty("--ry", (nx * 4).toFixed(2) + "deg");
      board.style.setProperty("--rx", (-ny * 3).toFixed(2) + "deg");
    });
    stage.addEventListener("pointerleave", () => {
      board.style.setProperty("--ry", "0deg");
      board.style.setProperty("--rx", "0deg");
    });
  }
}

/* ---------- Command palette ---------- */
(() => {
  const wrap = document.createElement("div");
  wrap.className = "pal hidden";
  wrap.innerHTML = `<div class="pal-box" role="dialog" aria-modal="true" aria-label="Quick actions">
    <input class="pal-input" placeholder="Type an effect, a page or an action" aria-label="Search actions" autocomplete="off">
    <ul class="pal-list" role="listbox"></ul></div>`;
  document.body.appendChild(wrap);
  const input = wrap.querySelector("input"), list = wrap.querySelector("ul");
  let items = [], shown = [], sel = 0;

  const open = document.createElement("button");
  open.className = "pal-open";
  open.innerHTML = `Quick actions<kbd>Ctrl K</kbd>`;
  document.querySelector(".sidebar").appendChild(open);

  function commands() {
    const go = (hash, label) => ({ label, hint: "Page", run: () => { location.hash = hash; } });
    const out = [go("lighting", "Lighting"), go("keys", "Keys"), go("macros", "Macros"), go("display", "Display"),
                 go("settings", "Settings"), go("devices", "Keyboards")];
    if (!def) return out;
    // Only what this keyboard can do is offered.
    for (const e of def.lighting.effects) {
      if (e.preview === "off") continue;
      out.push({ label: effectName(e), hint: "Lighting effect", run: () => { location.hash = "lighting"; update({ effect: e.id }); } });
    }
    if (def.lighting.effects.some(e => e.preview === "off")) {
      const isOff = effectById(state.effect)?.preview === "off";
      out.push({ label: isOff ? "Turn backlight on" : "Turn backlight off", hint: "Action", run: () => $("power").click() });
    }
    if (def.features?.clock) out.push({ label: "Sync clock", hint: "Action", run: () => $("clockSync").click() });
    if (def.features?.matrix) {
      for (const st of HALCYON_STYLES) out.push({ label: st.name, hint: "Matrix style", run: () => {
        location.hash = "display";
        VIEWS.display.pickStyle(st.name);
      } });
    }
    return out;
  }

  function render() {
    const q = input.value.toLowerCase().split(/\s+/).filter(Boolean);
    shown = items.filter(it => q.every(w => (it.label + " " + it.hint).toLowerCase().includes(w))).slice(0, 9);
    sel = Math.min(sel, Math.max(0, shown.length - 1));
    list.innerHTML = shown.length
      ? shown.map((it, i) => `<li class="${i === sel ? "active" : ""}" data-i="${i}" role="option" aria-selected="${i === sel}"><span>${escapeHtml(it.label)}</span><small>${escapeHtml(it.hint)}</small></li>`).join("")
      : `<li class="none">Nothing matches</li>`;
    for (const li of list.querySelectorAll("[data-i]")) {
      li.onmousemove = () => { if (sel !== +li.dataset.i) { sel = +li.dataset.i; render(); } };
      li.onclick = () => run(+li.dataset.i);
    }
  }
  let back = null;  // what had focus before the palette opened
  function show() {
    if (!$("gate").classList.contains("hidden")) return;
    releaseKeys();  // a recording or key capture would swallow what is typed here
    back = document.activeElement;
    items = commands(); input.value = ""; sel = 0; wrap.classList.remove("hidden"); render(); input.focus();
  }
  function hide() {
    wrap.classList.add("hidden");
    if (back?.isConnected) back.focus();
    back = null;
  }
  function run(i) { const it = shown[i]; hide(); it?.run(); }

  open.onclick = show;
  wrap.onclick = e => { if (e.target === wrap) hide(); };
  input.oninput = () => { sel = 0; render(); };
  input.onkeydown = e => {
    if (e.key === "ArrowDown") { sel = Math.min(shown.length - 1, sel + 1); render(); e.preventDefault(); }
    else if (e.key === "ArrowUp") { sel = Math.max(0, sel - 1); render(); e.preventDefault(); }
    else if (e.key === "Enter") run(sel);
    else if (e.key === "Escape") hide();
    else if (e.key === "Tab") e.preventDefault();  // focus stays in the dialog
  };
  window.addEventListener("keydown", e => {
    if (!(e.ctrlKey || e.metaKey) || e.key?.toLowerCase() !== "k") return;
    e.preventDefault();
    if (keysGrabbed()) return;  // the shortcut belongs to the macro being recorded or the key being captured
    if (!$("gate").classList.contains("hidden")) return;  // nothing opens over the first-run disclaimer
    wrap.classList.contains("hidden") ? show() : hide();
  });
})();
