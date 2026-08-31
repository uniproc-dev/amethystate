import Delaunator from 'delaunator';

const NS = 'http://www.w3.org/2000/svg';
const LIGHT = norm3([-0.40, -0.66, 0.64]);

function norm3(v) {
  const l = Math.hypot(v[0], v[1], v[2]) || 1;
  return [v[0] / l, v[1] / l, v[2] / l];
}

function rng(seed) {
  let s = seed >>> 0 || 1;
  return () => {
    s ^= s << 13; s >>>= 0;
    s ^= s >> 17;
    s ^= s << 5;  s >>>= 0;
    return s / 4294967296;
  };
}
const rand = (r, a, b) => a + (b - a) * r();
const smooth = t => t * t * (3 - 2 * t);

function hsl(h, s, l) {
  return `hsl(${((h % 360) + 360) % 360} ${clamp(s, 0, 100)}% ${clamp(l, 0, 100)}%)`;
}
const clamp = (v, a, b) => Math.min(b, Math.max(a, v));

function spaceOut(pts, minDist) {
  const cell = Math.max(1, minDist), grid = new Map(), keep = [];
  const key = (i, j) => i + ',' + j;
  for (const p of pts) {
    const gi = Math.floor(p[0]/cell), gj = Math.floor(p[1]/cell);
    let ok = true;
    for (let i = gi-1; i <= gi+1 && ok; i++)
      for (let j = gj-1; j <= gj+1 && ok; j++)
        for (const q of grid.get(key(i,j)) || [])
          if (Math.hypot(p[0]-q[0], p[1]-q[1]) < minDist) { ok = false; break; }
    if (!ok) continue;
    const k = key(gi, gj);
    (grid.get(k) || grid.set(k, []).get(k)).push(p);
    keep.push(p);
  }
  return keep;
}

function quality(A, B, C) {
  const ab = (B[0]-A[0])**2 + (B[1]-A[1])**2;
  const bc = (C[0]-B[0])**2 + (C[1]-B[1])**2;
  const ca = (A[0]-C[0])**2 + (A[1]-C[1])**2;
  const area = Math.abs((B[0]-A[0])*(C[1]-A[1]) - (C[0]-A[0])*(B[1]-A[1])) / 2;
  return clamp((4*Math.sqrt(3)*area) / (ab + bc + ca + 1e-9), 0, 1);
}

function polyDist(px, py, poly) {
  let inside = false, best = Infinity;
  for (let i = 0, n = poly.length; i < n; i++) {
    const [x1, y1] = poly[i], [x2, y2] = poly[(i + 1) % n];
    if ((y1 > py) !== (y2 > py) && px < ((x2 - x1) * (py - y1)) / (y2 - y1 + 1e-12) + x1)
      inside = !inside;
    const ax = x2 - x1, ay = y2 - y1;
    const t = clamp(((px - x1) * ax + (py - y1) * ay) / (ax * ax + ay * ay + 1e-12), 0, 1);
    best = Math.min(best, Math.hypot(px - (x1 + t * ax), py - (y1 + t * ay)));
  }
  return inside ? -best : best;
}

function silhouette(w, h, r, spread, room, apron = 0) {
  const L = 2 * (w + h);
  const base = Math.max(16, Math.min(w, h) * 0.055);
  const amp = base + Math.min(w, h) * spread * 0.95;
  const n = clamp(Math.round(11 + L / 420), 12, 26);

  let ts = [];
  for (let i = 0; i < n; i++)
    ts.push((((i * L) / n + rand(r, -L / n * 0.42, L / n * 0.42)) % L + L) % L);
  for (const t of ts.slice())
    if (r() < 0.3) ts.push(((t + (r() < 0.5 ? -1 : 1) * rand(r, 9, 26)) % L + L) % L);
  ts.push(0, w, w + h, 2 * w + h);
  ts.sort((a, b) => a - b);

  const P = [];
  for (const t of ts) {
    let p, nr;
    if (t <= w)            { p = [t, 0];         nr = [0, -1]; }
    else if (t <= w + h)   { p = [w, t - w];     nr = [1, 0]; }
    else if (t <= 2*w + h) { p = [2*w + h - t, h]; nr = [0, 1]; }
    else                   { p = [0, L - t];     nr = [-1, 0]; }

    const d = Math.min(t, Math.abs(t - w), Math.abs(t - (w + h)),
                       Math.abs(t - (2*w + h)), Math.abs(L - t));
    if (d < base * 1.6) {
      const vx = p[0] - w / 2, vy = p[1] - h / 2, vl = Math.hypot(vx, vy) || 1;
      nr = [vx / vl, vy / vl];
    }

    const u = r();
    let off = u < 0.32 ? base * rand(r, 0.85, 1.25)
            : u > 0.85 ? base + amp * rand(r, 1.3, 2.0)
                       : base + amp * rand(r, 0.10, 0.70);
    if (room && Math.abs(nr[0]) > 1e-6) {
      const avail = (nr[0] < 0 ? room[0] : room[1]) / Math.abs(nr[0]);
      if (avail < off) off = Math.max(base * 0.5, avail * rand(r, 0.55, 0.98));
    }
    off += apron;
    P.push([p[0] + nr[0] * off, p[1] + nr[1] * off]);
  }
  return P;
}

