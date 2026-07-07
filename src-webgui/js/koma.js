// koma gui glue (Wave 3): wires vendored xterm.js + addons to the real koma
// client running in a host-side PTY. Bytes flow both ways over wry ipc:
//   pty  -> host -> window.__koma.write(base64)         -> term.write(Uint8Array)
//   term -> window.ipc.postMessage(JSON {data,resize})  -> host -> pty
(function () {
  console.log('[koma.js] script loaded');
  try {
  document.body.classList.add('os-' + (window.__komaOS || 'linux'));
  // The host (Rust, run_gui) resolves koma's CONFIGURED palette canvas bg
  // (view::theme::palette(cfg).bg) and injects it as window.__komaBg via a
  // wry initialization script, so it's set before this script runs. Falls
  // back to black if missing/malformed (matches the palette's own default).
  var komaBg = (window.__komaBg && /^#[0-9a-fA-F]{6}$/.test(window.__komaBg)) ? window.__komaBg : '#000000';
  // Same deal for the titlebar text + button glyph FOREGROUND: the host resolves
  // koma's CONFIGURED palette fg (view::theme::palette(cfg).fg) and injects it as
  // window.__komaFg. Falls back to the old hardcoded color if missing/malformed.
  var komaFg = (window.__komaFg && /^#[0-9a-fA-F]{6}$/.test(window.__komaFg)) ? window.__komaFg : '#c8d3f5';

  const term = new Terminal({
    fontFamily: '"KomaMono", monospace',
    fontSize: 14,
    cursorBlink: true,
    allowProposedApi: true,
    theme: { background: komaBg, foreground: komaFg },
    scrollback: 10000,
  });

  const fit = new FitAddon.FitAddon();
  term.loadAddon(fit);
  try { term.loadAddon(new Unicode11Addon.Unicode11Addon()); term.unicode.activeVersion = '11'; } catch (e) {}
  try { term.loadAddon(new WebLinksAddon.WebLinksAddon()); } catch (e) {}
  try { term.loadAddon(new ClipboardAddon.ClipboardAddon()); } catch (e) {}

  term.open(document.getElementById('term'));

  // Match the container + body bg to the resolved palette so the cols/rows
  // remainder gutter (right/bottom strip xterm can't fill with whole cells)
  // reads as the same color instead of a visibly different near-black seam.
  var appEl = document.getElementById('app');
  if (appEl) appEl.style.backgroundColor = komaBg;
  var termEl = document.getElementById('term');
  if (termEl) termEl.style.backgroundColor = komaBg;
  var titlebarEl = document.getElementById('titlebar');
  if (titlebarEl) titlebarEl.style.backgroundColor = komaBg;

  // Drives the titlebar text + non-macOS button glyph color via a CSS custom
  // property (see index.html's #titlebar / .win-btn `color: var(--koma-fg, ...)`).
  function applyFg(fg) { document.documentElement.style.setProperty('--koma-fg', fg); }
  applyFg(komaFg);

  // Live palette sync: koma (in GUI mode) emits its canvas bg + titlebar fg via a
  // private OSC 5380 whenever the palette changes, as `#rrggbb,#rrggbb` (bg first,
  // fg second); repaint the xterm theme + window gutter + titlebar to match.
  try {
    term.parser.registerOscHandler(5380, function (data) {
      var parts = String(data).split(',');
      var bg = parts[0], fg = parts[1];
      if (bg && /^#[0-9a-fA-F]{6}$/.test(bg)) {
        komaBg = bg;
        try { term.options.theme = Object.assign({}, term.options.theme, { background: bg }); } catch (e) {}
        var appEl2 = document.getElementById('app');
        if (appEl2) appEl2.style.backgroundColor = bg;
        var el = document.getElementById('term');
        if (el) el.style.backgroundColor = bg;
        var tb = document.getElementById('titlebar');
        if (tb) tb.style.backgroundColor = bg;
      }
      if (fg && /^#[0-9a-fA-F]{6}$/.test(fg)) {
        komaFg = fg;
        applyFg(fg);
        try { term.options.theme = Object.assign({}, term.options.theme, { foreground: fg }); } catch (e) {}
      }
      return true; // handled — do not render the sequence
    });
  } catch (e) { /* parser API unavailable — static window.__komaBg/__komaFg still applies */ }

  // Ctrl+Shift+C copies the current selection, Ctrl+Shift+V pastes from the
  // system clipboard. Plain Ctrl+C is left alone so it still sends SIGINT to
  // the koma client running in the pty. The clipboard addon handles OSC 52
  // from the app side; this handles user-initiated copy/paste from the UI.
  term.attachCustomKeyEventHandler(function (e) {
    if (e.type === 'keydown' && e.ctrlKey && e.shiftKey) {
      const k = e.key.toLowerCase();
      if (k === 'c') {
        const sel = term.getSelection();
        if (sel && navigator.clipboard) { navigator.clipboard.writeText(sel).catch(function () {}); }
        return false;
      }
      if (k === 'v') {
        if (navigator.clipboard) {
          navigator.clipboard.readText().then(function (t) {
            if (t) {
              const b = new TextEncoder().encode(t);
              let s = '';
              for (let i = 0; i < b.length; i++) s += String.fromCharCode(b[i]);
              post({ t: 'data', d: btoa(s) });
            }
          }).catch(function () {});
        }
        return false;
      }
    }
    return true;
  });

  if (!window.__komaSoftware) {
    try {
      const _webgl = new WebglAddon.WebglAddon();
      _webgl.onContextLoss(function () { try { _webgl.dispose(); } catch (e) {} });
      term.loadAddon(_webgl);
    } catch (e) { /* WebGL unavailable — xterm falls back to its DOM renderer */ }
  } else {
    console.log('[koma.js] software mode: skipping WebGL, using DOM renderer');
  }

  let firstWrite = true;
  window.__koma = {
    term,
    // pty -> xterm: host base64's raw pty bytes; decode to a Uint8Array so
    // multibyte UTF-8 (braille, box-drawing) survives intact.
    write(b64) {
      const bin = atob(b64);
      const arr = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i);
      if (firstWrite) {
        firstWrite = false;
        console.log('[koma.js] first pty write, bytes=' + arr.length);
      }
      term.write(arr);
    },
  };

  function post(obj) {
    try { window.ipc.postMessage(JSON.stringify(obj)); } catch (e) {}
  }

  // Custom titlebar: the window is undecorated (tao `with_decorations(false)`),
  // so drag / minimize / maximize / close all have to be driven host-side via
  // ipc — the host's `event_loop.run` closure calls the actual tao `Window`
  // methods (drag_window / set_minimized / set_maximized / exit).
  var titlebar = document.getElementById('titlebar');
  if (titlebar) {
    titlebar.addEventListener('mousedown', function (e) {
      if (e.button !== 0) return;
      if (e.target.closest('.win-btn')) return; // buttons handle themselves
      if (e.detail === 2) { post({ t: 'win', a: 'max' }); return; } // dbl-click = toggle max
      post({ t: 'win', a: 'drag' });
    });
  }
  function bindBtn(id, a) {
    var b = document.getElementById(id);
    if (b) b.addEventListener('click', function () { post({ t: 'win', a: a }); });
  }
  bindBtn('btn-min', 'min');
  bindBtn('btn-max', 'max');
  bindBtn('btn-close', 'close');

  // Custom edge/corner resize handles -> host drag_resize_window(direction).
  var handles = document.querySelectorAll('.rz');
  for (var i = 0; i < handles.length; i++) {
    (function (h) {
      h.addEventListener('mousedown', function (e) {
        if (e.button !== 0) return;
        e.preventDefault();
        post({ t: 'winresize', dir: h.getAttribute('data-dir') });
      });
    })(handles[i]);
  }

  // keystrokes / paste -> pty (UTF-8 bytes, base64'd)
  term.onData(function (data) {
    const bytes = new TextEncoder().encode(data);
    let bin = '';
    for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
    post({ t: 'data', d: btoa(bin) });
  });

  // xterm computed a new grid size -> pty (TIOCSWINSZ -> SIGWINCH)
  term.onResize(function (size) {
    post({ t: 'resize', cols: size.cols, rows: size.rows });
  });

  // window resize -> refit (fit() triggers onResize above)
  window.addEventListener('resize', function () { try { fit.fit(); } catch (e) {} });

  // ResizeObserver on #term catches container size changes the window
  // 'resize' event misses (e.g. layout/DPI shifts without a window resize),
  // so any non-remainder margin gets refit away too. rAF-debounced so rapid
  // observer callbacks collapse to one fit() per frame.
  if (window.ResizeObserver) {
    var _pending = false;
    var ro = new ResizeObserver(function () {
      if (_pending) return;
      _pending = true;
      requestAnimationFrame(function () { _pending = false; try { fit.fit(); } catch (e) {} });
    });
    ro.observe(document.getElementById('term'));
  }

  // Gate the initial fit/ready handshake on the bundled font actually being
  // loaded: xterm measures cell size from the active font, so if we fit()
  // while the fallback font is still active and KomaMono swaps in after, the
  // grid ends up mis-sized. Fire `ready` exactly once, only after this.
  function boot() {
    try { fit.fit(); } catch (e) {}
    console.log('[koma.js] boot: cols=' + term.cols + ' rows=' + term.rows);
    term.writeln('\x1b[90m[koma gui] boot ok — ' + term.cols + 'x' + term.rows + ', waiting for pty...\x1b[0m');
    post({ t: 'ready' });
  }
  if (document.fonts && document.fonts.ready) {
    Promise.resolve(document.fonts.load('14px "KomaMono"')).catch(function () {})
      .then(function () { return document.fonts.ready; })
      .then(boot).catch(boot);
  } else {
    boot();
  }
  } catch (e) {
    console.error('[koma.js] init failed', e);
    try { document.body.innerText = 'koma.js init failed: ' + (e && e.stack || e); } catch (_){}
  }
})();
