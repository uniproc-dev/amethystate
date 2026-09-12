const STORE = 'amethystate:crystal-tuning';
const PRESETS = 'amethystate:crystal-presets';

const JITTER = [
  { flatHueJitter: 0, flatSatJitter: 0, flatLigJitter: 0 },
  { flatHueJitter: 5, flatSatJitter: 5, flatLigJitter: 0 },
  { flatHueJitter: 5, flatSatJitter: 5, flatLigJitter: 1.6 },
];

// Tunings that earned a name. They live here rather than in one browser's storage,
// so they survive a reset, a new machine and a cleared profile. Each holds only what
// it moves off the defaults, so a default that changes carries into all of them.
const BUILTIN = {
  druzy: {
    shapeRound: 0, spread: 1.2, depth: 20, apron: 600,
    plateauPad: 54, plateauRound: 0, plateauDome: 0, flatTilt: 24, tessel: 248,
  },
  outcrop: {
    shapeRound: 0, spread: 1.2, depth: 20, apron: 600,
    plateauPad: 54, plateauRound: 0, plateauDome: 0, flatTilt: 24, tessel: 248,
    rockSwell: -60,
  },
  geode: {
    shapeRound: 0, spread: 1.2, depth: 75, apron: 600,
    plateauPad: 54, plateauRound: 0, plateauDome: 1, flatTilt: 24, tessel: 248,
    rockSwell: -60,
    clusterSpread: 1, clusterCount: 1, clusterSize: 240, crystalField: 0,
    edge: 0.95, facetVary: 0.66, bevel: 2, facetBase: 0,
  },
  'deep vein': {
    shapeRound: 0, spread: 1.2, depth: 75, apron: 600,
    plateauPad: 54, plateauRound: 0, plateauDome: 1, flatTilt: 24, tessel: 248,
    rockSwell: -400, rockDome: 370, rockStep: 725,
    clusterSpread: 1, clusterCount: 1, clusterSize: 240, crystalField: 0,
    edge: 0.95, facetVary: 0.66, bevel: 2, facetBase: 0,
    stoneGlow: 2.5, stoneRise: 2.2, stoneSize: 8,
    breathAmp: 90, breathPeriod: 10.5,
  },
};

// Which section a control sits in, and whether the panel was left open, is not part
// of a tuning: two identical stones must compare equal however their panel was left.
const PANEL_ONLY = new Set(['panel', 'frozen', 'boxes']);
const tuningOf = o => {
  const out = {};
  for (const k of Object.keys(o).sort())
    if (!k.startsWith('open:') && !PANEL_ONLY.has(k)) out[k] = o[k];
  return out;
};