/** Geometry of one outcrop over a group of rectangles. Costly; runs once per layout. */
export function prepareShard(rects, opts = {}) {
  const { seed = 1, spread = 0.14, pageWidth = null, margin = 8,
          density: dens0 = null, plateauPad = 56, flatSpread = 0.05,
          apron = 0, rampFactor = 1.0 } = opts;

  const gx0 = Math.min(...rects.map(r => r[0]));
  const gy0 = Math.min(...rects.map(r => r[1]));
  const gx1 = Math.max(...rects.map(r => r[0] + r[2]));
  const gy1 = Math.max(...rects.map(r => r[1] + r[3]));
  const gw = gx1 - gx0, gh = gy1 - gy0;
  const r = rng(seed);
  const density = dens0 ?? Math.max(72, Math.min(gw, gh) / 3.2);

  const room = pageWidth == null ? null
    : [Math.max(6, gx0 - margin), Math.max(6, pageWidth - gx1 - margin)];

  let poly = silhouette(gw, gh, r, spread, room, apron);
  const x0 = Math.min(...poly.map(p => p[0])) - 6;
  const y0 = Math.min(...poly.map(p => p[1])) - 6;
  const W  = Math.max(...poly.map(p => p[0])) + 6 - x0;
  const H  = Math.max(...poly.map(p => p[1])) + 6 - y0;
  poly = poly.map(([x, y]) => [x - x0, y - y0]);
  const local = rects.map(([x, y, w, h]) => [x - gx0 - x0, y - gy0 - y0, w, h]);

  const flatPolys = local.map(([x, y, w, h]) => {
    const px = x - plateauPad, py = y - plateauPad;
    return silhouette(w + 2*plateauPad, h + 2*plateauPad, r, flatSpread, null)
             .map(([vx, vy]) => [vx + px, vy + py]);
  });

  const pts = [];
  for (let y = 0; y < H; y += density)
    for (let x = 0; x < W; x += density) {
      const px = x + rand(r, -density * 0.26, density * 0.26);
      const py = y + rand(r, -density * 0.26, density * 0.26);
      if (polyDist(px, py, poly) < -density * 0.35) pts.push([px, py]);
    }
  for (const P2 of [poly, ...flatPolys])
    for (let i = 0; i < P2.length; i++) {
      const a = P2[i], b = P2[(i + 1) % P2.length];
      const k = Math.max(2, Math.floor(Math.hypot(b[0]-a[0], b[1]-a[1]) / (density*0.55)) + 1);
      for (let j = 0; j < k; j++)
        pts.push([a[0] + (b[0]-a[0]) * j/k, a[1] + (b[1]-a[1]) * j/k]);
    }
  const filtered = spaceOut(pts, density * 0.30);
  pts.length = 0; pts.push(...filtered);

  const ramp = Math.max(density * 1.1, Math.min(W - gw, H - gh) * 0.75) * rampFactor;
  const flatIdx = [], wts = [], edgeT = [], ripple = [], tiltOff = [];
  const origin = [], dn = [];
  let dMaxOut = 1e-6;
  for (let i = 0; i < pts.length; i++) {
    const [x, y] = pts[i];
    const ds = flatPolys.map(fp => polyDist(x, y, fp));
    let inside = -1;
    for (let k = 0; k < ds.length; k++) if (ds[k] <= 0) { inside = k; break; }
    flatIdx[i] = inside;
    tiltOff[i] = rand(r, -1, 1);
    if (inside >= 0) {
      wts[i] = null; edgeT[i] = 1; ripple[i] = 0;
      let bx = x, by = y, bd = Infinity;
      for (const [rx, ry, rw, rh] of local) {
        const qx = clamp(x, rx, rx + rw), qy = clamp(y, ry, ry + rh);
        const d = Math.hypot(x - qx, y - qy);
        if (d < bd) { bd = d; bx = qx; by = qy; }
      }
      origin[i] = [bx, by]; dn[i] = bd;
      if (bd > dMaxOut) dMaxOut = bd;
      continue;
    }
    const w = ds.map(d => smooth(clamp(1 - d / ramp, 0, 1)));
    wts[i] = w;
    edgeT[i] = smooth(clamp(-polyDist(x, y, poly) / (density * 1.15), 0, 1));
    ripple[i] = 15 * Math.sin(x / 210) * Math.cos(y / 240);
    let bx = x, by = y, bd = Infinity;
    for (const [rx, ry, rw, rh] of local) {
      const qx = clamp(x, rx, rx + rw), qy = clamp(y, ry, ry + rh);
      const d = Math.hypot(x - qx, y - qy);
      if (d < bd) { bd = d; bx = qx; by = qy; }
    }
    origin[i] = [bx, by];
    dn[i] = bd;
    if (bd > dMaxOut) dMaxOut = bd;
  }
  for (let i = 0; i < dn.length; i++) dn[i] = clamp(dn[i] / dMaxOut, 0, 1);

  const del = new Delaunator(Float64Array.from(pts.flat()));
  const tris = [];
  for (let i = 0; i < del.triangles.length; i += 3) {
    const a = del.triangles[i], b = del.triangles[i+1], c = del.triangles[i+2];
    const cx = (pts[a][0]+pts[b][0]+pts[c][0])/3, cy = (pts[a][1]+pts[b][1]+pts[c][1])/3;
    if (polyDist(cx, cy, poly) >= 0) continue;
    let forced = false;
    const tx0 = Math.min(pts[a][0],pts[b][0],pts[c][0]), tx1 = Math.max(pts[a][0],pts[b][0],pts[c][0]);
    const ty0 = Math.min(pts[a][1],pts[b][1],pts[c][1]), ty1 = Math.max(pts[a][1],pts[b][1],pts[c][1]);
    for (const [rx, ry, rw, rh] of local)
      if (tx1 > rx && tx0 < rx+rw && ty1 > ry && ty0 < ry+rh) { forced = true; break; }
    const flat = (flatIdx[a] >= 0 && flatIdx[a] === flatIdx[b] && flatIdx[b] === flatIdx[c]) || forced;
    tris.push({ a, b, c, flat, q: smooth(clamp(quality(pts[a], pts[b], pts[c]) / 0.40, 0, 1)),
                jit: rand(r, -7, 7), jitS: rand(r, -1, 1), jitH: rand(r, -1, 1) });
  }

  return { pts, tris, flatIdx, wts, edgeT, ripple, tiltOff, origin, dn,
           nRects: local.length,
           W, H, x: gx0 + x0, y: gy0 + y0,
           poly: poly.map(([px, py]) => [px + gx0 + x0, py + gy0 + y0]) };
}

