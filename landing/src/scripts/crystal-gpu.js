import { prepareShard, shardHeights, prepareMatrix } from './crystal.js';

const CRYSTAL_FLOATS = 8;
const ROCK_FLOATS = 7;
const UNIFORM_BYTES = 240;
const ROCK_FORMAT = 'rgba8unorm';

const SHADER = /* wgsl */`
struct U {
  res: vec2f,
  grow: f32,
  stagger: f32,
  pointer: vec2f,
  time: f32,
  zMax: f32,
  absorb: vec3f,
  ior: f32,
  lightDir: vec3f,
  dispersion: f32,
  tint: vec3f,
  spec: f32,
  shininess: f32,
  refractScale: f32,
  glow: f32,
  bevel: f32,
  rockLow: vec3f,
  rockMin: f32,
  rockHigh: vec3f,
  rockSpan: f32,
  lightPower: f32,
  stoneGlow: f32,
  scatter: f32,
  veil: f32,
  edge: f32,
  eyeDist: f32,
  stoneDensity: f32,
  rockSwellNow: f32,
  veilFlash: f32,
  innerFacets: f32,
  core: f32,
  pad0: f32,
  eye: vec2f,
  stoneSpec: f32,
  stoneDeep: f32,
  stoneShine: f32,
  facetVary: f32,
  facetBase: f32,
  aoPower: f32,
  aoRadius: f32,
  aoRange: f32,
  rockDetail: f32,
  rockDetailScale: f32,
  stoneZoning: f32,
  stoneRiseRef: f32,
  aoBias: f32,
  pad4: f32,
};

// An orthographic eye sees no displacement through a flat top: the ray goes
// straight down and the rock behind stays exactly where it was, which is what
// makes a thick body read as a tinted film. A real eye is at a finite distance,
// and it is where the pointer is — so refraction, fresnel and the mirror all
// shift together as it moves, the way they would if the stone were being turned.
fn viewDir(world: vec3f) -> vec3f {
  return normalize(vec3f(u.eye, u.eyeDist) - world);
}

@group(0) @binding(0) var<uniform> u: U;

fn clipOf(p: vec2f, z: f32) -> vec4f {
  let x = p.x / u.res.x * 2.0 - 1.0;
  let y = 1.0 - p.y / u.res.y * 2.0;
  let d = 0.5 - clamp(z / u.zMax, -1.0, 1.0) * 0.5;
  return vec4f(x, y, clamp(d, 0.0001, 0.9999), 1.0);
}

fn hash3(c: vec3f) -> f32 {
  let p = fract(c * 0.3183099 + vec3f(0.1, 0.2, 0.3));
  let q = p * 17.0;
  return fract(q.x * q.y * q.z * (q.x + q.y + q.z));
}

/** Value noise on a lattice: irregular by construction, unlike a sum of sines. */
fn noise3(x: vec3f) -> f32 {
  let i = floor(x);
  let f = x - i;
  let w = f * f * (3.0 - 2.0 * f);
  let a = mix(mix(mix(hash3(i + vec3f(0.0, 0.0, 0.0)), hash3(i + vec3f(1.0, 0.0, 0.0)), w.x),
                  mix(hash3(i + vec3f(0.0, 1.0, 0.0)), hash3(i + vec3f(1.0, 1.0, 0.0)), w.x), w.y),
              mix(mix(hash3(i + vec3f(0.0, 0.0, 1.0)), hash3(i + vec3f(1.0, 0.0, 1.0)), w.x),
                  mix(hash3(i + vec3f(0.0, 1.0, 1.0)), hash3(i + vec3f(1.0, 1.0, 1.0)), w.x), w.y),
              w.z);
  return a;
}

// The same fold the geometry uses, at a size no triangle could carry. Two octaves:
// the third costs eight more lattice corners for detail this is already past.
fn rockBump(p: vec2f) -> f32 {
  var s = 0.0;
  var a = 1.0;
  var norm = 0.0;
  var q = p;
  for (var o = 0; o < 2; o = o + 1) {
    let n = 1.0 - abs(2.0 * noise3(vec3f(q, 3.7)) - 1.0);
    s = s + a * n * n;
    norm = norm + a;
    a = a * 0.5;
    q = q * 2.17;
  }
  return s / norm;
}

/**
 * Detail the mesh is too coarse to hold, added to the normal rather than to the
 * height: the surface answers the light per pixel, and a facet stops being one
 * flat tone across its whole width.
 */
fn bumped(n: vec3f, world: vec3f) -> vec3f {
  if (u.rockDetail <= 0.0) { return n; }
  let p = world.xy / max(u.rockDetailScale, 1.0);
  let h = rockBump(p);
  let e = 0.35;
  let g = vec2f(rockBump(p + vec2f(e, 0.0)) - h, rockBump(p + vec2f(0.0, e)) - h);
  return normalize(n - vec3f(g, 0.0) * u.rockDetail);
}

fn faceNormal(world: vec3f, jit: f32, bevel: f32) -> vec3f {
  var n = normalize(cross(dpdx(world), dpdy(world)));
  if (n.z < 0.0) { n = -n; }
  return normalize(n + vec3f(jit - 0.5, fract(jit * 7.0) - 0.5, 0.0) * bevel);
}

// One moving light for everything on the page, so rock and stone answer it together.
fn glintOf(world: vec3f, n: vec3f, shininess: f32) -> f32 {
  let l = normalize(vec3f(u.pointer, u.zMax * 2.2) - world);
  let h = normalize(l + viewDir(world));
  return pow(max(dot(n, h), 0.0), shininess);
}

/**
 * How much a face reflects at the angle the highlight actually happens at.
 * Schlick on the half vector, not on the normal: this is the microfacet form, and
 * it is what keeps a highlight reading as a reflection on something transparent
 * rather than as an opaque film laid over it.
 */
fn glintFresnel(world: vec3f, f0: f32) -> f32 {
  let v = viewDir(world);
  let l = normalize(vec3f(u.pointer, u.zMax * 2.2) - world);
  let h = normalize(l + v);
  let vdh = clamp(dot(v, h), 0.0, 1.0);
  return f0 + (1.0 - f0) * pow(1.0 - vdh, 5.0);
}

struct RockOut {
  @builtin(position) pos: vec4f,
  @location(0) world: vec3f,
  @location(1) @interpolate(flat) jit: f32,
  @location(2) @interpolate(flat) stone: f32,
  @location(3) thick: f32,
};

@vertex
fn rockVert(@location(0) p: vec2f, @location(1) z: f32,
            @location(2) jit: f32, @location(3) stone: f32,
            @location(4) socket: f32, @location(5) thick: f32) -> RockOut {
  // The ground heaves around the outcrop, and the heave is a uniform, so it can
  // rise with the stone instead of standing baked in the mesh.
  let zz = z + u.rockSwellNow * socket;
  var out: RockOut;
  out.pos = clipOf(p, zz);
  out.world = vec3f(p, zz);
  out.jit = jit;
  out.stone = stone;
  out.thick = thick;
  return out;
}

@fragment
fn rockFrag(in: RockOut) -> @location(0) vec4f {
  let n = bumped(faceNormal(in.world, in.jit, u.bevel * 0.5), in.world);
  let key = max(dot(n, normalize(u.lightDir)), 0.0) * u.lightPower;
  let height = clamp((in.world.z - u.rockMin) / max(u.rockSpan, 1e-3), 0.0, 1.0);
  let sh = clamp(0.38 * height + 0.62 * pow(key, 1.9), 0.0, 1.0);
  // A floor of colour. Left to bottom out at the rock's darkest value, faces turned
  // away from the light read as holes cut in the page rather than as unlit stone.
  let base = mix(u.rockLow, u.rockHigh, pow(sh, 1.5)) + u.tint * 0.045;

  // Rock is rough: it takes the same light and returns a broad dull sheen.
  let sheen = glintOf(in.world, n, 12.0) * u.spec * 0.18;
  // Alpha carries the rock's own height, so the stone above can measure how far
  // light actually travels through it instead of assuming a flat floor at zero.
  if (in.stone < 0.5) {
    return vec4f(base + vec3f(sheen), height);
  }

  // A chip is ten pixels across. Absorption, refraction and a mirror term are not
  // legible at that size — what is legible is that it is violet, that its facets
  // step against each other, and that the highlight belongs to a hard surface.
  // So: amethyst colour stepped by facet, deepened where the chip is thick, and a
  // fresnel-weighted glint, which is what a white one added flat is not.
  let t = max(in.thick, 0.0);
  let deep = exp(-u.absorb * u.stoneDensity * (t * u.stoneDeep) * 0.002);
  // Amethyst is zoned along its own growth: the colour gathers in the head and the
  // foot stays pale, often nearly white. Uniform violet from ring to point is the
  // one thing that reads as plastic however well the facets are lit.
  let zone = clamp(t / max(u.stoneRiseRef, 1e-3), 0.0, 1.0);
  // Massive quartz, not white paint: the foot of a crystal is duller and darker than
  // its head, not brighter. Taken as a pale grey it lights up under the glow the
  // heads are tuned for, and a crust of it burns the frame out.
  let foot = u.tint * 0.40 + vec3f(0.05, 0.045, 0.065);
  let zoned = mix(u.tint, mix(foot, u.tint, smoothstep(0.12, 0.88, zone)), u.stoneZoning);
  let facet = zoned * u.stoneGlow * (0.42 + 0.58 * key);
  let body = mix(facet, facet * deep * 1.7, 0.5);
  let spark = glintOf(in.world, n, u.stoneShine) * u.stoneSpec
            * glintFresnel(in.world, 0.08);
  return vec4f(mix(base, body, 0.86) + vec3f(spark), height);
}

struct BgOut { @builtin(position) pos: vec4f, @location(0) uv: vec2f };

@vertex
fn bgVert(@builtin(vertex_index) i: u32) -> BgOut {
  var xy = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  var out: BgOut;
  let p = xy[i];
  out.pos = vec4f(p, 0.9999, 1.0);
  out.uv = vec2f((p.x + 1.0) * 0.5, 1.0 - (p.y + 1.0) * 0.5);
  return out;
}

@group(1) @binding(0) var samp: sampler;
@group(1) @binding(1) var rockTex: texture_2d<f32>;

@fragment
fn bgFrag(in: BgOut) -> @location(0) vec4f {
  // Alpha carries the rock height and must survive into the mip levels; on screen
  // the canvas is opaque, so passing it through costs nothing there.
  return textureSampleLevel(rockTex, samp, in.uv, 0.0);
}

// The same blit, with the occlusion the rock owes itself. Kept apart from bgFrag
// because that one also builds the mip chain, and darkening applied once per level
// compounds into soot.
@fragment
fn screenFrag(in: BgOut) -> @location(0) vec4f {
  let c = textureSampleLevel(rockTex, samp, in.uv, 0.0);
  if (u.aoPower <= 0.0) { return c; }

  // Ambient occlusion read off the height the rock pass left in alpha. Every
  // neighbour standing above this pixel takes a share of its sky; how much they
  // take together is how deep in a crease it sits. Two rings, so a chip darkens
  // where it meets the ground and not merely along its own outline.
  //
  // Measured against the blurred field, not against the raw height. That alpha
  // carries the dome and the heave as well as the chips, and on a slope every
  // uphill neighbour stands higher — plain differences would shade the whole
  // upward side of the rock and leave a ten-pixel stone, a hundredth of the span,
  // invisible. A coarse mip is the surface those chips are standing on.
  // The smooth surface is read once, not per tap. Over a radius of a dozen pixels
  // a mip that coarse barely moves, and sampling it sixteen more times buys a
  // difference below the eighth bit at twice the bandwidth.
  let dim = vec2f(textureDimensions(rockTex, 0));
  let lod = log2(max(u.aoRadius, 2.0)) + 1.0;
  let flat = textureSampleLevel(rockTex, samp, in.uv, lod).a;
  let mine = c.a - flat;
  var occ = 0.0;
  var wsum = 0.0;
  for (var ring = 0; ring < 2; ring = ring + 1) {
    let rad = u.aoRadius * (0.45 + 0.75 * f32(ring));
    let w = 1.0 / (1.0 + f32(ring));
    for (var i = 0; i < 6; i = i + 1) {
      let a = (f32(i) + 0.5 * f32(ring)) * 1.0471975512;
      let uv = in.uv + vec2f(cos(a), sin(a)) * rad / dim;
      // A neighbour has to stand meaningfully higher to take any sky. Without the
      // bias two crystals grown into each other shade one another exactly as the
      // ground is shaded, and every one of them gets drawn round in its own dark
      // outline — which is what makes a sprout of them read as stacked meshes.
      let rel = textureSampleLevel(rockTex, samp, uv, 0.0).a - flat - mine;
      occ = occ + w * max(0.0, rel - u.aoBias);
      wsum = wsum + w;
    }
  }
  let ao = 1.0 - u.aoPower * smoothstep(0.0, max(u.aoRange, 1e-4), occ / max(wsum, 1e-4));
  return vec4f(c.rgb * ao, c.a);
}

struct Out {
  @builtin(position) pos: vec4f,
  @location(0) world: vec3f,
  @location(1) @interpolate(flat) isFlat: f32,
  @location(2) @interpolate(flat) jit: f32,
  @location(3) bary: vec3f,
  @location(4) dn: f32,
};

@vertex
fn vert(@builtin(vertex_index) vi: u32,
        @location(0) p: vec2f, @location(1) z: f32,
        @location(2) origin: vec2f, @location(3) dn: f32,
        @location(4) isFlat: f32, @location(5) jit: f32) -> Out {
  // The body does not assemble itself out of nothing: it is already whole and it
  // comes up through the rock. Only the height moves, and the rock hides whatever
  // has not surfaced yet, so the outline never has to collapse to be hidden.
  let s0 = dn * u.stagger;
  let gi = smoothstep(0.0, 1.0, clamp((u.grow - s0) / max(1e-6, 1.0 - s0), 0.0, 1.0));
  let zz = mix(-u.zMax * 0.9, z, gi);

  var out: Out;
  out.pos = clipOf(p, zz);
  out.world = vec3f(p, zz);
  out.isFlat = isFlat;
  out.jit = jit;
  // Triangles are drawn unindexed, so the corner is the vertex index modulo three
  // and the barycentric weights come free — no duplicated attributes for edges.
  let corner = vi % 3u;
  out.bary = vec3f(f32(corner == 0u), f32(corner == 1u), f32(corner == 2u));
  out.dn = dn;
  return out;
}


fn fbm3(x: vec3f) -> f32 {
  var sum = 0.0;
  var amp = 1.0;
  var norm = 0.0;
  var p = x;
  for (var o = 0u; o < 3u; o = o + 1u) {
    sum += amp * noise3(p);
    norm += amp;
    amp *= 0.5;
    p *= 2.07;
  }
  return sum / norm;
}

/**
 * The facet a point inside the body belongs to, as a normal.
 * The lattice is warped before it is cut into cells, otherwise the cells are a
 * regular grid and the body shows a lattice instead of faceting.
 */
fn innerFacet(p: vec3f, grain: f32) -> vec3f {
  // The warp has to vary on the scale of the cells. In page units the coordinates
  // run to hundreds, so an absolute frequency scrambles it per pixel into dust.
  let q = p / max(grain, 1e-3);
  let warp = vec3f(fbm3(q * 0.6), fbm3(q * 0.6 + 11.3), fbm3(q * 0.6 + 27.1)) - 0.5;
  let cell = floor(q + warp * 1.6);
  let d = vec3f(hash3(cell), hash3(cell + 19.7), hash3(cell + 41.3)) - 0.5;
  return normalize(d + vec3f(0.0, 0.0, 0.35));
}

fn rockTexel(p: vec2f) -> vec4f {
  return textureSampleLevel(rockTex, samp, clamp(p / u.res, vec2f(0.0), vec2f(1.0)), 0.0);
}

/**
 * The rock as seen from a given depth inside the stone.
 * Light that has travelled further through the medium arrives less sharp, so the
 * deeper samples read a coarser mip. Without that, every step returns the same
 * hard-edged rock and the steps show up as bands — or, if scattered to hide them,
 * as noise.
 */
fn rockAtDepth(p: vec2f, depth: f32) -> vec3f {
  let lod = clamp(log2(1.0 + depth * 0.05), 0.0, 7.0);
  return textureSampleLevel(rockTex, samp, clamp(p / u.res, vec2f(0.0), vec2f(1.0)), lod).rgb;
}

fn rockAt(p: vec2f) -> vec3f {
  return rockTexel(p).rgb;
}

/**
 * Where to read the rock for a ray leaving this point.
 * At the rim the facets are steep and the path is long, so an unbounded offset
 * reaches across the page and drags a hard rock edge into the body — several
 * times over, once per step of the march, which is what turns the rim into a
 * stepped band. Refraction may displace, but not arbitrarily far.
 */
fn rockLookup(at: vec2f, dir: vec2f, dist: f32, s: f32, limit: f32) -> vec2f {
  let o = dir * (dist * s);
  let l = length(o);
  return at + select(o, o * (limit / max(l, 1e-4)), l > limit);
}

// Path length from the surface down to the rock beneath, along the refracted ray.
fn pathLength(z: f32, floorZ: f32, dir: vec3f) -> f32 {
  return max(z - floorZ, 0.0) / max(-dir.z, 0.18);
}

@fragment
fn frag(in: Out) -> @location(0) vec4f {
  let floorZ = u.rockMin + rockTexel(in.world.xy).a * u.rockSpan;
  // Buried is buried — but only while it is still surfacing. The height comes from
  // an 8-bit channel over faceted rock, so at rest, where the skirt runs level with
  // the ground, testing against it chews the edge into a ragged band.
  if (u.grow < 0.999 && in.world.z < floorZ - 2.0) { discard; }

  let n = faceNormal(in.world, in.jit, u.bevel);
  let v = viewDir(in.world);
  let cosI = clamp(dot(n, v), 0.0, 1.0);

  // Fresnel splits the face between mirror and window.
  let f0 = 0.08;
  let fres = f0 + (1.0 - f0) * pow(1.0 - cosI, 5.0);

  // Dispersion: each channel refracts at its own index, so the rock seen through
  // the stone splits into colour at grazing angles.
  let d = u.dispersion * 0.04;
  let rr = refract(-v, n, 1.0 / (u.ior - d));
  let rg = refract(-v, n, 1.0 / u.ior);
  let rb = refract(-v, n, 1.0 / (u.ior + d));

  let tr = pathLength(in.world.z, floorZ, rr);
  let tg = pathLength(in.world.z, floorZ, rg);
  let tb = pathLength(in.world.z, floorZ, rb);

  let s = u.refractScale;
  // Colour zoning. Adding density at the core does nothing visible: the core is the
  // thick part, its optical depth is already saturated, and the only place a raised
  // coefficient still shows is the thin rim — which darkens the surroundings, the
  // opposite of what zoning means. So the zone redistributes instead of adding:
  // the core drinks more, the rim drinks correspondingly less, and the contrast is
  // between them rather than over the whole body.
  let toEdge = clamp(in.dn, 0.0, 1.0);
  let zone = pow(max(u.core, 0.05), 1.0 - 2.0 * toEdge);
  let k = u.absorb * u.stoneDensity * zone * 0.002;

  // Beer-Lambert at the far wall: what is left of the rock after the whole trip.
  let survive = exp(-k * vec3f(tr, tg, tb));
  let reach = min(u.zMax * 0.45, 70.0);
  let through = vec3f(rockAtDepth(rockLookup(in.world.xy, rr.xy, tr, s, reach), tr).r,
                      rockAtDepth(rockLookup(in.world.xy, rg.xy, tg, s, reach), tg).g,
                      rockAtDepth(rockLookup(in.world.xy, rb.xy, tb, s, reach), tb).b) * survive;

  // Depth is not a texture on the surface: it is seeing several depths at once.
  // Walking the refracted ray gives the rock over again at each step, displaced
  // further and dimmed more, and those overlapping images are what the eye reads
  // as thickness. Cloudiness is three-dimensional noise, not planes — planes at a
  // regular spacing are stripes, whatever they are called.
  let hIn = normalize(normalize(vec3f(u.pointer, u.zMax * 2.2) - in.world) + v);
  let grain = 90.0 / max(u.veil, 0.05);

  var inner = vec3f(0.0);
  var facets = 0.0;
  for (var i = 0u; i < 4u; i = i + 1u) {
    let f = (f32(i) + 0.5) / 4.0;
    let dr = tr * f; let dg = tg * f; let db = tb * f;
    let p = in.world + rg * dg;
    let atten = exp(-k * vec3f(dr, dg, db));
    inner += vec3f(rockAtDepth(rockLookup(in.world.xy, rr.xy, dr, s, reach), dr).r,
                   rockAtDepth(rockLookup(in.world.xy, rg.xy, dg, s, reach), dg).g,
                   rockAtDepth(rockLookup(in.world.xy, rb.xy, db, s, reach), db).b) * atten;

    // A facet down here answers the light on its own. This is what a dense body
    // still has to show: reflections from inside, not one highlight on the skin.
    let facetN = innerFacet(p, grain);
    facets += pow(max(dot(facetN, hIn), 0.0), u.shininess * 0.35)
            * exp(-length(k) * dg * 0.45);
  }
  inner *= 0.25;
  facets *= 0.25;

  // Absorption alone leaves the thick middle dead black — a husk with a lit rim.
  let filled = 1.0 - exp(-tg * 0.006);

  // Scattering is driven by the light that gets in, so a facet turned away from the
  // source stays dark. Without this the body glows on its own and the whole surface
  // stops answering the light at all.
  let lit = max(dot(n, normalize(u.lightDir)), 0.0) * u.lightPower;
  let entering = (1.0 - fres) * (0.18 + 0.82 * lit);
  // Scattered light still has to get out through the same stone. Without this the
  // scattering term ignores density entirely, so once the transmitted image dies
  // the whole body goes evenly dark and the zoning stops showing at all.
  let escape = exp(-k * (tg * 0.5));
  // Weighted by how much stone the light actually crossed. Added flat, these four
  // copies land on top of the transmitted image, which already carries the rock —
  // and where the body is thin nothing attenuates any of them, so the skirt ends up
  // brighter than the rock it is standing in.
  // The layered depth takes the stone's colour. Light scattered inside a coloured
  // medium leaves it coloured; passed through untinted it is the rock's own grey
  // light, and on shallow steps almost nothing absorbs it — which is what washes
  // the amethyst out of the body and leaves the facets looking like clear glass.
  let body = u.tint * u.glow * (0.25 * survive + u.scatter * filled * escape) * entering
           + inner * u.tint * 1.8 * u.veilFlash * filled
           + mix(u.tint, vec3f(1.0), 0.55) * facets * u.spec * u.innerFacets;

  // The mirror half. Sampling the rock at a wide planar offset tears the rim apart:
  // there the view grazes, fresnel goes to one, and the whole colour becomes rock
  // dragged in from a hundred pixels away. The offset is bounded, and a reflection
  // that would leave the surface fades to ambient rather than reaching for rock
  // that is nowhere near the point it is meant to be reflecting.
  let refl = reflect(-v, n);
  let leaving = clamp(1.0 - abs(refl.z), 0.0, 1.0);
  let mirrorUV = in.world.xy + refl.xy * min(u.zMax * 0.10, 34.0);
  // What a grazing facet reflects here is the page, and the page is dark. Falling
  // back to a bright constant instead floods the whole rim with flat light, because
  // that is exactly where fresnel hands the colour over to this term.
  let ambient = u.rockLow * 1.4 + u.tint * 0.04;
  let mirrored = mix(rockAt(mirrorUV), ambient, leaving);

  let key = max(dot(n, normalize(u.lightDir)), 0.0) * u.lightPower;
  let glint = glintOf(in.world, n, u.shininess) * u.spec * glintFresnel(in.world, f0);

  // Every facet is cut and polished a little differently, so no two send back quite
  // the same light. This does not go through the normal, which is why it still tells
  // facets apart on a table that is perfectly flat and lit straight down its own
  // normal — the case where nothing that depends on the normal can.
  let vary = 1.0 + (in.jit - 0.5) * u.facetVary;
  // A colour the facet owns. Everything else here multiplies whatever light happens
  // to arrive, and a multiplier on colourless light is colourless — which is why a
  // body lit through pale rock ends up looking like clear glass however hard the
  // facets are told to differ. Mixed in rather than added: adding colour always
  // adds light, and the body only got paler for it.
  let arriving = mix(through + body, mirrored, fres) + u.tint * key * 0.06;
  let own = u.tint * (0.18 + 0.55 * in.jit);
  var col = mix(arriving, own, clamp(u.facetBase, 0.0, 1.0)) * vary + vec3f(glint);

  // The seam between facets. Cut stone shows where one face ends and the next
  // begins; without it large facets read as flat shapes rather than as a body.
  if (u.edge > 0.0) {
    let e = min(min(in.bary.x, in.bary.y), in.bary.z);
    let w = fwidth(e) * u.edge;
    col *= 1.0 - 0.55 * (1.0 - smoothstep(0.0, max(w, 1e-5), e));
  }
  return vec4f(col, 1.0);
}
`;

