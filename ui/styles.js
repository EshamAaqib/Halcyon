/* Matrix animations generated in code, so every keyboard with a matrix gets them.
   Every style is a seamless loop: its last frame flows into its first, because all motion is a whole number
   of cycles per loop (t runs 0..2π), or wraps around the loop length. Each make() returns
   { pixels: Uint8Array(frames * w * h * 3), frames, delay }. */

function rng(seed) {  // small deterministic PRNG so each style looks the same every time
  return () => { seed = (seed * 1664525 + 1013904223) >>> 0; return seed / 4294967296; };
}

// Build n frames; draw(f, put, t) paints one frame. put() keeps the brightest value per LED.
function buildLoop(n, w, h, delay, draw) {
  const px = new Uint8Array(n * w * h * 3);
  const cl = v => Math.max(0, Math.min(255, v)) | 0;
  for (let f = 0; f < n; f++) {
    const o = f * w * h * 3;
    draw(f, (x, y, r, g, b) => {
      x = Math.round(x); y = Math.round(y);
      if (x < 0 || y < 0 || x >= w || y >= h) return;
      const i = o + (y * w + x) * 3;
      px[i] = Math.max(px[i], cl(r)); px[i + 1] = Math.max(px[i + 1], cl(g)); px[i + 2] = Math.max(px[i + 2], cl(b));
    }, f / n * Math.PI * 2);
  }
  return { pixels: px, frames: n, delay };
}
const tri = u => 1 - Math.abs(2 * (u % 1) - 1);       // 0 → 1 → 0 over one cycle
const frac = u => ((u % 1) + 1) % 1;
const tint = (c, k) => [c[0] * k, c[1] * k, c[2] * k];