/** Shades prepared geometry into SVG. Cheap enough to call every frame. */
export function renderShard(g, opts = {}) {
  const { depth = 150, hue = 266, floor = 7, flatTilt = 0, sweep = 0, sweepScale = 1.0,
          flatHueJitter = 5, flatSatJitter = 5, flatLigJitter = 0,
          grow = 1, stagger = 0.5 } = opts;

  const dArr = Array.isArray(depth) ? Array.from({length: g.nRects}, (_, i) => depth[i] ?? depth[0])
                                    : Array.from({length: g.nRects}, () => depth);
  const dMax = Math.max(...dArr, 1e-6);

  const P = g.pts;
  const XY = grow >= 1 ? P : P.map((p, i) => {
    const s0 = g.dn[i] * stagger;
    const gi = smooth(clamp((grow - s0) / Math.max(1e-6, 1 - s0), 0, 1));
    const o = g.origin[i];
    return [o[0] + (p[0] - o[0]) * gi, o[1] + (p[1] - o[1]) * gi];
  });
  const growF = new Float64Array(P.length);
  for (let i = 0; i < P.length; i++) {
    if (grow >= 1) { growF[i] = 1; continue; }
    const s0 = g.dn[i] * stagger;
    growF[i] = smooth(clamp((grow - s0) / Math.max(1e-6, 1 - s0), 0, 1));
  }

  const Z = new Float64Array(P.length);
  for (let i = 0; i < P.length; i++) {
    if (g.flatIdx[i] >= 0) { Z[i] = dArr[g.flatIdx[i]] + g.tiltOff[i] * flatTilt; continue; }
    const w = g.wts[i];
    let wsum = 0, dsum = 0;
    for (let k = 0; k < w.length; k++) { wsum += w[k]; dsum += w[k] * dArr[k]; }
    const target = wsum > 1e-6 ? dsum / wsum : dMax;
    const t = clamp(wsum, 0, 1) * g.edgeT[i];
    const z = target * t - dMax * 0.18 * (1 - t) + g.ripple[i] * t * (1 - t) * 2;
    Z[i] = z * growF[i];
  }

  const out = [`<svg xmlns="${NS}" viewBox="0 0 ${g.W.toFixed(0)} ${g.H.toFixed(0)}" `
             + `width="${g.W.toFixed(0)}" height="${g.H.toFixed(0)}" aria-hidden="true" focusable="false">`];
  const shaded = [];
  for (const t of g.tris) {
    const a = t.a, b = t.b, c = t.c;
    const A = [XY[a][0],XY[a][1],Z[a]], B = [XY[b][0],XY[b][1],Z[b]], C = [XY[c][0],XY[c][1],Z[c]];
    let N = norm3([
      (B[1]-A[1])*(C[2]-A[2]) - (B[2]-A[2])*(C[1]-A[1]),
      (B[2]-A[2])*(C[0]-A[0]) - (B[0]-A[0])*(C[2]-A[2]),
      (B[0]-A[0])*(C[1]-A[1]) - (B[1]-A[1])*(C[0]-A[0]),
    ]);
    if (N[2] < 0) N = N.map(v => -v);
    let lam = clamp(N[0]*LIGHT[0] + N[1]*LIGHT[1] + N[2]*LIGHT[2], 0, 1);
    lam = lam*t.q + clamp(LIGHT[2], 0, 1)*(1 - t.q);
    const lamF = t.flat && flatTilt === 0 ? clamp(LIGHT[2], 0, 1) : lam;
    const cz = (A[2]+B[2]+C[2])/3;
    let sh = clamp(0.30*clamp(cz/dMax, 0, 1) + 0.70*Math.pow(lamF, 1.7), 0, 1);
    if (sweep) {
      const gx_ = ((A[0]+B[0]+C[0])/3)/(g.W*sweepScale), gy_ = ((A[1]+B[1]+C[1])/3)/(g.H*sweepScale);
      sh = clamp(sh + sweep*(0.6*Math.sin(gx_*2.1 + 0.7) + 0.4*Math.cos(gy_*1.7 - 0.4)), 0, 1);
    }
    shaded.push({ t, cz, sh, N });
  }
  shaded.sort((p, q) => p.cz - q.cz);

  for (const { t, sh } of shaded) {
    const p = [t.a, t.b, t.c].map(k => `${XY[k][0].toFixed(0)},${XY[k][1].toFixed(0)}`).join(' ');
    const col = hsl(
      hue + 18*sh + (t.flat ? t.jitH*flatHueJitter : t.jit),
      34 + 20*(1-sh) + (t.flat ? t.jitS*flatSatJitter : 0),
      floor + (43-floor)*Math.pow(sh, 1.7) + (t.flat ? t.jitS*flatLigJitter : 0));
    out.push(`<polygon points="${p}" fill="${col}" stroke="${col}" stroke-width="0.7"/>`);
  }

  const edges = new Map();
  for (const s of shaded)
    for (const [u, v] of [[s.t.a,s.t.b],[s.t.b,s.t.c],[s.t.c,s.t.a]]) {
      const k = u < v ? `${u},${v}` : `${v},${u}`;
      (edges.get(k) || edges.set(k, []).get(k)).push(s);
    }
  for (const [k, ss] of edges) {
    if (ss.length !== 2) continue;
    if (ss[0].t.flat && ss[1].t.flat) continue;
    const dp = ss[0].N[0]*ss[1].N[0] + ss[0].N[1]*ss[1].N[1] + ss[0].N[2]*ss[1].N[2];
    const lit = Math.max(ss[0].sh, ss[1].sh);
    if (dp > 0.80 || lit < 0.34) continue;
    const [u, v] = k.split(',').map(Number);
    out.push(`<line x1="${XY[u][0].toFixed(0)}" y1="${XY[u][1].toFixed(0)}" `
           + `x2="${XY[v][0].toFixed(0)}" y2="${XY[v][1].toFixed(0)}" `
           + `stroke="#d3b2ea" stroke-width="0.9" stroke-opacity="${Math.min(0.55,(0.84-dp)*1.7*lit).toFixed(2)}"/>`);
  }
  out.push('</svg>');
  return { svg: out.join(''), x: g.x, y: g.y, w: g.W, h: g.H, faces: g.tris.length, poly: g.poly };
}