function crystalMesh(g, heights) {
  const { Z } = heights;
  const data = new Float32Array(g.tris.length * 3 * CRYSTAL_FLOATS);
  let o = 0;
  for (const t of g.tris)
    for (const k of [t.a, t.b, t.c]) {
      data[o++] = g.pts[k][0] + g.x;
      data[o++] = g.pts[k][1] + g.y;
      data[o++] = Z[k];
      data[o++] = g.origin[k][0] + g.x;
      data[o++] = g.origin[k][1] + g.y;
      data[o++] = g.dn[k];
      data[o++] = t.flat ? 1 : 0;
      data[o++] = (t.jitS + 1) * 0.5;
    }
  return data;
}

/**
 * Rock mesh. Stones are faces of this same surface, flagged so the shader treats
 * them as crystal — they are part of the rock, not bodies laid over it.
 */
function rockMesh(m) {
  const data = new Float32Array(m.tris.length * 3 * ROCK_FLOATS);
  let o = 0;
  for (const t of m.tris) {
    const jit = (t.jit + 6) / 12;
    const stone = t.stone ? 1 : 0;
    for (const k of [t.a, t.b, t.c]) {
      data[o++] = m.pts[k][0];
      data[o++] = m.pts[k][1];
      // Without the heave: the shader puts it back, so it can move per frame.
      data[o++] = m.Z[k] - m.swell * m.socket[k];
      data[o++] = jit;
      data[o++] = stone;
      data[o++] = m.socket[k];
      // How much stone stands above the ground here: zero on the ring, full at the
      // apex. Interpolated across the face it is the thickness light travels.
      data[o++] = m.apex[k] ?? 0;
    }
  }
  return data;
}