const HALCYON_STYLES = [
  { name: "Rainbow wave", make: (w, h) => buildLoop(90, w, h, 50, (f, put) => {
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) put(x, y, ...hsl((x * 5.7 + y * 8 + f * 4) % 360, 1, 0.5));
  }) },

  { name: "Plasma", make: (w, h) => buildLoop(100, w, h, 50, (f, put, t) => {
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
      const v = Math.sin(x / 6 + t) + Math.sin((y + x) / 5 - t) + Math.sin(Math.hypot(x - w / 2, (y - 2) * 4) / 4 + t * 2);
      put(x, y, ...hsl((v * 60 + f * 3.6 + 720) % 360, 1, 0.5));
    }
  }) },

  { name: "Fire", make(w, h) {
    // A simulation has no natural loop, so the first frames are cross-faded with the frames after the end.
    const n = 72, k = 18, r = rng(7), sim = [];
    const heat = Array.from({ length: h + 1 }, () => new Float32Array(w));
    for (let f = 0; f < n + k + 20; f++) {
      for (let x = 0; x < w; x++) heat[h][x] = 0.55 + r() * 0.45;
      for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
        const b = heat[y + 1];
        heat[y][x] = Math.max(0, (b[(x + w - 1) % w] + b[x] * 2 + b[(x + 1) % w]) / 4 - 0.11 - r() * 0.07);
      }
      if (f < 20) continue;
      const fr = new Uint8Array(w * h * 3);
      for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
        const v = heat[y][x];
        fr.set([Math.min(255, v * 420), Math.min(255, Math.max(0, v - 0.35) * 380), Math.min(255, Math.max(0, v - 0.8) * 600)], (y * w + x) * 3);
      }
      sim.push(fr);
    }
    const px = new Uint8Array(n * w * h * 3);
    for (let i = 0; i < n; i++) {
      const a = i < k ? i / k : 1;
      for (let j = 0; j < w * h * 3; j++) px[i * w * h * 3 + j] = sim[i][j] * a + (i < k ? sim[n + i][j] * (1 - a) : 0);
    }
    return { pixels: px, frames: n, delay: 60 };
  } },

  { name: "Digital rain", make(w, h) {
    const r = rng(3), cols = Array.from({ length: w }, () => ({ c: 1 + Math.floor(r() * 3), p: r() }));
    return buildLoop(120, w, h, 55, (f, put) => {
      for (let x = 0; x < w; x++) {
        const head = frac(f / 120 * cols[x].c + cols[x].p) * (h + 5);
        for (let j = 0; j < 5; j++) {
          const k = 1 - j / 5;
          put(x, Math.floor(head) - j, j ? 0 : 190, j ? 255 * k : 255, j ? 70 * k : 200);
        }
      }
    });
  } },

  { name: "Starfield", make(w, h) {
    const r = rng(11), stars = Array.from({ length: 36 }, () => ({ x: Math.floor(r() * w), y: Math.floor(r() * h), p: r() * 6.28, c: 1 + Math.floor(r() * 3), blue: r() > 0.6 }));
    return buildLoop(100, w, h, 60, (f, put, t) => {
      for (const s of stars) {
        const k = Math.pow(Math.max(0, Math.sin(s.p + t * s.c)), 3);
        put(s.x, s.y, ...tint(s.blue ? [120, 170, 255] : [255, 255, 230], k));
      }
    });
  } },

  { name: "Heartbeat", make(w, h) {
    const beat = [2, 2, 2, 2, 1, 3, 2, 2, 0, 4, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 2];
    return buildLoop(w, w, h, 45, (f, put) => {
      for (let x = 0; x < w; x++) {
        const k = Math.max(0, 1 - ((f - x + w) % w) / 40);   // the pen is at column f; older columns fade
        if (k > 0) put(x, beat[x % beat.length], 255 * k, 40 * k, 70 * k);
      }
    });
  } },

  { name: "Ocean", make: (w, h) => buildLoop(96, w, h, 60, (f, put, t) => {
    for (let x = 0; x < w; x++) {
      const level = 1.8 + 1.4 * Math.sin(x / 7 + t) + 0.6 * Math.sin(x / 3 - 2 * t);
      for (let y = 0; y < h; y++) {
        if (y < level) continue;
        if (y - level < 1) put(x, y, 150, 240, 255);
        else put(x, y, 0, 50 + 22 * (h - y), 120 + 28 * (h - y));
      }
    }
  }) },

  { name: "Northern lights", make: (w, h) => buildLoop(120, w, h, 60, (f, put, t) => {
    for (let x = 0; x < w; x++) {
      const top = 1.6 + Math.sin(x / 9 + t) + 0.8 * Math.sin(x / 5 - 2 * t + 1.3);
      const hue = 140 + 70 * Math.sin(x / 14 + t);
      for (let y = 0; y < h; y++) {
        const k = Math.max(0, 1 - Math.abs(y - top) / 2.6);
        put(x, y, ...tint(hsl(hue, 1, 0.5), k * k));
      }
    }
  }) },

  { name: "Scanner", make(w, h) {
    const n = 80, pos = f => (w - 1) * (0.5 - 0.5 * Math.cos(((f + n) % n) / n * Math.PI * 2));
    return buildLoop(n, w, h, 40, (f, put) => {
      for (let j = 0; j < 12; j++) {
        const p = pos(f - j), k = Math.pow(1 - j / 12, 2);
        for (let y = 0; y < h; y++) for (let d = -1; d <= 1; d++) put(p + d, y, 255 * k, 20 * k, 10 * k);
      }
    });
  } },

  { name: "Equalizer", make: (w, h) => buildLoop(96, w, h, 55, (f, put, t) => {
    for (let i = 0; i * 3 < w; i++) {
      const v = Math.abs(Math.sin(t * 2 + i * 1.7) * 0.6 + Math.sin(t * 3 + i * 0.9) * 0.4);
      const bars = 1 + Math.round(v * (h - 1));
      for (let j = 0; j < bars; j++) {
        const c = j >= h - 1 ? [255, 40, 40] : j >= h - 2 ? [255, 200, 0] : [40, 255, 90];
        put(i * 3, h - 1 - j, ...c); put(i * 3 + 1, h - 1 - j, ...c);
      }
    }
  }) },

  { name: "Comets", make(w, h) {
    const r = rng(21), cs = Array.from({ length: 6 }, (_, i) => ({ y: i % h, c: 1 + Math.floor(r() * 2), p: r(), hue: r() * 360, dir: r() > 0.5 ? 1 : -1 }));
    return buildLoop(110, w, h, 45, (f, put) => {
      for (const c of cs) {
        const u = frac(f / 110 * c.c + c.p) * (w + 14) - 7, head = c.dir > 0 ? u : w - 1 - u;
        for (let j = 0; j < 11; j++) {
          const k = Math.pow(1 - j / 11, 1.6);
          put(head - j * c.dir, c.y, ...(j ? tint(hsl(c.hue, 1, 0.5), k) : [255, 255, 255]));
        }
      }
    });
  } },

  { name: "Fireworks", make(w, h) {
    const n = 120, r = rng(5), bursts = Array.from({ length: 7 }, (_, i) => ({ s: Math.floor(i * n / 7 + r() * 8), x: 5 + r() * (w - 10), cy: 1 + Math.floor(r() * 2), hue: r() * 360 }));
    return buildLoop(n, w, h, 50, (f, put) => {
      for (const b of bursts) {
        const age = (f - b.s + n) % n;
        if (age < 6) { put(b.x, h - 1 - age * (h - 1 - b.cy) / 6, 255, 220, 160); continue; }
        const a = age - 6;
        if (a > 20) continue;
        const k = Math.pow(1 - a / 20, 1.5), c = tint(hsl(b.hue, 1, 0.55), k);
        for (let p = 0; p < 12; p++) {
          const ang = p / 12 * Math.PI * 2;
          put(b.x + Math.cos(ang) * a * 0.9, b.cy + Math.sin(ang) * a * 0.32 + a * a * 0.006, ...c);
        }
        if (a < 2) put(b.x, b.cy, 255, 255, 255);
      }
    });
  } },

  { name: "Snowfall", make(w, h) {
    const r = rng(9), fl = Array.from({ length: 34 }, () => ({ x: r() * w, c: 1 + Math.floor(r() * 2), p: r(), d: r() * 6.28, k: 0.45 + r() * 0.55 }));
    return buildLoop(140, w, h, 70, (f, put, t) => {
      for (const s of fl) put(s.x + Math.sin(t + s.d) * 1.5, frac(f / 140 * s.c + s.p) * (h + 1) - 0.5, 255 * s.k, 255 * s.k, 255 * s.k);
    });
  } },

  { name: "Lava", make: (w, h) => buildLoop(120, w, h, 60, (f, put, t) => {
    const blobs = [[1, 0, 2, 1], [2, 2, 1, 3], [1, 4, 3, 5], [3, 1, 2, 0.5]].map(([a, p, b, q]) =>
      [w * (0.5 + 0.42 * Math.sin(t * a + p)), (h - 1) * (0.5 + 0.5 * Math.cos(t * b + q))]);
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
      let v = 0;
      for (const [bx, by] of blobs) v += 2.2 / (((x - bx) / 3.2) ** 2 + (y - by) ** 2 + 0.6);
      if (v > 1) put(x, y, ...hsl(8 + Math.min(42, (v - 1) * 26), 1, Math.min(0.6, 0.3 + (v - 1) * 0.2)));
      else put(x, y, 70 * v * v, 0, 4 * v);
    }
  }) },

  { name: "Pulse rings", make: (w, h) => buildLoop(90, w, h, 50, (f, put, t) => {
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
      const d = Math.hypot((x - (w - 1) / 2) / 2.2, y - (h - 1) / 2);
      const k = Math.pow(Math.max(0, Math.sin(d * 1.1 - t * 2)), 3);
      put(x, y, ...tint(hsl((d * 14 + f * 4) % 360, 1, 0.5), k));
    }
  }) },

  { name: "Candy stripes", make: (w, h) => buildLoop(48, w, h, 60, (f, put) => {
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
      const on = frac((x + y * 2 + f * 0.5) / 12) < 0.5;   // 0.5 px per frame × 48 frames = two full stripes
      put(x, y, ...(on ? [255, 40, 140] : [0, 200, 255]));
    }
  }) },

  { name: "Pong", make(w, h) {
    const n = 124;
    return buildLoop(n, w, h, 45, (f, put) => {
      const bx = 2 + (w - 5) * tri(f / n), by = (h - 1) * tri(f / n * 5);
      for (let y = 0; y < h; y += 2) put((w - 1) / 2, y, 50, 50, 50);
      const py = Math.max(0, Math.min(h - 2, Math.round(by) - 1));
      for (let j = 0; j < 2; j++) { put(0, py + j, 0, 230, 255); put(w - 1, py + j, 255, 60, 160); }
      put(bx, by, 255, 255, 255);
    });
  } },

  { name: "Thunderstorm", make(w, h) {
    const n = 120, r = rng(13), drops = Array.from({ length: w }, () => ({ c: 2 + Math.floor(r() * 2), p: r() }));
    const bolts = Array.from({ length: 4 }, (_, i) => ({ s: Math.floor(i * n / 4 + r() * 14), x: 6 + r() * (w - 12), zig: Array.from({ length: h }, () => Math.round(r() * 2 - 1)) }));
    return buildLoop(n, w, h, 50, (f, put) => {
      for (let x = 0; x < w; x += 2) put(x, Math.floor(frac(f / n * drops[x].c + drops[x].p) * (h + 1)), 20, 50, 130);
      for (const b of bolts) {
        const age = (f - b.s + n) % n;
        if (age > 5) continue;
        const k = 1 - age / 6;
        if (age < 2) for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) put(x, y, 45, 45, 70);
        let x = b.x;
        for (let y = 0; y < h; y++) { x += b.zig[y]; put(x, y, 255 * k, 255 * k, 255 * k); }
      }
    });
  } },

  { name: "Confetti", make(w, h) {
    const r = rng(17), dots = Array.from({ length: 70 }, () => ({ x: Math.floor(r() * w), y: Math.floor(r() * h), p: r() * 6.28, c: 1 + Math.floor(r() * 3), hue: r() * 360 }));
    return buildLoop(90, w, h, 55, (f, put, t) => {
      for (const d of dots) put(d.x, d.y, ...tint(hsl(d.hue, 1, 0.55), Math.pow(Math.max(0, Math.sin(d.p + t * d.c)), 2)));
    });
  } },

  { name: "Oscilloscope", make: (w, h) => buildLoop(80, w, h, 45, (f, put, t) => {
    for (let x = 0; x < w; x++) {
      put(x, (h - 1) / 2 * (1 + Math.sin(x / 5 + t * 2)), 0, 255, 230);
      put(x, (h - 1) / 2 * (1 + Math.sin(x / 7 - t + 2)), 255, 40, 200);
    }
  }) },

  { name: "DNA", make: (w, h) => buildLoop(80, w, h, 55, (f, put, t) => {
    for (let x = 0; x < w; x++) {
      const s = Math.sin(x / 4 + t), front = Math.cos(x / 4 + t) > 0;
      const y1 = (h - 1) / 2 * (1 + s), y2 = (h - 1) / 2 * (1 - s);
      if (x % 4 === 0) for (let y = Math.min(y1, y2) + 1; y < Math.max(y1, y2); y++) put(x, y, 60, 60, 80);
      put(x, y1, ...tint([0, 220, 255], front ? 1 : 0.4));
      put(x, y2, ...tint([255, 60, 170], front ? 0.4 : 1));
    }
  }) },
];

// Suggested brightness per style: sparse animations can run brighter than ones that light every LED.
const SPARSE_STYLES = new Set(["Starfield", "Snowfall", "Scanner", "Comets", "Fireworks", "Heartbeat", "Pong", "Oscilloscope", "DNA", "Digital rain", "Thunderstorm", "Confetti"]);
for (const s of HALCYON_STYLES) s.bright = SPARSE_STYLES.has(s.name) ? 50 : 25;