function noise2(x, y, seed) {
  const h = (i, j) => {
    let n = (i * 374761393 + j * 668265263 + seed * 1442695040888963407) | 0;
    n = (n ^ (n >> 13)) * 1274126177;
    return ((n ^ (n >> 16)) >>> 0) / 4294967296;
  };
  const i = Math.floor(x), j = Math.floor(y), fx = x - i, fy = y - j;
  const u = fx*fx*(3-2*fx), v = fy*fy*(3-2*fy);
  return (h(i,j)*(1-u) + h(i+1,j)*u)*(1-v) + (h(i,j+1)*(1-u) + h(i+1,j+1)*u)*v;
}

function fbm(x, y, seed, oct = 4, scale = 900) {
  let a = 1, f = 1 / scale, sum = 0, norm = 0;
  for (let o = 0; o < oct; o++) {
    sum += a * noise2(x*f, y*f, seed + o*77);
    norm += a; a *= 0.5; f *= 2.07;
  }
  return sum / norm;
}

/** Matrix rock the outcrops grow out of. `sockets` are outcrop contours in page coordinates. */
export function buildMatrix(W, H, sockets, opts = {}) {
  const { seed = 3, step = 300, lo = 4.5, hi = 19, hue = 264, relief = 52,
          sink = 92, sizeVar = 0.62 } = opts;
  const r = rng(seed);
  const pts = [];

  for (let gy = -1; gy < H/step + 2; gy++)
    for (let gx = -1; gx < W/step + 2; gx++) {
      const bx = gx*step, by = gy*step;
      const d = fbm(bx, by, seed + 500, 3, 1400);
      const n = d > 0.62 ? 3 : d > 0.38 ? 2 : 1;
      for (let k = 0; k < n; k++)
        pts.push([bx + rand(r, -step*0.5, step*0.5) * (1 + sizeVar*(1-d)),
                  by + rand(r, -step*0.5, step*0.5) * (1 + sizeVar*(1-d))]);
    }

  for (const poly of sockets) {
    const cx = poly.reduce((a,p) => a+p[0], 0)/poly.length;
    const cy = poly.reduce((a,p) => a+p[1], 0)/poly.length;
    for (const k of [1.04, 1.22, 1.5])
      for (let i = 0; i < poly.length; i++) {
        const a = poly[i], b = poly[(i+1)%poly.length];
        const kk = Math.max(1, Math.round(Math.hypot(b[0]-a[0], b[1]-a[1]) / (step*0.45)));
        for (let j = 0; j < kk; j++) {
          const px = a[0] + (b[0]-a[0])*j/kk, py = a[1] + (b[1]-a[1])*j/kk;
          pts.push([cx + (px-cx)*k + rand(r,-18,18), cy + (py-cy)*k + rand(r,-18,18)]);
        }
      }
  }

  const kept = spaceOut(pts, step * 0.34);
  pts.length = 0; pts.push(...kept);

  const Z = pts.map(([x, y]) => {
    let z = relief * (fbm(x, y, seed, 5, 1100) - 0.5)
          + relief * 0.45 * (fbm(x, y, seed + 91, 3, 320) - 0.5);
    for (const poly of sockets) {
      const d = polyDist(x, y, poly);
      z -= sink * smooth(clamp(1 - Math.max(d, 0)/260, 0, 1));
    }
    return z;
  });

  const zmin = Math.min(...Z), zmax = Math.max(...Z);
  const del = new Delaunator(Float64Array.from(pts.flat()));
  const tri = [];
  for (let i = 0; i < del.triangles.length; i += 3) {
    const [a,b,c] = [del.triangles[i], del.triangles[i+1], del.triangles[i+2]];
    const A=[pts[a][0],pts[a][1],Z[a]], B=[pts[b][0],pts[b][1],Z[b]], C=[pts[c][0],pts[c][1],Z[c]];
    let N = norm3([
      (B[1]-A[1])*(C[2]-A[2])-(B[2]-A[2])*(C[1]-A[1]),
      (B[2]-A[2])*(C[0]-A[0])-(B[0]-A[0])*(C[2]-A[2]),
      (B[0]-A[0])*(C[1]-A[1])-(B[1]-A[1])*(C[0]-A[0]),
    ]);
    if (N[2] < 0) N = N.map(v => -v);
    let lam = clamp(N[0]*LIGHT[0]+N[1]*LIGHT[1]+N[2]*LIGHT[2], 0, 1);
    const q = smooth(clamp(quality(A, B, C) / 0.40, 0, 1));
    lam = lam*q + clamp(LIGHT[2], 0, 1)*(1 - q);
    const cz = (A[2]+B[2]+C[2])/3;
    const sh = clamp(0.38*((cz-zmin)/(zmax-zmin+1e-9)) + 0.62*Math.pow(lam,1.9), 0, 1);
    tri.push({ a, b, c, cz, sh });
  }
  tri.sort((p, q) => p.cz - q.cz);
  const out = [`<svg xmlns="${NS}" viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" aria-hidden="true" focusable="false">`,
               `<rect width="${W}" height="${H}" fill="${hsl(266,34,lo)}"/>`];
  for (const t of tri) {
    const p = [t.a,t.b,t.c].map(k => `${pts[k][0].toFixed(0)},${pts[k][1].toFixed(0)}`).join(' ');
    const col = hsl(hue + 10*t.sh + rand(r,-6,6), 32 + 16*(1-t.sh),
                    lo + (hi-lo)*Math.pow(t.sh,1.5));
    out.push(`<polygon points="${p}" fill="${col}" stroke="${col}" stroke-width="0.7"/>`);
  }
  out.push('</svg>');
  return { svg: out.join(''), faces: tri.length };
}