/** hsl -> linear-ish rgb, matching the lightness range the SVG matrix uses. */
function hslRgb(h, s, l) {
  const a = s * Math.min(l, 1 - l);
  const f = k => {
    const n = (k + h / 30) % 12;
    return l - a * Math.max(-1, Math.min(Math.min(n - 3, 9 - n), 1));
  };
  return [f(0), f(8), f(4)];
}

/**
 * Where the plateau is in its rise, from the same curve the vertex shader applies.
 * The text sits on that plateau, so it has to be driven by this and not by a
 * second curve of its own.
 */
const tablePhase = grow => {
  const t = Math.min(1, Math.max(0, grow));
  return t * t * (3 - 2 * t);
};

/** True when this browser can run the GPU renderer at all. */
export function gpuAvailable() {
  return typeof navigator !== 'undefined' && !!navigator.gpu;
}

/**
 * GPU renderer for the outcrops and the rock they grow out of.
 * Geometry and silhouette still come from prepareShard and prepareMatrix; light,
 * absorption, refraction and growth run in WGSL, and both meshes take the same
 * moving light so they answer it together. Resolves to null when WebGPU is
 * missing or the adapter is refused, so the caller can fall back to SVG.
 */
export async function mountCrystalsGPU({
  container, groups, seed = 500, pad = 0, tessel = 78,
  plateauPad = 14, plateauRound = 0, plateauBulge = 0, shapeRound = 1, plateauGrain = 5,
  spread = 0.58, apron = 155, rampFactor = 1 + 155 / 120,
  depth = 20, flatTilt = 0, hue = 259, floor = 6,
  absorb = [1, 2.65, 0.7], ior = 2.4, dispersion = 4,
  spec = 0.51, shininess = 138, refractScale = 1.5, glow = 0, bevel = 0.8,
  scatter = 0.05, veil = 0.5, edge = 0, plateauDome = 0,
  eyeDist = 6000, eyeFollow = 0.6, stoneDensity = 0.84, veilFlash = 1.45,
  innerFacets = 0, core = 3.95,
  rockStep = 340, rockSwell = 55, rockDome = 40, rockRelief = 52,
  breathAmp = 41, breathPeriod = 3.5,
  stoneShare = 1, stoneSize = 28, stoneRise = 1.05, stoneGlow = 1.2, crystalField = 0,
  stoneCrown = 0.42,
  clusterSize = 115, clusterCount = 25, clusterSpread = 4.4,
  stoneSpec = 1.06, stoneDeep = 8, stoneShine = 192, facetVary = 0.9, facetBase = 0.55,
  aoPower = 0.9, aoRadius = 14, aoRange = 0.03,
  rockRidge = 1.5, rockDetail = 0.5, rockDetailScale = 26,
  rockFracture = 0.65, fractureSize = 530, rockRim = 2,
  maxPixelRatio = 1.5, idleFps = 30,
  stoneZoning = 0.7, intergrow = 0.6, stoneLean = 0.3,
  nestCrust = true, crustHeight = 0.45, crustSpread = 0.5, crustFacet = 0.3,
  crustBudget = 500,
  stoneSink = 0.45, aoBias = 0.02,
  // On the screen normal every upward-facing facet takes the same amount of light,
  // so the body fills evenly and only the highlight tells one facet from another.
  // Tuned to 90 all the same; lowering it is what brings the facets back.
  lightAzimuth = 199, lightElevation = 90, lightPower = 1.05,
  animateIn = 1600, stagger = 0.5, onDone, onGrow,
} = {}) {
  if (!container || !gpuAvailable()) return null;

  const adapter = await navigator.gpu.requestAdapter();
  if (!adapter) return null;
  const device = await adapter.requestDevice();
  if (!device) return null;

  const canvas = document.createElement('canvas');
  canvas.setAttribute('aria-hidden', 'true');
  Object.assign(canvas.style, { position: 'absolute', inset: '0', zIndex: '0',
                                width: '100%', height: '100%', pointerEvents: 'none',
                                display: 'block' });
  if (getComputedStyle(container).position === 'static') container.style.position = 'relative';
  container.prepend(canvas);

  const ctx = canvas.getContext('webgpu');
  const format = navigator.gpu.getPreferredCanvasFormat();
  ctx.configure({ device, format, alphaMode: 'opaque' });

  // Validation failures are otherwise silent and leave only a black canvas.
  device.addEventListener?.('uncapturederror', e => console.error('crystal gpu:', e.error?.message));

  const module = device.createShaderModule({ code: SHADER, label: 'crystal' });
  // A shader that fails to compile otherwise shows up only as a black canvas and a
  // pipeline that is "invalid due to a previous error". Say which line it was.
  module.getCompilationInfo?.().then(info => {
    for (const m of info.messages)
      if (m.type === 'error')
        console.error(`crystal shader ${m.lineNum}:${m.linePos} ${m.message}`);
  }).catch(() => {});
  const uniformBuf = device.createBuffer({ size: UNIFORM_BYTES,
                                           usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
  const sampler = device.createSampler({ magFilter: 'linear', minFilter: 'linear',
                                         mipmapFilter: 'linear',
                                         addressModeU: 'clamp-to-edge', addressModeV: 'clamp-to-edge' });
  // Mip generation reads exactly one level, so it must not filter between levels.
  const mipSampler = device.createSampler({ magFilter: 'linear', minFilter: 'linear',
                                            addressModeU: 'clamp-to-edge',
                                            addressModeV: 'clamp-to-edge' });

  const uniformLayout = device.createBindGroupLayout({
    entries: [{ binding: 0, visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT, buffer: {} }],
  });
  const texLayout = device.createBindGroupLayout({
    entries: [
      { binding: 0, visibility: GPUShaderStage.FRAGMENT, sampler: {} },
      { binding: 1, visibility: GPUShaderStage.FRAGMENT, texture: {} },
    ],
  });
  const uniformBind = device.createBindGroup({
    layout: uniformLayout,
    entries: [{ binding: 0, resource: { buffer: uniformBuf } }],
  });

  const rockPipe = device.createRenderPipeline({
    layout: device.createPipelineLayout({ bindGroupLayouts: [uniformLayout] }),
    vertex: {
      module, entryPoint: 'rockVert',
      buffers: [{
        arrayStride: ROCK_FLOATS * 4,
        attributes: [
          { shaderLocation: 0, offset: 0,  format: 'float32x2' },
          { shaderLocation: 1, offset: 8,  format: 'float32' },
          { shaderLocation: 2, offset: 12, format: 'float32' },
          { shaderLocation: 3, offset: 16, format: 'float32' },
          { shaderLocation: 4, offset: 20, format: 'float32' },
          { shaderLocation: 5, offset: 24, format: 'float32' },
        ],
      }],
    },
    fragment: { module, entryPoint: 'rockFrag', targets: [{ format: ROCK_FORMAT }] },
    primitive: { topology: 'triangle-list' },
    depthStencil: { format: 'depth24plus', depthWriteEnabled: true, depthCompare: 'less' },
  });

  const screenLayout = device.createPipelineLayout({ bindGroupLayouts: [uniformLayout, texLayout] });

  const crystalPipe = device.createRenderPipeline({
    layout: screenLayout,
    vertex: {
      module, entryPoint: 'vert',
      buffers: [{
        arrayStride: CRYSTAL_FLOATS * 4,
        attributes: [
          { shaderLocation: 0, offset: 0,  format: 'float32x2' },
          { shaderLocation: 1, offset: 8,  format: 'float32' },
          { shaderLocation: 2, offset: 12, format: 'float32x2' },
          { shaderLocation: 3, offset: 20, format: 'float32' },
          { shaderLocation: 4, offset: 24, format: 'float32' },
          { shaderLocation: 5, offset: 28, format: 'float32' },
        ],
      }],
    },
    fragment: { module, entryPoint: 'frag', targets: [{ format }] },
    primitive: { topology: 'triangle-list' },
    depthStencil: { format: 'depth24plus', depthWriteEnabled: true, depthCompare: 'less' },
  });

  // Each level is drawn from the one above it with the same fullscreen triangle.
  const mipPipe = device.createRenderPipeline({
    layout: screenLayout,
    vertex: { module, entryPoint: 'bgVert' },
    fragment: { module, entryPoint: 'bgFrag', targets: [{ format: ROCK_FORMAT }] },
    primitive: { topology: 'triangle-list' },
  });

  const bgPipe = device.createRenderPipeline({
    layout: screenLayout,
    vertex: { module, entryPoint: 'bgVert' },
    fragment: { module, entryPoint: 'screenFrag', targets: [{ format }] },
    primitive: { topology: 'triangle-list' },
    depthStencil: { format: 'depth24plus', depthWriteEnabled: false, depthCompare: 'always' },
  });

  const cfg = { tessel, plateauPad, plateauRound, plateauBulge, shapeRound, plateauGrain,
                spread, apron, rampFactor, depth, flatTilt, hue, floor,
                absorb: [...absorb], ior, dispersion, spec, shininess, refractScale, glow, bevel,
                scatter, veil, edge, plateauDome, eyeDist, eyeFollow, stoneDensity,
                veilFlash, innerFacets, core,
                breathAmp, breathPeriod,
                rockStep, rockSwell, rockDome, rockRelief,
                stoneShare, stoneSize, stoneRise, stoneGlow, crystalField,
                stoneCrown, clusterSize, clusterCount, clusterSpread,
                stoneSpec, stoneDeep, stoneShine, facetVary, facetBase,
                aoPower, aoRadius, aoRange,
                rockRidge, rockDetail, rockDetailScale,
                rockFracture, fractureSize, rockRim,
                maxPixelRatio, idleFps, stoneZoning, intergrow, stoneLean,
                nestCrust, crustHeight, crustSpread, crustFacet, crustBudget,
                stoneSink, aoBias,
                lightAzimuth, lightElevation, lightPower, stagger };

  let lastDraw = 0, drawn = 0;
  let W = 0, H = 0, zMax = 1, rockMin = 0, rockSpan = 1, apexMax = 1;
  let cbuf = null, ccount = 0, rbuf = null, rcount = 0;
  let depthTex = null, rockDepth = null, rockTex = null, texBind = null;
  let mipCount = 1, mipSteps = [], rockLevel0 = null;
  let geoms = [];
  let busy = false;
  const pointer = { x: 0, y: 0 };
  let grow = 1, raf = 0, riseStart = 0, riseMs = 0;
  let disposed = false, dirty = true;

  const resolve = g => (Array.isArray(g) ? g : [g])
    .flatMap(s => typeof s === 'string' ? [...container.querySelectorAll(s)] : [s])
    .filter(Boolean);

  const writeMesh = (old, data, usage) => {
    old?.destroy();
    const buf = device.createBuffer({ size: Math.max(32, data.byteLength), usage });
    device.queue.writeBuffer(buf, 0, data);
    return buf;
  };

  function rebuild() {
    const base = container.getBoundingClientRect();
    W = Math.round(base.width);
    H = Math.round(container.scrollHeight);
    // Every pixel here is paid for four times over: the rock pass, the mip chain
    // built from it, the occlusion read back off it and the ray march through it.
    // At full device ratio a 1440p window is six megapixels of that, sixty times a
    // second, for a decorative low-poly background. Capped where the flat facets
    // and the soft rock stop showing the difference.
    const dpr = Math.min(devicePixelRatio || 1, Math.max(0.5, cfg.maxPixelRatio));
    canvas.width = Math.max(1, Math.round(W * dpr));
    canvas.height = Math.max(1, Math.round(H * dpr));

    geoms = [];
    const chunks = [];
    zMax = 1;
    groups.forEach((g, i) => {
      const els = resolve(g);
      const rects = els.map(el => {
        const b = el.getBoundingClientRect();
        const v = parseFloat(el.dataset.pad);
        const p = Number.isFinite(v) ? v : pad;
        return [b.left - base.left - p, b.top - base.top - p, b.width + 2 * p, b.height + 2 * p];
      });
      if (!rects.length) return;
      const geom = prepareShard(rects, { seed: seed + i, pageWidth: W, ...cfg });
      const elev = els.map(el => {
        const v = parseFloat(el.dataset.elev);
        return Number.isFinite(v) ? v : cfg.depth;
      });
      const heights = shardHeights(geom, { depth: elev, flatTilt: cfg.flatTilt,
                                           plateauDome: cfg.plateauDome });
      zMax = Math.max(zMax, heights.dMax);
      geoms.push(geom);
      chunks.push(crystalMesh(geom, heights));
    });

    const m = prepareMatrix(W, H, geoms.map(g => g.poly), {
      hue: cfg.hue, step: cfg.rockStep, swell: cfg.rockSwell, dome: cfg.rockDome,
      relief: cfg.rockRelief, rockRidge: cfg.rockRidge,
      rockFracture: cfg.rockFracture, fractureSize: cfg.fractureSize,
      rockRim: cfg.rockRim,
      stoneShare: cfg.stoneShare, stoneSize: cfg.stoneSize, stoneRise: cfg.stoneRise,
      stoneCrown: cfg.stoneCrown,
      nestCrust: cfg.nestCrust, crustHeight: cfg.crustHeight,
      crustSpread: cfg.crustSpread, crustFacet: cfg.crustFacet,
      crustBudget: cfg.crustBudget,
      intergrow: cfg.intergrow, stoneLean: cfg.stoneLean, stoneSink: cfg.stoneSink,
      crystalField: cfg.crystalField, clusterSize: cfg.clusterSize,
      clusterCount: cfg.clusterCount, clusterSpread: cfg.clusterSpread,
      keepOut: geoms.flatMap(g => g.plateaus),
    });
    rockMin = m.zmin;
    rockSpan = m.zmax - m.zmin;
    apexMax = Math.max(1e-3, m.apexMax);
    const rock = rockMesh(m);
    rcount = rock.length / ROCK_FLOATS;
    rbuf = writeMesh(rbuf, rock, GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST);
    rockLow = hslRgb(266, 0.34, m.lo / 100);
    rockHigh = hslRgb(cfg.hue + 10, 0.40, m.hi / 100);

    const total = chunks.reduce((a, c) => a + c.length, 0);
    const all = new Float32Array(total);
    let o = 0;
    for (const c of chunks) { all.set(c, o); o += c.length; }
    ccount = total / CRYSTAL_FLOATS;
    cbuf = writeMesh(cbuf, all, GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST);

    zMax = Math.max(zMax, Math.abs(rockMin), Math.abs(m.zmax), 1);

    depthTex?.destroy();
    depthTex = device.createTexture({ size: [canvas.width, canvas.height], format: 'depth24plus',
                                      usage: GPUTextureUsage.RENDER_ATTACHMENT });
    rockDepth?.destroy();
    rockDepth = device.createTexture({ size: [canvas.width, canvas.height], format: 'depth24plus',
                                       usage: GPUTextureUsage.RENDER_ATTACHMENT });
    rockTex?.destroy();
    // Each level is its own render pass every frame. Past a 256-fold reduction the
    // rock behind the stone is one flat colour and the levels below only add passes,
    // so the chain stops there; a lookup asking for deeper is clamped to the last.
    mipCount = Math.min(9, Math.floor(Math.log2(Math.max(canvas.width, canvas.height))) + 1);
    rockTex = device.createTexture({
      size: [canvas.width, canvas.height], format: ROCK_FORMAT, mipLevelCount: mipCount,
      usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING,
    });
    texBind = device.createBindGroup({
      layout: texLayout,
      entries: [
        { binding: 0, resource: sampler },
        { binding: 1, resource: rockTex.createView() },
      ],
    });

    // A render target must name exactly one level; the rock is drawn into level 0
    // and the rest of the chain is filtered down from it.
    rockLevel0 = rockTex.createView({ baseMipLevel: 0, mipLevelCount: 1 });

    // One target view and one source bind group per level, built once per layout.
    mipSteps = [];
    for (let level = 1; level < mipCount; level++) {
      const src = rockTex.createView({ baseMipLevel: level - 1, mipLevelCount: 1 });
      mipSteps.push({
        view: rockTex.createView({ baseMipLevel: level, mipLevelCount: 1 }),
        bind: device.createBindGroup({
          layout: texLayout,
          entries: [{ binding: 0, resource: mipSampler }, { binding: 1, resource: src }],
        }),
      });
    }

    dirty = true;
    onDone?.([{ faces: ccount / 3 + rcount / 3 }]);
  }

  function layout2() {
    if (busy || disposed) return;
    busy = true;
    try { rebuild(); } finally { busy = false; }
  }

  let rockLow = hslRgb(266, 0.34, 0.045);
  let rockHigh = hslRgb(274, 0.40, 0.19);

  const uni = new Float32Array(UNIFORM_BYTES / 4);
  function writeUniforms(t) {
    uni[0] = W; uni[1] = H; uni[2] = grow; uni[3] = cfg.stagger;
    uni[4] = pointer.x; uni[5] = pointer.y; uni[6] = t; uni[7] = zMax;
    uni[8] = cfg.absorb[0]; uni[9] = cfg.absorb[1]; uni[10] = cfg.absorb[2]; uni[11] = cfg.ior;
    // Azimuth turns the light around the page, elevation lifts it off the plane;
    // at 90 degrees it stands orthogonal to the screen.
    const az = cfg.lightAzimuth * Math.PI / 180;
    const el = cfg.lightElevation * Math.PI / 180;
    uni[12] = Math.cos(el) * Math.cos(az);
    uni[13] = Math.cos(el) * Math.sin(az);
    uni[14] = Math.sin(el);
    uni[15] = cfg.dispersion;
    const tint = hslRgb(cfg.hue, 0.55, 0.55);
    uni[16] = tint[0]; uni[17] = tint[1]; uni[18] = tint[2]; uni[19] = cfg.spec;
    uni[20] = cfg.shininess; uni[21] = cfg.refractScale; uni[22] = cfg.glow; uni[23] = cfg.bevel;
    uni[24] = rockLow[0]; uni[25] = rockLow[1]; uni[26] = rockLow[2]; uni[27] = rockMin;
    uni[28] = rockHigh[0]; uni[29] = rockHigh[1]; uni[30] = rockHigh[2]; uni[31] = rockSpan;
    uni[32] = cfg.lightPower; uni[33] = cfg.stoneGlow;
    uni[34] = cfg.scatter; uni[35] = cfg.veil;
    uni[36] = cfg.edge; uni[37] = cfg.eyeDist; uni[38] = cfg.stoneDensity;
    // The ground never quite settles: a slow breath in and out around the outcrop.
    // The amplitude is absolute, in the same units as the swell, so it still reads
    // when the swell itself is near zero.
    const breath = cfg.breathAmp
      ? Math.sin(2 * Math.PI * t / Math.max(1, cfg.breathPeriod)) * cfg.breathAmp
      : 0;
    // The same factor prepareMatrix scaled its lengths by: the heave is left out of
    // the mesh so it can move per frame, and it has to arrive here already scaled or
    // the moving part and the baked part are in different worlds.
    uni[39] = (cfg.rockSwell + breath) * Math.max(1, Math.sqrt((W * H) / (1920 * 1080)));
    uni[40] = cfg.veilFlash; uni[41] = cfg.innerFacets; uni[42] = cfg.core;
    // The eye rides towards the pointer, so what is seen through the stone moves.
    uni[44] = W * 0.5 + (pointer.x - W * 0.5) * cfg.eyeFollow;
    uni[45] = H * 0.5 + (pointer.y - H * 0.5) * cfg.eyeFollow;
    uni[46] = cfg.stoneSpec; uni[47] = cfg.stoneDeep; uni[48] = cfg.stoneShine;
    uni[49] = cfg.facetVary; uni[50] = cfg.facetBase;
    uni[51] = cfg.aoPower; uni[52] = cfg.aoRadius; uni[53] = cfg.aoRange;
    uni[54] = cfg.rockDetail; uni[55] = cfg.rockDetailScale;
    uni[56] = cfg.stoneZoning;
    uni[57] = apexMax; uni[58] = cfg.aoBias;
    device.queue.writeBuffer(uniformBuf, 0, uni);
  }

  function frame(now) {
    if (disposed) return;
    if (busy || !texBind || !depthTex) { raf = requestAnimationFrame(frame); return; }
    // Idle costs nothing unless something is actually moving: the pointer, the
    // rise, or the ground's own breath.
    const breathing = cfg.breathAmp > 0 && !reduced;
    if (!dirty && riseMs === 0 && !breathing) { raf = requestAnimationFrame(frame); return; }
    // The breath has a period of seconds. Drawing it at the display's rate spends a
    // whole pass — rock, mips, occlusion, ray march — to move the ground by a
    // fraction of a pixel. Nothing else is animating in that case, and half rate is
    // the same picture. The pointer and the rise are never throttled: those are
    // answered by eye and a dropped frame on them is felt.
    if (!dirty && riseMs === 0 && now - lastDraw < 1000 / Math.max(1, cfg.idleFps) - 2) {
      raf = requestAnimationFrame(frame);
      return;
    }
    lastDraw = now;
    dirty = false;

    if (riseMs > 0) {
      const p = Math.min(1, (now - riseStart) / riseMs);
      grow = p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2;
      if (p >= 1) riseMs = 0;
      onGrow?.(tablePhase(grow));
    }
    writeUniforms(now * 0.001);

    const enc = device.createCommandEncoder();

    const rockPass = enc.beginRenderPass({
      colorAttachments: [{ view: rockLevel0,
                           clearValue: { r: rockLow[0], g: rockLow[1], b: rockLow[2], a: 1 },
                           loadOp: 'clear', storeOp: 'store' }],
      depthStencilAttachment: { view: rockDepth.createView(), depthClearValue: 1.0,
                                depthLoadOp: 'clear', depthStoreOp: 'store' },
    });
    if (rcount) {
      rockPass.setPipeline(rockPipe);
      rockPass.setBindGroup(0, uniformBind);
      rockPass.setVertexBuffer(0, rbuf);
      rockPass.draw(rcount);
    }
    rockPass.end();

    for (const step of mipSteps) {
      const p = enc.beginRenderPass({
        colorAttachments: [{ view: step.view, loadOp: 'clear', storeOp: 'store',
                             clearValue: { r: 0, g: 0, b: 0, a: 0 } }],
      });
      p.setPipeline(mipPipe);
      p.setBindGroup(0, uniformBind);
      p.setBindGroup(1, step.bind);
      p.draw(3);
      p.end();
    }

    const pass = enc.beginRenderPass({
      colorAttachments: [{ view: ctx.getCurrentTexture().createView(),
                           clearValue: { r: 0.03, g: 0.02, b: 0.06, a: 1 },
                           loadOp: 'clear', storeOp: 'store' }],
      depthStencilAttachment: { view: depthTex.createView(), depthClearValue: 1.0,
                                depthLoadOp: 'clear', depthStoreOp: 'store' },
    });
    pass.setBindGroup(0, uniformBind);
    pass.setBindGroup(1, texBind);
    pass.setPipeline(bgPipe);
    pass.draw(3);
    if (ccount) {
      pass.setPipeline(crystalPipe);
      pass.setVertexBuffer(0, cbuf);
      pass.draw(ccount);
    }
    pass.end();

    device.queue.submit([enc.finish()]);
    drawn++;
    raf = requestAnimationFrame(frame);
  }

  const onPointer = e => {
    const b = container.getBoundingClientRect();
    pointer.x = e.clientX - b.left;
    pointer.y = e.clientY - b.top;
    dirty = true;
  };
  addEventListener('pointermove', onPointer, { passive: true });

  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
  try {
    layout2();
  } catch (err) {
    canvas.remove();
    removeEventListener('pointermove', onPointer);
    throw err;
  }
  if (!texBind) { canvas.remove(); removeEventListener('pointermove', onPointer); return null; }

  // Not the middle: parked there, the moving light puts a highlight over the core
  // and the body reads as glowing from the inside out before anyone touches it.
  pointer.x = W * 0.12; pointer.y = H * 0.10;
  grow = animateIn && !reduced ? 0 : 1;
  if (animateIn && !reduced) { riseStart = performance.now(); riseMs = animateIn; }
  raf = requestAnimationFrame(frame);

  let timer, lw = 0, lh = 0;
  const relayout = () => {
    try { layout2(); } catch (err) { console.warn('crystal: relayout failed', err); }
  };
  const schedule = () => {
    const b = container.getBoundingClientRect();
    const h = container.scrollHeight;
    if (Math.abs(b.width - lw) < 2 && Math.abs(h - lh) < 2) return;
    lw = b.width; lh = h;
    clearTimeout(timer);
    timer = setTimeout(relayout, 140);
  };
  const ro = new ResizeObserver(schedule);
  ro.observe(container);
  for (const g of groups) for (const el of resolve(g)) ro.observe(el);
  addEventListener('resize', schedule, { passive: true });
  document.fonts?.ready.then(relayout).catch(() => {});

  const GEOM_KEYS = ['spread', 'plateauPad', 'plateauRound', 'plateauBulge', 'shapeRound',
                     'apron', 'rampFactor',
                     'tessel', 'depth', 'flatTilt', 'plateauDome', 'plateauGrain',
                     'hue', 'rockStep', 'rockSwell', 'rockDome', 'rockRelief', 'rockRidge',
                     'rockFracture', 'fractureSize', 'rockRim',
                     'stoneShare', 'stoneSize', 'stoneRise', 'stoneCrown',
                     'intergrow', 'stoneLean', 'stoneSink', 'crystalField',
                     'nestCrust', 'crustHeight', 'crustSpread', 'crustFacet', 'crustBudget',
                     'clusterSize', 'clusterCount', 'clusterSpread', 'maxPixelRatio'];

  const handle = () => {
    disposed = true;
    cancelAnimationFrame(raf);
    ro.disconnect();
    removeEventListener('resize', schedule);
    removeEventListener('pointermove', onPointer);
    clearTimeout(timer);
    canvas.remove();
  };
  handle.update = patch => {
    Object.assign(cfg, patch);
    dirty = true;
    if (Object.keys(patch).some(k => GEOM_KEYS.includes(k))) relayout();
  };
  handle.redraw = relayout;
  handle.rise = (ms = 1600) => {
    dirty = true;
    if (!ms || reduced) { grow = 1; riseMs = 0; onGrow?.(1); return; }
    grow = 0; riseStart = performance.now(); riseMs = ms; onGrow?.(tablePhase(0));
  };
  handle.freeze = k => {
    riseMs = 0;
    grow = Math.max(0, Math.min(1, k));
    dirty = true;
    onGrow?.(tablePhase(grow));
  };
  handle.backend = 'webgpu';
  /** What the frame actually costs: passes submitted, and over how many pixels. */
  handle.cost = () => ({
    drawn, mips: mipCount, w: canvas.width, h: canvas.height,
    rockTris: rcount / 3, crystalTris: ccount / 3,
    megapixels: +(canvas.width * canvas.height / 1e6).toFixed(2),
  });
  return handle;
}
