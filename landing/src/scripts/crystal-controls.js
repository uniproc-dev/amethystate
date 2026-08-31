const JITTER = [
  { flatHueJitter: 0, flatSatJitter: 0, flatLigJitter: 0 },
  { flatHueJitter: 5, flatSatJitter: 5, flatLigJitter: 0 },
  { flatHueJitter: 5, flatSatJitter: 5, flatLigJitter: 1.6 },
];

const CSS = `
#ctlbtn{position:fixed;right:14px;bottom:14px;z-index:101;
 font:11px 'JetBrains Mono',monospace;cursor:pointer;padding:9px 14px;
 background:#150f21;color:#c3a3e4;border:1px solid #4b3a69;letter-spacing:.08em}
#ctl{position:fixed;right:14px;bottom:58px;z-index:100;width:296px;max-height:76vh;
 overflow:auto;display:flex;flex-wrap:wrap;gap:6px;align-items:center;padding:14px;
 background:rgba(8,5,16,.94);backdrop-filter:blur(12px);border:1px solid #2c2140;
 font:11px 'JetBrains Mono',monospace}
#ctl[hidden]{display:none}
#ctl .l{color:#6b5f82;text-transform:uppercase;letter-spacing:.1em;flex:0 0 100%;margin-top:8px}
#ctl .l:first-child{margin-top:0}
#ctl button{font:11px 'JetBrains Mono',monospace;cursor:pointer;padding:5px 11px;
 background:#150f21;color:#a396bd;border:1px solid #2c2140}
#ctl button[aria-pressed=true]{background:#b08ad8;color:#150e21;border-color:#b08ad8}
#ctl input[type=range]{flex:0 0 100%;width:100%;margin:2px 0;accent-color:#b08ad8}
#ctl output{color:#c3a3e4;text-transform:none;letter-spacing:0;margin-left:6px}
#ctl .stat{color:#7d6f99;flex:0 0 100%;margin-top:12px;padding-top:10px;border-top:1px solid #241a34}
body[data-boxes=on] [data-crystal]{outline:1px dashed rgba(255,90,168,.8)}
`;

/**
 * Live tuning panel for a mountCrystals handle. Toggled with H or the corner button.
 * Development instrument: mount it behind import.meta.env.DEV so it never ships.
 */
export function mountControls(handle, opts = {}) {
  const anim = { ms: opts.animateIn ?? 1600, ease: opts.easing ?? 'inOut' };

  const style = document.createElement('style');
  style.textContent = CSS;
  document.head.append(style);

  const btn = document.createElement('button');
  btn.id = 'ctlbtn';
  btn.type = 'button';

  const ctl = document.createElement('div');
  ctl.id = 'ctl';
  document.body.append(btn, ctl);

  const label = text => {
    const s = document.createElement('span');
    s.className = 'l';
    s.textContent = text;
    ctl.append(s);
    return s;
  };

  function slider(name, min, max, step, value, digits, apply) {
    const l = label(name);
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
      if (raf) return;
      raf = requestAnimationFrame(() => { raf = 0; apply(v); });
    };
    ctl.append(el);
  }

  function choice(name, options, active, apply) {
    if (name) label(name);
    const made = options.map(([text, value]) => {
      const b = document.createElement('button');
      b.type = 'button';
      b.textContent = text;
      b.setAttribute('aria-pressed', String(value === active));
      b.onclick = () => {
        for (const other of made) other.setAttribute('aria-pressed', String(other === b));
        apply(value);
      };
      ctl.append(b);
      return b;
    });
  }

  function action(group, text, run) {
    label(group);
    const b = document.createElement('button');
    b.type = 'button';
    b.textContent = text;
    b.onclick = run;
    ctl.append(b);
  }

  const geom = patch => { handle.update(patch); handle.redraw(); };

  choice('matrix', [['on', true], ['off', false]], true, v => handle.update({ matrix: v }));
  choice('plateau jitter', [['none', 0], ['hue', 1], ['hue+light', 2]], 1,
         v => handle.update(JITTER[v]));

  slider('face tilt',    0,    24,  0.5,  opts.flatTilt   ?? 5,   1, v => handle.update({ flatTilt: v }));
  slider('height',      40,   320,  5,    opts.depth      ?? 150, 0, v => geom({ depth: v }));
  slider('contour rise', 0.04, 0.40, 0.01, opts.spread    ?? 0.14, 2, v => handle.update({ spread: v }));
  slider('plateau pad', 16,   140,  2,    opts.plateauPad ?? 56,  0, v => handle.update({ plateauPad: v }));
  slider('apron',        0,   160,  5,    opts.apron      ?? 60,  0,
         v => handle.update({ apron: v, rampFactor: 1 + v / 120 }));
  slider('sweep',        0,  0.25,  0.01, opts.sweep      ?? 0,   2, v => handle.update({ sweep: v }));
  slider('hue',        248,   286,  1,    opts.hue        ?? 266, 0, v => handle.update({ hue: v }));
  slider('floor',        3,    15,  0.5,  opts.floor      ?? 7,   1, v => handle.update({ floor: v }));

  slider('rise duration', 200, 3000, 50, anim.ms, 0, v => { anim.ms = v; handle.rise(v, anim.ease); });
  slider('face order',      0, 0.95, 0.05, opts.stagger ?? 0.5, 2, v => {
    handle.update({ stagger: v });
    handle.rise(anim.ms, anim.ease);
  });
  slider('frozen stage', 0, 1, 0.01, 1, 2, v => handle.freeze(v));

  choice('easing', [['out', 'out'], ['in-out', 'inOut'], ['soft', 'soft']], anim.ease,
         v => { anim.ease = v; handle.rise(anim.ms, v); });
  action('animation', 'grow again', () => handle.rise(anim.ms, anim.ease));
  choice('boxes', [['hide', 'off'], ['show', 'on']], 'off',
         v => { document.body.dataset.boxes = v; });

  const stat = document.createElement('span');
  stat.className = 'stat';
  ctl.append(stat);

  function show(on) {
    ctl.hidden = !on;
    btn.textContent = on ? 'hide · H' : 'tune · H';
  }
  show(true);
  btn.onclick = () => show(ctl.hidden);

  const onKey = e => {
    if (e.target.matches('input, textarea')) return;
    if (e.key === 'h' || e.key === 'H') show(ctl.hidden);
    if (e.key === 'Escape') show(false);
  };
  addEventListener('keydown', onKey);

  return {
    report: shards => {
      stat.textContent = `${shards.length} outcrop(s) · `
                       + `${shards.reduce((a, s) => a + s.faces, 0)} faces`;
    },
    destroy: () => {
      removeEventListener('keydown', onKey);
      btn.remove(); ctl.remove(); style.remove();
    },
  };
}