/**
 * Measures the real boxes of `groups` inside `container` and draws stone under them.
 * Returns a destroy function carrying `update`, `redraw`, `rise` and `freeze`.
 * Markup drives geometry: `data-elev` sets a box's own elevation, `data-pad` pads
 * the measured box.
 */
export function mountCrystals({ container, groups, matrix = true, spread = 0.14,
                                seed = 500, pad = 0, plateauPad = 56,
                                hue = 266, floor = 7,
                                flatHueJitter = 5, flatSatJitter = 5,
                                flatLigJitter = 0, flatTilt = 0, sweep = 0,
                                apron = 0, rampFactor = 1.0, depth = 150,
                                animateIn = 0, easing = 'inOut', stagger = 0.5,
                                onDone } = {}) {
  if (!container) return () => {};
  const layer = document.createElement('div');
  layer.setAttribute('aria-hidden', 'true');
  Object.assign(layer.style, { position: 'absolute', inset: '0', zIndex: '0',
                               pointerEvents: 'none', overflow: 'hidden' });
  if (getComputedStyle(container).position === 'static') container.style.position = 'relative';
  container.prepend(layer);

  const cfg = { spread, plateauPad, hue, floor, flatHueJitter, flatSatJitter,
                flatLigJitter, flatTilt, sweep, apron, rampFactor, depth, matrix,
                stagger };
  const GEOM_KEYS = ['spread', 'plateauPad', 'apron', 'rampFactor'];

  const resolve = g => (Array.isArray(g) ? g : [g])
    .flatMap(s => typeof s === 'string' ? [...container.querySelectorAll(s)] : [s])
    .filter(Boolean);

  let geoms = [], baseDepths = [], mxCache = { key: null, html: '' };
  let W = 0, H = 0;

  function layout() {
    const base = container.getBoundingClientRect();
    W = Math.round(base.width); H = Math.round(container.scrollHeight);
    geoms = []; baseDepths = [];
    groups.forEach((g, i) => {
      const els = resolve(g);
      const rects = els.map(el => {
        const b = el.getBoundingClientRect();
        const v = parseFloat(el.dataset.pad);
        const p = Number.isFinite(v) ? v : pad;
        return [b.left - base.left - p, b.top - base.top - p,
                b.width + 2*p, b.height + 2*p];
      });
      if (!rects.length) return;
      geoms.push(prepareShard(rects, { seed: seed + i, pageWidth: W, ...cfg }));
      baseDepths.push(els.map(el => {
        const v = parseFloat(el.dataset.elev);
        return Number.isFinite(v) ? v : cfg.depth;
      }));
    });
  }

  function paint(k = 1) {
    if (!geoms.length) return;
    const shards = geoms.map((g, i) => {
      const d = baseDepths[i].map(v => v * k - (1 - k) * 46);
      return renderShard(g, { ...cfg, depth: d, grow: k });
    });
    let html = '';
    if (cfg.matrix) {
      const key = W + 'x' + H + '|' + geoms.map(g =>
        g.poly.map(v => v.map(Math.round).join()).join(';')).join('|');
      if (key !== mxCache.key)
        mxCache = { key, html: `<div style="position:absolute;inset:0">${buildMatrix(W, H, geoms.map(g => g.poly)).svg}</div>` };
      html += mxCache.html;
    }
    for (const s of shards)
      html += `<div style="position:absolute;left:${s.x.toFixed(0)}px;top:${s.y.toFixed(0)}px;`
            + `width:${s.w.toFixed(0)}px;height:${s.h.toFixed(0)}px">${s.svg}</div>`;
    layer.innerHTML = html;
    layer.querySelectorAll('svg').forEach(sv => { sv.style.display = 'block';
      sv.style.width = '100%'; sv.style.height = '100%'; });
    onDone?.(shards);
  }

  const draw = () => { layout(); paint(); };

  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
  const EASE = {
    out:   p => 1 - Math.pow(1 - p, 3),
    inOut: p => p < 0.5 ? 4*p*p*p : 1 - Math.pow(-2*p + 2, 3)/2,
    soft:  p => p*p*(3 - 2*p),
  };
  let anim = 0;
  function rise(ms = 1600, ease = 'inOut') {
    cancelAnimationFrame(anim);
    if (!ms || reduced) return paint(1);
    const fn = EASE[ease] || EASE.inOut;
    const t0 = performance.now();
    const step = now => {
      const p = clamp((now - t0) / ms, 0, 1);
      paint(fn(p));
      if (p < 1) anim = requestAnimationFrame(step);
    };
    anim = requestAnimationFrame(step);
  }

  layout();
  rise(animateIn, easing);
  document.fonts?.ready.then(draw).catch(() => {});

  let t, lw = 0, lh = 0;
  const schedule = () => {
    const b = container.getBoundingClientRect();
    const h = container.scrollHeight;
    if (Math.abs(b.width - lw) < 2 && Math.abs(h - lh) < 2) return;
    lw = b.width; lh = h;
    clearTimeout(t); t = setTimeout(draw, 140);
  };
  const ro = new ResizeObserver(schedule);
  ro.observe(container);
  for (const g of groups) for (const el of resolve(g)) ro.observe(el);
  addEventListener('resize', schedule, { passive: true });

  const destroy = () => { cancelAnimationFrame(anim); ro.disconnect();
                          removeEventListener('resize', schedule);
                          clearTimeout(t); layer.remove(); };
  destroy.update = patch => {
    const needGeom = Object.keys(patch).some(k => GEOM_KEYS.includes(k));
    Object.assign(cfg, patch);
    if (needGeom) layout();
    paint();
  };
  destroy.redraw = draw;
  destroy.rise = rise;
  destroy.freeze = k => { cancelAnimationFrame(anim); paint(clamp(k, 0, 1)); };
  return destroy;
}