const CSS = `
/* A bookmark, not a control: pinned to the edge, out of the way until wanted. */
#ctlbtn{position:fixed;right:0;top:14px;z-index:101;
 font:10px 'JetBrains Mono',monospace;cursor:pointer;padding:7px 10px 7px 12px;
 background:rgba(21,15,33,.72);color:#8d7fae;border:1px solid #2c2140;border-right:0;
 border-radius:6px 0 0 6px;letter-spacing:.14em;opacity:.5;
 backdrop-filter:blur(8px);transition:opacity .18s ease,color .18s ease}
#ctlbtn:hover{opacity:1;color:#c3a3e4}
/* The named tunings, mirrored to the opposite edge: one click each, no panel needed. */
#ctlpre{position:fixed;left:0;top:14px;z-index:101;
 display:flex;flex-direction:column;align-items:flex-start;gap:4px}
#ctlpre button{font:10px 'JetBrains Mono',monospace;cursor:pointer;padding:7px 12px 7px 10px;
 background:rgba(21,15,33,.72);color:#8d7fae;border:1px solid #2c2140;border-left:0;
 border-radius:0 6px 6px 0;letter-spacing:.14em;text-transform:uppercase;opacity:.5;
 backdrop-filter:blur(8px);transition:opacity .18s ease,color .18s ease,background .18s ease}
#ctlpre button:hover{opacity:1;color:#c3a3e4}
#ctlpre button[aria-pressed=true]{opacity:1;background:#b08ad8;color:#150e21;border-color:#b08ad8}
#ctl{position:fixed;right:14px;top:52px;z-index:100;width:296px;max-height:76vh;
 overflow:auto;display:flex;flex-wrap:wrap;gap:6px;align-items:center;padding:14px;
 background:rgba(8,5,16,.94);backdrop-filter:blur(12px);border:1px solid #2c2140;
 font:11px 'JetBrains Mono',monospace}
#ctl[hidden]{display:none}
#ctl details{flex:0 0 100%;border-top:1px solid #241a34}
#ctl details:first-of-type{border-top:0}
#ctl summary{cursor:pointer;list-style:none;padding:7px 0;color:#b08ad8;
 text-transform:uppercase;letter-spacing:.12em;font-weight:500}
#ctl summary::-webkit-details-marker{display:none}
#ctl summary::before{content:'+ ';color:#6b5f82}
#ctl details[open] summary::before{content:'− '}
#ctl .body{display:flex;flex-wrap:wrap;gap:6px;align-items:center;padding:0 0 10px}
#ctl .row{flex:0 0 100%;display:flex;flex-wrap:wrap;gap:6px;align-items:center}
#ctl .row[hidden]{display:none}
#ctl .find{flex:0 0 100%;margin-bottom:8px;padding:6px 9px;background:#0d0916;
 color:#e4dcf0;border:1px solid #2c2140;font:11px 'JetBrains Mono',monospace}
#ctl .find::placeholder{color:#5b5175}
#ctl .name{flex:1 1 120px;min-width:0;padding:5px 8px;background:#0d0916;
 color:#e4dcf0;border:1px solid #2c2140;font:11px 'JetBrains Mono',monospace}
#ctl .preset{flex:0 0 100%;display:flex;gap:6px;align-items:center}
#ctl .preset button:first-child{flex:1 1 auto;text-align:left}
#ctl .preset .kill{flex:0 0 auto;color:#7d6f99}
#ctl .preset .kill:hover{color:#e88}
#ctl .l{color:#6b5f82;text-transform:uppercase;letter-spacing:.1em;flex:0 0 100%;margin-top:8px}
#ctl .body > .l:first-child{margin-top:0}
#ctl button{font:11px 'JetBrains Mono',monospace;cursor:pointer;padding:5px 11px;
 background:#150f21;color:#a396bd;border:1px solid #2c2140}
#ctl button[aria-pressed=true]{background:#b08ad8;color:#150e21;border-color:#b08ad8}
#ctl input[type=range]{flex:0 0 100%;width:100%;margin:2px 0;accent-color:#b08ad8}
#ctl output{color:#c3a3e4;text-transform:none;letter-spacing:0;margin-left:6px}
#ctl .stat{color:#7d6f99;flex:0 0 100%;margin-top:12px;padding-top:10px;border-top:1px solid #241a34}
body[data-boxes=on] [data-crystal]{outline:1px dashed rgba(255,90,168,.8)}
`;

function loadPresets() {
  try {
    const parsed = JSON.parse(localStorage.getItem(PRESETS) || '{}');
    return parsed && typeof parsed === 'object' ? parsed : {};
  } catch {
    return {};
  }
}

function loadSaved() {
  try {
    const raw = localStorage.getItem(STORE);
    const parsed = raw ? JSON.parse(raw) : null;
    return parsed && typeof parsed === 'object' ? parsed : {};
  } catch {
    return {};
  }
}

/**
 * Live tuning panel for a mountCrystals handle. Toggled with H or the corner button.
 * Every control carries the key it drives, so what you tune is what gets stored and
 * restored on the next load. Development instrument: mount it behind
 * import.meta.env.DEV so it never ships.
 */
