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

/**
 * Thins points to a minimum spacing. `radiusAt` may vary the spacing across the
 * field, which is how the grain can change gradually: decimating a lattice by whole
 * steps instead puts a hard seam wherever the step changes, and the seam takes the
 * shape of whatever decided it.
 */
function spaceOut(pts, minDist, radiusAt = null) {
  const cell = Math.max(1, minDist), grid = new Map(), keep = [];
  const key = (i, j) => i + ',' + j;
  for (const p of pts) {
    const rp = radiusAt ? radiusAt(p) : minDist;
    const gi = Math.floor(p[0]/cell), gj = Math.floor(p[1]/cell);
    const span = Math.max(1, Math.ceil(rp / cell));
    let ok = true;
    // The radius is kept beside the point, never on it: these arrays are flattened
    // into the triangulation, and a third number there shifts every index.
    for (let i = gi-span; i <= gi+span && ok; i++)
      for (let j = gj-span; j <= gj+span && ok; j++)
        for (const [q, rq] of grid.get(key(i,j)) || [])
          if (Math.hypot(p[0]-q[0], p[1]-q[1]) < Math.max(rp, rq)) { ok = false; break; }
    if (!ok) continue;
    const k = key(gi, gj);
    (grid.get(k) || grid.set(k, []).get(k)).push([p, rp]);
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

/**
 * The plateau contour: the box offset outwards by `pad`, with the corners turned
 * through an arc of radius `pad * round`. Straight runs stay straight and the
 * corners come round, so the shape reads as a rounded shelf rather than a square,
 * and it still contains the box by construction. Jitter only ever pushes outwards,
 * which keeps that guarantee.
 */
function plateauContour(w, h, pad, round, r, jitter, bulge = 0.9) {
  const cx = w / 2, cy = h / 2;
  const boxW = cx + pad, boxH = cy + pad;
  // The ellipse of the box's own aspect that circumscribes it: at the corners
  // (w/2)^2/a^2 + (h/2)^2/b^2 = 1, so the box is inscribed exactly.
  const a = cx * Math.SQRT2 + pad, b = cy * Math.SQRT2 + pad;

  const t = clamp(round, 0, 1);
  const n = clamp(Math.round((boxW + boxH) / Math.max(13, pad * 0.55)), 12, 110);
  const harmonics = [[1, r() * 6.283, 1.0], [2, r() * 6.283, 0.62], [3, r() * 6.283, 0.34]];
  const weight = harmonics.reduce((s, [, , k]) => s + k, 0);

  const P = [];
  for (let i = 0; i < n; i++) {
    const th = ((i + rand(r, -0.32, 0.32)) / n) * Math.PI * 2;
    const c = Math.cos(th), s = Math.sin(th);

    // Both shapes contain the box, so any blend of their radii contains it too.
    const rect = Math.min(boxW / Math.max(Math.abs(c), 1e-6),
                          boxH / Math.max(Math.abs(s), 1e-6));
    const ell = 1 / Math.sqrt((c / a) ** 2 + (s / b) ** 2);
    const base = rect * (1 - t) + ell * t;

    let sum = 0;
    for (const [k, phase, amp] of harmonics) sum += amp * Math.sin(k * th + phase);
    const wave = 0.5 + 0.5 * sum / weight;

    const rad = base * (1 + bulge * 0.18 * wave)
              + (jitter > 0 ? rand(r, 0, pad * jitter * 2) : 0);
    P.push([cx + c * rad, cy + s * rad]);
  }
  return P;
}

function silhouette(w, h, r, spread, room, apron = 0, shapeRound = 0) {
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

  if (shapeRound <= 0) return P;

  // Extruding a perimeter can only ever give a boxy outline. Pulling each radius
  // towards the ellipse that circumscribes the block turns the whole body round
  // while keeping the irregularity as a wobble on top of it. Both radii already
  // clear the block, so blending them still clears it.
  const cx = w / 2, cy = h / 2;
  const t = clamp(shapeRound, 0, 1);
  const A = cx * Math.SQRT2 + base + apron, B = cy * Math.SQRT2 + base + apron;
  return P.map(([x, y]) => {
    const vx = x - cx, vy = y - cy;
    const len = Math.hypot(vx, vy) || 1;
    const c = vx / len, s = vy / len;
    const ell = 1 / Math.sqrt((c / A) ** 2 + (s / B) ** 2);
    const rad = len * (1 - t) + ell * t;
    return [cx + c * rad, cy + s * rad];
  });
}

/** Geometry of one outcrop over a group of rectangles. Costly; runs once per layout. */
export function prepareShard(rects, opts = {}) {
  const { seed = 1, spread = 0.14, pageWidth = null, margin = 8,
          density: dens0 = null, tessel: tess0 = null,
          plateauPad = 56, flatSpread = 0.05, plateauRound = 1, plateauBulge = 0.9,
          apron = 0, rampFactor = 1.0, shapeRound = 0.8, plateauGrain = 2.4 } = opts;

  const gx0 = Math.min(...rects.map(r => r[0]));
  const gy0 = Math.min(...rects.map(r => r[1]));
  const gx1 = Math.max(...rects.map(r => r[0] + r[2]));
  const gy1 = Math.max(...rects.map(r => r[1] + r[3]));
  const gw = gx1 - gx0, gh = gy1 - gy0;
  const r = rng(seed);
  const density = dens0 ?? Math.max(72, Math.min(gw, gh) / 3.2);
  const step = tess0 || density;

  const room = pageWidth == null ? null
    : [Math.max(6, gx0 - margin), Math.max(6, pageWidth - gx1 - margin)];

  // The body is grown from the plateau's extent, not from the bare text box: the
  // plateau is the box plus plateauPad, and the outcrop has to be that plus its own
  // rise and apron. Built from the box alone, a wide plateau reaches past the body
  // it is meant to sit on and the skirt disappears.
  // The plateau reaches plateauPad beyond the box and then bulges on top of that,
  // so the body has to be grown by at least as much again before its own rise and
  // apron are added. Built from the bare box, a wide plateau overruns it and the
  // skirt vanishes.
  const under = plateauPad * (1 + plateauBulge * 0.18) + 0.12 * Math.min(gw, gh);
  let poly = silhouette(gw + 2 * under, gh + 2 * under, r, spread, room, apron, shapeRound)
    .map(([px, py]) => [px - under, py - under]);
  const x0 = Math.min(...poly.map(p => p[0])) - 6;
  const y0 = Math.min(...poly.map(p => p[1])) - 6;
  const W  = Math.max(...poly.map(p => p[0])) + 6 - x0;
  const H  = Math.max(...poly.map(p => p[1])) + 6 - y0;
  poly = poly.map(([x, y]) => [x - x0, y - y0]);
  const local = rects.map(([x, y, w, h]) => [x - gx0 - x0, y - gy0 - y0, w, h]);

  const flatPolys = local.map(([x, y, w, h]) =>
    plateauContour(w, h, plateauPad, plateauRound, r, flatSpread, plateauBulge)
      .map(([vx, vy]) => [vx + x, vy + y]));

  const pts = [];
  const nearRect = (px, py) => {
    let best = Infinity;
    for (const [rx, ry, rw, rh] of local) {
      const dx = Math.max(rx - px, px - (rx + rw), 0);
      const dy = Math.max(ry - py, py - (ry + rh), 0);
      best = Math.min(best, Math.hypot(dx, dy));
    }
    return best;
  };

  // Cut stone carries one broad table and keeps the small faceting for the slopes.
  // The grain is a field, not a switch: points are laid at the fine step everywhere
  // and thinned by a spacing that grows towards the text, so the density changes
  // gradually instead of leaving a rectangular seam where a step size changed.
  const reach = Math.max(plateauPad * 2.4, step * 3);
  const grainAt = ([x, y]) =>
    step * 0.30 * (1 + (plateauGrain - 1) * (1 - smooth(clamp(nearRect(x, y) / reach, 0, 1))));
  for (let y = 0; y < H; y += step)
    for (let x = 0; x < W; x += step) {
      const px = x + rand(r, -step * 0.26, step * 0.26);
      const py = y + rand(r, -step * 0.26, step * 0.26);
      if (polyDist(px, py, poly) < -step * 0.35) pts.push([px, py]);
    }
  for (const P2 of [poly, ...flatPolys])
    for (let i = 0; i < P2.length; i++) {
      const a = P2[i], b = P2[(i + 1) % P2.length];
      const k = Math.max(2, Math.floor(Math.hypot(b[0]-a[0], b[1]-a[1]) / (step*0.55)) + 1);
      for (let j = 0; j < k; j++)
        pts.push([a[0] + (b[0]-a[0]) * j/k, a[1] + (b[1]-a[1]) * j/k]);
    }
  // Assigned rather than spread: a fine tessellation runs to hundreds of thousands
  // of points, and push(...arr) overflows the stack long before that.
  const filtered = spaceOut(pts, step * 0.30 * Math.max(1, plateauGrain), grainAt);
  pts.length = filtered.length;
  for (let i = 0; i < filtered.length; i++) pts[i] = filtered[i];

  const ramp = Math.max(density * 1.1, Math.min(W - gw, H - gh) * 0.75) * rampFactor;
  const flatIdx = [], wts = [], edgeT = [], ripple = [], tiltOff = [], flatU = [];
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
      // How far across the plateau this vertex sits: 0 on the text box, 1 at the
      // plateau's rim. It is what lets the top be a dome with a table on it
      // instead of one flat lid.
      const toRim = -ds[inside];
      flatU[i] = bd + toRim > 1e-6 ? clamp(bd / (bd + toRim), 0, 1) : 0;
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

  return { pts, tris, flatIdx, wts, edgeT, ripple, tiltOff, flatU, origin, dn,
           nRects: local.length,
           W, H, x: gx0 + x0, y: gy0 + y0,
           poly: poly.map(([px, py]) => [px + gx0 + x0, py + gy0 + y0]),
           plateaus: flatPolys.map(fp => fp.map(([px, py]) => [px + gx0 + x0, py + gy0 + y0])) };
}

/**
 * Vertex heights of a fully grown shard, plus the deepest plateau elevation.
 * `depth` is one number or one per rectangle.
 */
export function shardHeights(g, opts = {}) {
  const { depth = 150, flatTilt = 0, plateauDome = 0.55 } = opts;
  const dArr = Array.isArray(depth) ? Array.from({length: g.nRects}, (_, i) => depth[i] ?? depth[0])
                                    : Array.from({length: g.nRects}, () => depth);
  const dMax = Math.max(...dArr, 1e-6);
  const Z = new Float64Array(g.pts.length);
  for (let i = 0; i < g.pts.length; i++) {
    if (g.flatIdx[i] >= 0) {
      // A spherical cap over the plateau: flat where the text lies, falling away
      // towards the rim, so the top reads as a rounded stone and not as a lid.
      const u = g.flatU[i] ?? 0;
      const cap = 1 - plateauDome * (1 - Math.sqrt(Math.max(0, 1 - u * u)));
      // The table stays flat; the tilt that gives faces their steps of lightness
      // only comes in on the slopes, where the small faceting belongs.
      Z[i] = dArr[g.flatIdx[i]] * cap + g.tiltOff[i] * flatTilt * smooth(u);
      continue;
    }
    const w = g.wts[i];
    let wsum = 0, dsum = 0;
    for (let k = 0; k < w.length; k++) { wsum += w[k]; dsum += w[k] * dArr[k]; }
    const target = wsum > 1e-6 ? dsum / wsum : dMax;
    const t = clamp(wsum, 0, 1) * g.edgeT[i];
    Z[i] = target * t - dMax * 0.18 * (1 - t) + g.ripple[i] * t * (1 - t) * 2;
  }
  return { Z, dMax };
}

/** Shades prepared geometry into SVG. Cheap enough to call every frame. */
export function renderShard(g, opts = {}) {
  const { depth = 150, hue = 266, floor = 7, flatTilt = 0, sweep = 0, sweepScale = 1.0,
          flatHueJitter = 5, flatSatJitter = 5, flatLigJitter = 0,
          grow = 1, stagger = 0.5 } = opts;

  const { Z: Zfull, dMax } = shardHeights(g, { depth, flatTilt });
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
  for (let i = 0; i < P.length; i++)
    Z[i] = g.flatIdx[i] >= 0 ? Zfull[i] : Zfull[i] * growF[i];

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

function hash2(i, j, seed) {
  let n = (i * 374761393 + j * 668265263 + seed * 1442695040888963407) | 0;
  n = (n ^ (n >> 13)) * 1274126177;
  return (n ^ (n >> 16)) >>> 0;
}

/**
 * Cellular noise, and the three things a broken surface asks of it: how far a point
 * lies into its block, how close it is to the seam with the next, and which block it
 * is in. A sum of octaves can answer none of the last two — it has no blocks and no
 * seams, which is why smooth noise reads as cloth however it is folded.
 */
function cells(x, y, seed, scale) {
  const fx = x / scale, fy = y / scale;
  const ix = Math.floor(fx), iy = Math.floor(fy);
  let f1 = 1e9, f2 = 1e9, own = 0;
  for (let j = -1; j <= 1; j++)
    for (let i = -1; i <= 1; i++) {
      const gx = ix + i, gy = iy + j;
      const h = hash2(gx, gy, seed);
      const dx = gx + (h & 1023) / 1024 - fx;
      const dy = gy + ((h >>> 10) & 1023) / 1024 - fy;
      const d = Math.sqrt(dx * dx + dy * dy);
      if (d < f1) { f2 = f1; f1 = d; own = ((h >>> 20) & 4095) / 4095; }
      else if (d < f2) { f2 = d; }
    }
  return [f1, f2, own];
}

/**
 * Folded noise. A sum of octaves settles into round hills because every octave is
 * smooth through its own zero; folding at the zero puts a crease there instead, and
 * weighting each octave by the one above carves the creases into ridges and gullies
 * that run, rather than blobs that sit.
 */
function ridged(x, y, seed, oct = 4, scale = 900) {
  let a = 1, f = 1 / scale, sum = 0, norm = 0, prev = 1;
  for (let o = 0; o < oct; o++) {
    const n = 1 - Math.abs(2 * noise2(x * f, y * f, seed + o * 131) - 1);
    sum += a * n * n * prev;
    norm += a;
    prev = n;
    a *= 0.52; f *= 2.13;
  }
  return sum / norm;
}

/** Matrix rock the outcrops grow out of. `sockets` are outcrop contours in page coordinates. */
export function prepareMatrix(W, H, sockets, opts = {}) {
  const { seed = 3, step: step0 = 300, lo = 4.5, hi = 19, hue = 264, relief: relief0 = 52,
          swell: swell0 = 45, dome: dome0 = 70, sizeVar = 0.62, rockRidge = 0.55,
          rockFracture = 0.7, fractureSize: fractureSize0 = 260, rockRim = 0.5,
          // Every length below is in page pixels, and the page has no fixed size.
          // Left alone, a wider window does not show a bigger stone in more rock — it
          // shows more of the same small one, until the nests tile it like wallpaper
          // and the triangle count climbs with the area. Measured against a reference
          // window, the picture keeps its proportions and its cost. Never below one:
          // a small window should not be given a coarser world, only a smaller view
          // of the same one.
          sceneRef = 1920 * 1080,
          stoneShare = 0.35, stoneSize: stoneSize0 = 26, stoneRise = 0.9, stoneCrown = 0.42,
          intergrow = 0, stoneLean = 0, stoneSink = 0.08,
          nestCrust = false, crustHeight = 0.45, crustSpread = 0.5, crustFacet = 0.3,
          crustBudget = 500,
          clusterSize: clusterSize0 = 90, clusterCount = 5, clusterSpread = 2.4,
          // How much of the rock itself is crystalline, beyond the chips: 0 leaves
          // it stone, 1 turns the whole field to crystal.
          crystalField = 0,
          // Where stones must not be seeded. Defaults to the outcrops themselves;
          // pass the plateaux to keep the ground clear only under the thick body
          // and still have stones lying along the skirt.
          keepOut = sockets } = opts;
  const r = rng(seed);
  const pts = [];

  const scene = Math.max(1, Math.sqrt((W * H) / Math.max(1, sceneRef)));
  const step = step0 * scene;
  const relief = relief0 * scene;
  const dome = dome0 * scene;
  const fractureSize = fractureSize0 * scene;
  const stoneSize = stoneSize0 * scene;
  const clusterSize = clusterSize0 * scene;
  // Kept out of the mesh and re-added by the renderer each frame, so whoever moves it
  // there has to apply the same factor or the two drift apart.
  const swell = swell0 * scene;

  // Candidates are laid finer than the grain wants and thinned by a spacing that
  // follows the density field. Dropping one to three points into each cell of a
  // lattice instead leaves the lattice visible and pairs of points close enough to
  // triangulate into slivers.
  const dense = step * 0.42;
  for (let y = -step; y < H + step; y += dense)
    for (let x = -step; x < W + step; x += dense)
      pts.push([x + rand(r, -dense * 0.55, dense * 0.55),
                y + rand(r, -dense * 0.55, dense * 0.55)]);

  const grainAt = ([x, y]) => {
    const d = fbm(x, y, seed + 500, 3, 1400);
    return step * 0.34 * (1 - sizeVar * 0.5 + sizeVar * (1 - d));
  };
  const thinned = spaceOut(pts, step * 0.34 * (1 + sizeVar * 0.5), grainAt);
  pts.length = thinned.length;
  for (let i = 0; i < thinned.length; i++) pts[i] = thinned[i];

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

  // Same field as the first pass: a uniform radius here would flatten the grain
  // back out and undo it.
  const kept = spaceOut(pts, step * 0.34 * (1 + sizeVar * 0.5), grainAt);
  pts.length = kept.length;
  for (let i = 0; i < kept.length; i++) pts[i] = kept[i];

  // The rock is triangulated on its own points and nothing else. A stone seeded into
  // this set drags the grain around it into slivers reaching for its ring, which is
  // the tessellation the rock gets judged by.
  const socket = [];
  const ground = (x, y) => {
    // The heave around the outcrop is kept as a weight as well as baked in, so a
    // renderer that can move it per frame has the shape to move. Read first: how
    // near the outcrop a point is decides how broken it gets.
    let w = 0;
    for (const poly of sockets)
      w = Math.max(w, smooth(clamp(1 - Math.max(polyDist(x, y, poly), 0) / 260, 0, 1)));

    let z = relief * (fbm(x, y, seed, 5, 1100) - 0.5)
          + relief * 0.45 * (fbm(x, y, seed + 91, 3, 320) - 0.5);
    // An octave the width of one facet. Without it a finer grain only interpolates
    // the same two long waves more times: more polygons, and a smoother surface for
    // it, because neighbouring faces end up holding the same normal.
    z += relief * 0.30 * (fbm(x, y, seed + 217, 2, Math.max(70, step * 0.85)) - 0.5);

    // Wall, not landscape. Blocks each sitting at their own level, parted by the
    // seams they broke along, and more broken the nearer they are to what came
    // through. Two sizes, because a block that broke is not one clean slab either.
    if (rockFracture > 0) {
      const near = rockFracture * (1 + 1.6 * w);
      const [f1, f2, own] = cells(x, y, seed + 811, fractureSize);
      z += relief * near * (own - 0.5) * 1.15;
      z -= relief * near * 0.85 * Math.exp(-Math.pow((f2 - f1) / 0.11, 2));
      const [g1, g2, gown] = cells(x, y, seed + 1213, fractureSize * 0.34);
      z += relief * near * 0.40 * (gown - 0.5);
      z -= relief * near * 0.28 * Math.exp(-Math.pow((g2 - g1) / 0.16, 2));
    }
    // The lip of the socket: rock heaved and split along the line the body came out
    // of, rather than a rim smoothly swelling out of nowhere.
    if (rockRim > 0)
      z += relief * rockRim * Math.exp(-Math.pow((w - 0.5) / 0.19, 2))
         * (0.7 + 1.1 * (cells(x, y, seed + 1601, fractureSize * 0.55)[2] - 0.5));

    if (rockRidge > 0)
      z += relief * rockRidge *
           ((ridged(x, y, seed + 401, 4, 640) - 0.42) * 1.5
          + (ridged(x, y, seed + 733, 3, Math.max(90, step * 1.6)) - 0.42) * 0.8);

    // A spherical cap over the page: the ground reads as a body bulging towards
    // the viewer rather than as a hollow seen into.
    const dx = (x - W/2) / (W/2 || 1), dy = (y - H/2) / (H/2 || 1);
    z += dome * Math.sqrt(Math.max(0, 1 - Math.min(1, dx*dx + dy*dy)));
    return [z + swell * w, w];
  };

  const Z = pts.map(([x, y], i) => {
    const [z, w] = ground(x, y);
    socket[i] = w;
    return z;
  });

  const del = new Delaunator(Float64Array.from(pts.flat()));
  const faces = [];
  for (let i = 0; i < del.triangles.length; i += 3)
    faces.push([del.triangles[i], del.triangles[i + 1], del.triangles[i + 2],
                crystalField > 0 && ((i * 2654435761) >>> 0) / 4294967296 < crystalField]);

  // Stones are bodies standing on that rock, wired by hand: a ring in the ground, a
  // crown short of the point, and the point. Their faces are known to be stone as
  // they are made, which is also the end of guessing it back from which vertices a
  // triangle happens to touch.
  const stoneApex = new Array(pts.length).fill(0);
  const stones = [];
  const nest = [];
  if (stoneShare > 0 && stoneSize > 0) {
    const seedStone = (cx, cy) => {
      if (cx < 0 || cy < 0 || cx > W || cy > H) return;
      // Not under the thick body: a stone seeded there sits inside it and reads
      // as a blot rather than as something lying in the ground.
      if (keepOut.some(poly => polyDist(cx, cy, poly) < stoneSize * 0.9)) return;
      // Crystals in a druse grow into one another: they share ground, they
      // interpenetrate, and the taller ones take the light off the rest. Stones used
      // to be kept apart because rings overlapping inside one shared triangulation
      // fused into chains — they carry their own meshes now, and what crosses is
      // sorted by depth like any other pair of solids.
      const crowd = 0.85 * (1 - 0.92 * clamp(intergrow, 0, 1));
      for (const st of stones)
        if (Math.hypot(cx - st.cx, cy - st.cy) < (stoneSize + st.rad) * crowd) return;

      const rad = stoneSize * rand(r, 0.6, 1.35);
      // Quartz grows a six-sided prism closed by two rhombohedra, so the head is six
      // triangles in alternating sizes rather than a ring of equal ones. That
      // alternation is what the eye reads as quartz and not as a spike.
      const n = 6;
      const phase = r() * Math.PI * 2;
      const rise = rad * stoneRise * rand(r, 0.7, 1.4);
      const [base, w] = ground(cx, cy);
      const me = { cx, cy, rad, rise, base, w };
      stones.push(me);
      nest.push(me);

      // A druse does not grow straight up. Crystals start on the wall of a cavity
      // and lean off it, and a crowd of them points every way at once. Standing
      // them all on one axis is what makes intergrowth read as nested cones rather
      // than as a nest.
      const dir = r() * Math.PI * 2;
      const tilt = rise * stoneLean * Math.sqrt(r());
      const lx = Math.cos(dir) * tilt, ly = Math.sin(dir) * tilt;

      const vert = (x, y, lift) => {
        pts.push([x, y]);
        Z.push(base + lift);
        stoneApex.push(Math.max(0, lift));
        socket.push(w);
        return pts.length - 1;
      };

      // The ring sits under the surface, and how far under is the difference between
      // a crystal and a pebble laid on the ground. Two that grew into each other meet
      // along a line running down their sides; buried, that line is buried with it and
      // only the heads stand out — which is a sprout. Left at the surface, the line is
      // in full view and reads as one mesh sitting inside another.
      const ring = [], crown = [];
      for (let i = 0; i < n; i++) {
        const a = phase + (i / n) * Math.PI * 2;
        const rr = rad * rand(r, 0.94, 1.07);
        ring.push(vert(cx + Math.cos(a) * rr, cy + Math.sin(a) * rr, -rise * stoneSink));
      }
      // A shoulder short of the point. Ring straight to apex is a fan of faces that
      // all lean the same way — a cone, taking light as one surface. The break puts
      // a small crown on top standing at an angle the sides do not have.
      const cr = clamp(stoneCrown, 0.06, 0.9);
      for (let i = 0; i < n; i++) {
        const a = phase + (i / n) * Math.PI * 2;
        const alt = i % 2 ? 0.64 : 1;
        const rr = rad * cr * alt * rand(r, 0.94, 1.07);
        const lift = i % 2 ? 0.72 : 0.56;
        crown.push(vert(cx + Math.cos(a) * rr + lx * lift,
                        cy + Math.sin(a) * rr + ly * lift, rise * lift));
      }
      // A termination meets on the axis.
      const tip = vert(cx + lx, cy + ly, rise);

      for (let i = 0; i < n; i++) {
        const j = (i + 1) % n;
        faces.push([ring[i], ring[j], crown[j], true]);
        faces.push([ring[i], crown[j], crown[i], true]);
        faces.push([crown[i], crown[j], tip, true]);
      }
    };

    /**
     * One crust under a whole nest. Not a stone joined to a stone: a single height
     * field taking the highest cap at every point, triangulated once. Between two
     * crystals there is then no boundary to draw, because there are not two surfaces
     * — the pad runs from one into the other and buries the feet, the rings and every
     * line where the bodies cross, and what stands above it is heads.
     */
    const emitCrust = () => {
      if (nest.length < 2) return;
      // Scattered and triangulated on its own points, not drawn as a fan from the
      // middle: a fan gives petals, and a smooth dome over the nest gives a pillow
      // facing straight into the light. The crust of a druse is itself crystal, so
      // it wants faces — the height carries a jitter of its own, and each triangle
      // takes its own angle.
      let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
      for (const s of nest) {
        const rr = s.rad * (1.1 + 1.3 * crustSpread);
        x0 = Math.min(x0, s.cx - rr); x1 = Math.max(x1, s.cx + rr);
        y0 = Math.min(y0, s.cy - rr); y1 = Math.max(y1, s.cy + rr);
      }
      // A facet the size of the crystals is the right look, and on a wide nest it is
      // also thousands of triangles that go on being drawn every frame. The step is
      // whichever is coarser: that size, or what fits the budget over this nest's own
      // area — so a nest costs about the same whether it is small or spread out.
      const grid = Math.max(3, stoneSize * 0.38,
                            Math.sqrt((x1 - x0) * (y1 - y0) / crustBudget));
      // Squared radii, kept beside the nest rather than recomputed per sample. This
      // runs over the whole bounding box of every nest on the page, and the two
      // square roots per stone per point were most of what it cost.
      const reach2 = new Float64Array(nest.length);
      const edge2 = new Float64Array(nest.length);
      const cap = new Float64Array(nest.length);
      for (let i = 0; i < nest.length; i++) {
        const s = nest[i];
        const rr = s.rad * (1.1 + 1.3 * crustSpread);
        reach2[i] = rr * rr;
        edge2[i] = (rr + grid * 1.2) * (rr + grid * 1.2);
        cap[i] = s.rise * crustHeight;
      }

      // How far the crust stands above the rock here, and whether the point is close
      // enough to a cap to be worth keeping at all. Zero lift outside every cap, so
      // the pad rides the relief it lies on instead of flattening it. One ring of
      // flat points beyond them gives it an edge to sit on.
      let near = false;
      const liftAt = (x, y) => {
        let lift = 0;
        near = false;
        for (let i = 0; i < nest.length; i++) {
          const dx = x - nest[i].cx, dy = y - nest[i].cy;
          const d2 = dx * dx + dy * dy;
          if (d2 >= edge2[i]) continue;
          near = true;
          if (d2 >= reach2[i]) continue;
          const h = cap[i] * (1 - d2 / reach2[i]);
          if (h > lift) lift = h;
        }
        return lift;
      };

      // The rock under a nest, read on a lattice three steps wide and interpolated
      // between. Asking ground() per sample means five octaves of fBm, three walks
      // of the cellular field, two folded ones and a distance to the plateau contour
      // — for every point of every crust on the page, which is where the whole cost
      // of this was. The rock has no feature that fine to lose.
      const gx0 = x0 - grid * 2, gy0 = y0 - grid * 2;
      const gstep = grid * 3;
      const gcols = Math.ceil((x1 - x0 + grid * 4) / gstep) + 2;
      const grows = Math.ceil((y1 - y0 + grid * 4) / gstep) + 2;
      const gz = new Float64Array(gcols * grows);
      const gwt = new Float64Array(gcols * grows);
      for (let j = 0; j < grows; j++)
        for (let i = 0; i < gcols; i++) {
          const [z, w] = ground(gx0 + i * gstep, gy0 + j * gstep);
          gz[j * gcols + i] = z;
          gwt[j * gcols + i] = w;
        }
      const groundAt = (x, y) => {
        const u = (x - gx0) / gstep, v = (y - gy0) / gstep;
        const i = Math.max(0, Math.min(gcols - 2, Math.floor(u)));
        const j = Math.max(0, Math.min(grows - 2, Math.floor(v)));
        const fu = u - i, fv = v - j;
        const k = j * gcols + i;
        const lerp = a =>
          (a[k] * (1 - fu) + a[k + 1] * fu) * (1 - fv)
        + (a[k + gcols] * (1 - fu) + a[k + gcols + 1] * fu) * fv;
        return [lerp(gz), lerp(gwt)];
      };

      const local = [], idx = [], lifts = [];
      for (let y = y0 - grid; y <= y1 + grid; y += grid)
        for (let x = x0 - grid; x <= x1 + grid; x += grid) {
          const px = x + rand(r, -grid * 0.42, grid * 0.42);
          const py = y + rand(r, -grid * 0.42, grid * 0.42);
          const lift = liftAt(px, py);
          if (!near) continue;
          const [gz0, gw] = groundAt(px, py);
          const rough = lift > 0 ? rand(r, -1, 1) * grid * crustFacet : 0;
          local.push([px, py]);
          lifts.push(lift);
          pts.push([px, py]);
          Z.push(gz0 + lift + rough);
          socket.push(gw);
          stoneApex.push(Math.max(0, lift + rough));
          idx.push(pts.length - 1);
        }
      if (local.length < 3) return;

      const dl = new Delaunator(Float64Array.from(local.flat()));
      for (let i = 0; i < dl.triangles.length; i += 3) {
        const [a, b, c] = [dl.triangles[i], dl.triangles[i + 1], dl.triangles[i + 2]];
        // Only where the crust actually stands: the scatter reaches past the caps to
        // give the pad an edge, and triangulating that margin would tile the whole
        // bounding box in crystal. Read off the lift already computed for each point,
        // which is the same answer as asking the field again at the centre.
        if (lifts[a] <= 0 && lifts[b] <= 0 && lifts[c] <= 0) continue;
        faces.push([idx[a], idx[b], idx[c], true]);
      }
    };

    // Crystal grows in pockets, not evenly over a field. Cluster centres are seeded
    // first and the stones are placed around them, so there is bare rock between
    // the nests instead of one uniform scatter.
    const cell = stoneSize * 3.2 * clusterSpread;
    for (let gy = 0; gy < H / cell + 1; gy++)
      for (let gx = 0; gx < W / cell + 1; gx++) {
        if (r() > stoneShare) continue;
        const cx = (gx + rand(r, 0.15, 0.85)) * cell;
        const cy = (gy + rand(r, 0.15, 0.85)) * cell;
        const count = 1 + Math.floor(r() * Math.max(1, clusterCount));
        nest.length = 0;
        for (let k = 0; k < count; k++) {
          const a = r() * Math.PI * 2;
          const d = clusterSize * Math.sqrt(r());
          seedStone(cx + Math.cos(a) * d, cy + Math.sin(a) * d);
        }
        if (nestCrust) emitCrust();
      }
  }

  // Spread over an argument list this overflows the stack once the stones have put
  // their own vertices in.
  let zmin = Infinity, zmax = -Infinity;
  for (const z of Z) { if (z < zmin) zmin = z; if (z > zmax) zmax = z; }
  // The tallest chip on the page: what a shader needs to turn a thickness in page
  // units into how far up its own stone a pixel sits.
  let apexMax = 0;
  for (const a of stoneApex) if (a > apexMax) apexMax = a;

  const tri = [];
  for (let i = 0; i < faces.length; i++) {
    const [a, b, c, isStone] = faces[i];
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
    tri.push({ a, b, c, cz, sh, stone: isStone, jit: rand(r, -6, 6) });
  }
  tri.sort((p, q) => p.cz - q.cz);
  return { pts, Z, socket, swell, apex: stoneApex, apexMax, tris: tri, stones,
           zmin, zmax, W, H, lo, hi, hue };
}

/** Serialises a prepared matrix into SVG. Used by the CPU renderer. */
export function buildMatrix(W, H, sockets, opts = {}) {
  const m = prepareMatrix(W, H, sockets, opts);
  const { pts, tris, lo, hi, hue } = m;
  const out = [`<svg xmlns="${NS}" viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" aria-hidden="true" focusable="false">`,
               `<rect width="${W}" height="${H}" fill="${hsl(266,34,lo)}"/>`];
  for (const t of tris) {
    const p = [t.a,t.b,t.c].map(k => `${pts[k][0].toFixed(0)},${pts[k][1].toFixed(0)}`).join(' ');
    const col = hsl(hue + 10*t.sh + t.jit, 32 + 16*(1-t.sh),
                    lo + (hi-lo)*Math.pow(t.sh,1.5));
    out.push(`<polygon points="${p}" fill="${col}" stroke="${col}" stroke-width="0.7"/>`);
  }
  out.push('</svg>');
  return { svg: out.join(''), faces: tris.length };
}

/**
 * Measures the real boxes of `groups` inside `container` and draws stone under them.
 * Returns a destroy function carrying `update`, `redraw`, `rise` and `freeze`.
 * Markup drives geometry: `data-elev` sets a box's own elevation, `data-pad` pads
 * the measured box.
 */
export function mountCrystals({ container, groups, matrix = true, spread = 0.14,
                                seed = 500, pad = 0, plateauPad = 56, plateauRound = 1,
                                shapeRound = 0.8, plateauGrain = 2.4,
                                hue = 266, floor = 7,
                                flatHueJitter = 5, flatSatJitter = 5,
                                flatLigJitter = 0, flatTilt = 0, sweep = 0,
                                apron = 0, rampFactor = 1.0, depth = 150,
                                animateIn = 0, easing = 'inOut', stagger = 0.5,
                                onDone, onGrow } = {}) {
  if (!container) return () => {};
  const layer = document.createElement('div');
  layer.setAttribute('aria-hidden', 'true');
  Object.assign(layer.style, { position: 'absolute', inset: '0', zIndex: '0',
                               pointerEvents: 'none', overflow: 'hidden' });
  if (getComputedStyle(container).position === 'static') container.style.position = 'relative';
  container.prepend(layer);

  const cfg = { spread, plateauPad, plateauRound, shapeRound, plateauGrain,
                hue, floor, flatHueJitter, flatSatJitter,
                flatLigJitter, flatTilt, sweep, apron, rampFactor, depth, matrix,
                stagger };
  const GEOM_KEYS = ['spread', 'plateauPad', 'plateauRound', 'shapeRound', 'plateauGrain',
                     'apron', 'rampFactor'];

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
    onGrow?.(k);
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