export function mountControls(handle, opts = {}) {
  const saved = loadSaved();
  const state = { ...saved };
  const anim = {
    ms: saved.riseMs ?? opts.animateIn ?? 1600,
    ease: saved.easing ?? opts.easing ?? 'inOut',
  };

  const persist = () => {
    try { localStorage.setItem(STORE, JSON.stringify(state)); } catch { /* private mode */ }
  };
  const start = (key, fallback) => saved[key] ?? opts[key] ?? fallback;

  const style = document.createElement('style');
  style.textContent = CSS;
  document.head.append(style);

  const btn = document.createElement('button');
  btn.id = 'ctlbtn';
  btn.type = 'button';

  const ctl = document.createElement('div');
  ctl.id = 'ctl';
  document.body.append(btn, ctl);

  // Swapping a tuning means swapping what the panel restores from, so the page is
  // rebuilt from the top rather than patched control by control. The outgoing one
  // goes to the undo slot first: every route into a new tuning has a way back.
  const applyTuning = next => {
    try {
      const raw = localStorage.getItem(STORE);
      if (raw) localStorage.setItem(`${STORE}:undo`, raw);
      if (next) localStorage.setItem(STORE, JSON.stringify(next));
      else localStorage.removeItem(STORE);
    } catch { /* private mode */ }
    location.reload();
  };

  const strip = document.createElement('div');
  strip.id = 'ctlpre';
  const live = JSON.stringify(tuningOf(state));
  for (const [key, tuning] of [['defaults', null], ...Object.entries(BUILTIN)]) {
    const b = document.createElement('button');
    b.type = 'button';
    b.textContent = key;
    b.setAttribute('aria-pressed', String(live === JSON.stringify(tuning ? tuningOf(tuning) : {})));
    b.onclick = () => applyTuning(tuning && tuningOf(tuning));
    strip.append(b);
  }
  document.body.append(strip);

  // Forty-odd controls in one list is a list nobody can find anything in.
  // Everything after a section() call lands in that section.
  let current = ctl;
  const sections = [];
  function section(title) {
    const d = document.createElement('details');
    const s = document.createElement('summary');
    s.textContent = title;
    const body = document.createElement('div');
    body.className = 'body';
    d.append(s, body);
    ctl.append(d);
    d.open = saved[`open:${title}`] ?? true;
    d.addEventListener('toggle', () => { state[`open:${title}`] = d.open; persist(); });
    sections.push(d);
    current = body;
  }

  const label = (text, host) => {
    const s = document.createElement('span');
    s.className = 'l';
    s.textContent = text;
    (host || current).append(s);
    return s;
  };

  // Each control lives in a row of its own, so the filter has one thing to hide.
  const rows = [];
  function row(name) {
    const d = document.createElement('div');
    d.className = 'row';
    d.dataset.name = name.toLowerCase();
    current.append(d);
    rows.push(d);
    return d;
  }

  /** apply defaults to handle.update({[key]: v}); pass one to fan a key out further. */
  function slider(key, name, min, max, step, fallback, digits, apply) {
    const value = Number(start(key, fallback));
    const run = apply || (v => handle.update({ [key]: v }));
    const host = row(`${name} ${key}`);
    const l = label(name, host);
    const out = document.createElement('output');
    out.textContent = value.toFixed(digits);
    l.append(out);
    const el = document.createElement('input');
    el.type = 'range';
    Object.assign(el, { min, max, step, value });
    let raf = 0;
    el.oninput = () => {
      const v = parseFloat(el.value);
      out.textContent = v.toFixed(digits);
      state[key] = v;
      persist();
      if (raf) return;
      raf = requestAnimationFrame(() => { raf = 0; run(v); });
    };
    host.append(el);
    return value;
  }

  function choice(key, name, options, fallback, apply) {
    const active = start(key, fallback);
    const host = row(`${name} ${key}`);
    if (name) label(name, host);
    const made = options.map(([text, value]) => {
      const b = document.createElement('button');
      b.type = 'button';
      b.textContent = text;
      b.setAttribute('aria-pressed', String(value === active));
      b.onclick = () => {
        for (const other of made) other.setAttribute('aria-pressed', String(other === b));
        state[key] = value;
        persist();
        apply(value);
      };
      host.append(b);
      return b;
    });
    return active;
  }

  function action(group, text, run) {
    const host = row(`${group} ${text}`);
    if (group) label(group, host);
    const b = document.createElement('button');
    b.type = 'button';
    b.textContent = text;
    b.onclick = run;
    host.append(b);
  }

  const geom = patch => { handle.update(patch); handle.redraw(); };

  const gpu = handle.backend === 'webgpu';

  section('body shape');
  slider('depth', 'height', 20, 1200, 5, 150, 0, v => geom({ depth: v }));
  slider('spread', 'contour rise', 0.02, 1.2, 0.01, 0.14, 2);
  slider('apron', 'apron', 0, 600, 5, 60, 0,
         v => handle.update({ apron: v, rampFactor: 1 + v / 120 }));
  if (gpu) slider('shapeRound', 'body roundness', 0, 1, 0.02, 0.8, 2);

  section('plateau');
  slider('plateauPad', 'plateau pad', 8, 400, 2, 56, 0);
  slider('plateauRound', 'plateau round', 0, 1, 0.02, 1, 2);
  slider('plateauBulge', 'plateau bulge', 0, 2, 0.05, 0.9, 2);
  if (gpu) slider('plateauDome', 'plateau dome', 0, 1, 0.02, 0.8, 2);

  section('faceting');
  slider('flatTilt', 'face tilt', 0, 24, 0.5, 5, 1);
  if (gpu) {
    slider('tessel', 'tessellation (0 = auto)', 0, 320, 2, 0, 0);
    slider('plateauGrain', 'table grain', 1, 5, 0.1, 2.4, 1);
    slider('edge', 'facet edges', 0, 4, 0.05, 0, 2);
    slider('facetVary', 'facet variation', 0, 3, 0.02, 0.9, 2);
    // The one that gives each facet its own normal, and so its own view into the
    // body. Without it a flat table refracts, reflects and lights identically
    // everywhere, and no amount of per-facet tinting will show different things.
    slider('bevel', 'facet tilt spread', 0, 2, 0.01, 0.8, 2);
    slider('facetBase', 'facet own colour', 0, 1, 0.02, 0.55, 2);
  }
  choice('jitter', 'plateau jitter', [['none', 0], ['hue', 1], ['hue+light', 2]], 1,
         v => handle.update(JITTER[v]));

  if (gpu) {
    section('stone optics');
    const absorb = [...(opts.absorb ?? [1.5, 3.4, 0.9])];
    const setAbsorb = (i, v) => { absorb[i] = v; handle.update({ absorb: [...absorb] }); };
    absorb[0] = slider('absorbR', 'absorb red', 0, 6, 0.05, absorb[0], 2, v => setAbsorb(0, v));
    absorb[1] = slider('absorbG', 'absorb green', 0, 6, 0.05, absorb[1], 2, v => setAbsorb(1, v));
    absorb[2] = slider('absorbB', 'absorb blue', 0, 6, 0.05, absorb[2], 2, v => setAbsorb(2, v));
    slider('stoneDensity', 'stone density', 0, 16, 0.02, 1, 2);
    slider('core', 'denser core', 0.2, 24, 0.05, 2.2, 2);
    slider('ior', 'refractive index', 1, 3.2, 0.01, 1.55, 2);
    slider('dispersion', 'dispersion', 0, 4, 0.05, 1, 2);
    slider('refractScale', 'refraction offset', 0, 5, 0.01, 0.25, 2);

    section('inside the stone');
    slider('innerFacets', 'inner facets', 0, 14, 0.1, 0, 1);
    slider('veil', 'inner facet grain', 0.05, 6, 0.05, 1, 2);
    slider('veilFlash', 'depth layers', 0, 3, 0.05, 0.9, 2);
    slider('scatter', 'inner scatter', 0, 4, 0.05, 0.45, 2);
    slider('glow', 'inner glow', 0, 2, 0.02, 0.5, 2);

    section('light');
    slider('lightAzimuth', 'light azimuth', 0, 360, 1, 239, 0);
    slider('lightElevation', 'light elevation', 0, 90, 1, 52, 0);
    slider('lightPower', 'light power', 0, 2.5, 0.05, 1, 2);
    slider('spec', 'specular', 0, 1.5, 0.01, 0.55, 2);
    slider('shininess', 'shininess', 4, 300, 2, 90, 0);
    slider('eyeDist', 'eye distance', 300, 20000, 25, 1800, 0);
    slider('eyeFollow', 'eye follows pointer', 0, 1.6, 0.02, 0.7, 2);
  }

  section('rock');
  choice('matrix', 'matrix', [['on', true], ['off', false]], true,
         v => handle.update({ matrix: v }));
  if (gpu) {
    slider('rockStep', 'rock grain', 40, 900, 5, 130, 0);
    slider('rockRelief', 'relief', 0, 240, 2, 52, 0);
    slider('rockFracture', 'broken into blocks', 0, 2, 0.05, 1, 2);
    slider('fractureSize', 'block size', 40, 700, 10, 260, 0);
    slider('rockRim', 'lip of the socket', 0, 5, 0.05, 1, 2);
    slider('rockRidge', 'ridges', 0, 4, 0.05, 0.2, 2);
    slider('rockDetail', 'surface detail', 0, 2, 0.05, 0.5, 2);
    slider('rockDetailScale', 'detail size', 4, 120, 1, 26, 0);
    slider('rockDome', 'rock dome', -500, 500, 5, 85, 0);
    slider('rockSwell', 'swell at outcrop', -400, 400, 5, 10, 0);
    slider('breathAmp', 'swell breath', 0, 90, 1, 28, 0);
    slider('breathPeriod', 'breath period, s', 1.5, 40, 0.5, 5, 1);

    section('stones in rock');
    slider('stoneShare', 'how many', 0, 1, 0.02, 0.35, 2);
    slider('stoneSize', 'size', 8, 140, 1, 54, 0);
    slider('stoneRise', 'rise', 0.1, 2.2, 0.05, 0.9, 2);
    slider('stoneCrown', 'crown', 0, 0.8, 0.02, 0.42, 2);
    slider('stoneZoning', 'pale foot, violet head', 0, 1, 0.05, 0.7, 2);
    // One pad under the whole nest instead of a body per crystal.
    choice('nestCrust', 'nest', [['separate', false], ['one crust', true]], false,
           v => handle.update({ nestCrust: v }));
    slider('crustHeight', 'how far the crust comes up', 0, 1, 0.05, 0.45, 2);
    slider('crustSpread', 'how far it reaches out', 0, 1.5, 0.05, 0.5, 2);
    slider('crustFacet', 'how rough the crust is', 0, 1.2, 0.05, 0.3, 2);
    slider('crustBudget', 'points a nest may spend', 80, 3000, 20, 500, 0);
    slider('intergrow', 'grown into each other', 0, 1, 0.05, 0.6, 2);
    slider('stoneLean', 'lean off the wall', 0, 1.2, 0.05, 0.3, 2);
    slider('stoneSink', 'how deep the foot is buried', 0, 1.2, 0.05, 0.45, 2);
    slider('stoneGlow', 'glow', 0, 2.5, 0.05, 0.9, 2);
    slider('stoneSpec', 'specular', 0, 2, 0.02, 0.55, 2);
    slider('stoneShine', 'shininess', 4, 240, 2, 54, 0);
    slider('stoneDeep', 'colour with thickness', 0, 30, 0.5, 7, 1);
    slider('crystalField', 'rock turned to crystal', 0, 1, 0.02, 0, 2);
    slider('clusterSize', 'nest radius', 10, 320, 5, 90, 0);
    slider('clusterCount', 'stones per nest', 1, 60, 1, 5, 0);
    slider('clusterSpread', 'space between nests', 1, 6, 0.1, 2.4, 1);
  }

  label('ambient occlusion');
  slider('aoPower', 'strength', 0, 1.5, 0.05, 0.9, 2);
  slider('aoRadius', 'radius', 2, 60, 1, 14, 0);
  slider('aoRange', 'depth it reads as full', 0.005, 0.3, 0.005, 0.03, 3);
  slider('aoBias', 'height before it shades at all', 0, 0.1, 0.002, 0.02, 3);

  section('colour');
  slider('hue', 'hue', 240, 300, 1, 266, 0);
  slider('floor', 'floor', 3, 20, 0.5, 7, 1);
  slider('sweep', 'sweep', 0, 0.25, 0.01, 0, 2);

  section('motion');
  slider('riseMs', 'rise duration', 200, 3000, 50, 1600, 0,
         v => { anim.ms = v; handle.rise(v, anim.ease); });
  slider('stagger', 'face order', 0, 0.95, 0.05, 0.5, 2, v => {
    handle.update({ stagger: v });
    handle.rise(anim.ms, anim.ease);
  });
  // Deliberately not stored: coming back to a half-grown stone reads as breakage.
  slider('frozen', 'frozen stage', 0, 1, 0.01, 1, 2, v => handle.freeze(v));
  choice('easing', 'easing', [['out', 'out'], ['in-out', 'inOut'], ['soft', 'soft']], 'inOut',
         v => { anim.ease = v; handle.rise(anim.ms, v); });
  action('', 'grow again', () => handle.rise(anim.ms, anim.ease));

  // A tuning worth keeping should not have to live in the one slot the panel
  // happens to be using. Presets are named copies of it, stored beside it.
  section('presets');
  const presetHost = current;
  const name = document.createElement('input');
  name.className = 'name';
  name.type = 'text';
  name.placeholder = 'name this tuning';

  const writePresets = all => {
    try { localStorage.setItem(PRESETS, JSON.stringify(all)); } catch { /* private mode */ }
  };

  function renderPresets() {
    for (const old of [...presetHost.querySelectorAll('.preset')]) old.remove();
    const all = loadPresets();
    for (const key of Object.keys(all).sort()) {
      const line = document.createElement('div');
      line.className = 'preset';

      const load = document.createElement('button');
      load.type = 'button';
      load.textContent = key;
      load.onclick = () => applyTuning(all[key]);

      const kill = document.createElement('button');
      kill.type = 'button';
      kill.className = 'kill';
      kill.textContent = '×';
      kill.title = `delete ${key}`;
      kill.onclick = () => {
        const rest = loadPresets();
        delete rest[key];
        writePresets(rest);
        renderPresets();
      };

      line.append(load, kill);
      presetHost.append(line);
    }
  }

  const saveRow = row('save preset');
  saveRow.append(name);
  const save = document.createElement('button');
  save.type = 'button';
  save.textContent = 'save';
  save.onclick = () => {
    const key = name.value.trim();
    if (!key) { name.focus(); return; }
    const all = loadPresets();
    all[key] = { ...state };
    writePresets(all);
    name.value = '';
    renderPresets();
  };
  saveRow.append(save);
  name.addEventListener('keydown', e => { if (e.key === 'Enter') save.click(); });
  renderPresets();

  // Every pixel is paid for by four passes and every frame by all of them, so these
  // two are the whole cost of the page in two numbers.
  section('cost');
  slider('maxPixelRatio', 'render scale cap', 0.5, 2, 0.05, 1.5, 2);
  slider('idleFps', 'frames while only breathing', 6, 60, 1, 30, 0);

  section('panel');
  choice('boxes', 'boxes', [['hide', 'off'], ['show', 'on']], 'off',
         v => { document.body.dataset.boxes = v; });
  action('stored tuning', 'copy as defaults', async () => {
    const text = JSON.stringify(state, null, 2);
    try { await navigator.clipboard.writeText(text); } catch { /* fall through */ }
    console.log(text);
  });
  // Reset keeps what it discarded. Losing an evening of tuning to one button with
  // no way back is the button's fault, not the person's.
  action('', 'reset', () => applyTuning(null));
  action('', 'undo reset', () => {
    try {
      const raw = localStorage.getItem(`${STORE}:undo`);
      if (!raw) { console.warn('crystal: nothing to undo'); return; }
      localStorage.setItem(STORE, raw);
    } catch { /* private mode */ }
    location.reload();
  });

  for (const d of sections)
    if (!d.querySelector('.body').childElementCount) d.remove();

  // Forty-five controls need a way to be found by name, not only by category.
  const find = document.createElement('input');
  find.className = 'find';
  find.type = 'search';
  find.placeholder = 'find a control…';
  find.oninput = () => {
    const q = find.value.trim().toLowerCase();
    for (const r of rows) r.hidden = q !== '' && !r.dataset.name.includes(q);
    for (const d of sections) {
      if (!d.isConnected) continue;
      const shown = [...d.querySelectorAll('.row')].some(r => !r.hidden);
      d.hidden = !shown;
      if (q !== '' && shown) d.open = true;
    }
  };
  ctl.prepend(find);

  const stat = document.createElement('span');
  stat.className = 'stat';
  ctl.append(stat);

  // One pass at the end: everything restored is handed over together, so a stored
  // set costs a single rebuild instead of one per control.
  const patch = {};
  for (const [k, v] of Object.entries(saved)) {
    if (k === 'frozen' || k === 'riseMs' || k === 'easing' || k === 'boxes') continue;
    if (k === 'jitter') { Object.assign(patch, JITTER[v]); continue; }
    if (k === 'absorbR' || k === 'absorbG' || k === 'absorbB') continue;
    if (k === 'apron') { patch.apron = v; patch.rampFactor = 1 + v / 120; continue; }
    patch[k] = v;
  }
  if (saved.absorbR !== undefined || saved.absorbG !== undefined || saved.absorbB !== undefined) {
    const base = opts.absorb ?? [1.5, 3.4, 0.9];
    patch.absorb = [saved.absorbR ?? base[0], saved.absorbG ?? base[1], saved.absorbB ?? base[2]];
  }
  if (saved.boxes) document.body.dataset.boxes = saved.boxes;
  if (Object.keys(patch).length) { handle.update(patch); handle.redraw(); }

  function show(on) {
    ctl.hidden = !on;
    btn.textContent = on ? 'hide · H' : 'tune · H';
  }
  // Closed unless it was left open: the panel is an instrument, not the page.
  show(saved.panel === true);
  btn.onclick = () => {
    show(ctl.hidden);
    state.panel = !ctl.hidden;
    persist();
  };

  const onKey = e => {
    if (e.target.matches('input, textarea')) return;
    if (e.key === 'h' || e.key === 'H') { show(ctl.hidden); state.panel = !ctl.hidden; persist(); }
    if (e.key === 'Escape') { show(false); state.panel = false; persist(); }
  };
  addEventListener('keydown', onKey);

  return {
    report: shards => {
      stat.textContent = `${shards.length} outcrop(s) · `
                       + `${shards.reduce((a, s) => a + s.faces, 0)} faces`;
    },
    destroy: () => {
      removeEventListener('keydown', onKey);
      btn.remove(); ctl.remove(); strip.remove(); style.remove();
    },
  };
}
