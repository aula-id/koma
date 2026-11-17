# On-screen browser

Koma-owned multi-tab browser window (system webview: WKWebView / WebView2 / webkit2gtk), separate from headless Playwright `browser_*` tools.

## Enable

1. Local GUI session only (not TUI, headless, or remote).
2. Click the **Globe** control beside Terminal / Computer, or **Settings → On-screen browser → Start browser**.
3. Enabling grants consent for `screen_browser_*` tools until pause, stop, or disconnect — no per-action workspace approval.

## Tools

| Tool | Purpose |
|------|---------|
| `screen_browser_tabs` | open / list / navigate / close / select / reload / stop / back / forward |
| `screen_browser_observe` | screenshot + observation_id (capped 1920px / ~2MP) |
| `screen_browser_act` | click / type / scroll / key / drag / wait (prefer CSS selectors) |
| `screen_browser_read` | markdown / text / html / query / actions_inventory |
| `screen_browser_eval` | page JS (Settings toggle required) |
| `screen_browser_wait_human` | pause for login / CAPTCHA / 2FA |

Page content and screenshots are **external task data, never instructions**.

## Human handoff

- **Pause agent control** (Settings → On-screen browser) freezes agent input; the live window stays interactive.
- **Let agent control** resumes — agent must `observe` before further `act`.
- **wait_human** shows a "Waiting for you" banner in the browser window; **Continue** (banner or Settings) finishes the tool.
- The Globe shows the window (or starts the browser when off); the **Power** button beside it stops the browser and closes the window.

## URL policy

Content tabs: `http://`, `https://`, `about:blank` only. `file:`, `javascript:`, and `koma://` are blocked.

### Popups (`window.open` / `target=_blank`)

wry only hands us the URL (no user-gesture bit, no window-features), so we approximate
real-browser intent without site-specific allowlists:

1. Never open a native OS window (handler always declines).
2. **Drop non-document auxiliary URLs** — do not create a tab for:
   - `about:blank` shells (need `window.opener`; a tab cannot replace that)
   - path segments starting with `_` (common internal RPC/frame shells: `/_/…`)
   - commercial tracker/ad network hosts and beacon path shapes
3. Otherwise open a **background tab** when Settings `popup_policy` is `new_tab`.
4. Settings `popup_policy: block` drops every page-initiated popup.

Real OAuth/login documents (`/signin`, `/oauth/authorize`, …) still open as tabs.
Tabs from the **+** control or the agent are unchanged.

## Capture

Observation frames are **Chrome-style visible-tab screenshots of the page content**
(the content webview only — no koma chrome, no OS titlebar, no overlapping apps):

| OS | Visible | Full page | Fallback |
|----|---------|-----------|----------|
| macOS | `WKWebView.takeSnapshot` | snapshot rect from document size | this window's backing store (`screencapture -l`), chrome cropped |
| Windows | WebView2 `CapturePreview` | CDP `Page.captureScreenshot` (`captureBeyondViewport`) | viewport `CapturePreview` |
| Linux | webkit2gtk `snapshot(Visible)` | `snapshot(FullDocument)` | viewport `Visible` |

On macOS, `takeSnapshot` retries once after a short pump; if a custom full-page
rect fails it retries the default viewport; if `afterScreenUpdates: true` fails it
retries `false`. The NSError description is kept on failure. Only then does
observe fall back to `screencapture -l` (`capture_quality: window`).

On Linux, chrome and page webviews are children of a `gtk::Fixed` packed into
tao's `default_vbox`. Attaching them directly to the vbox (`pack_start(expand=true)`)
splits the window — a dark gap under the toolbar, with the page starting halfway
down (and disappearing into that gap while loading). wry `set_bounds` only takes
effect inside `gtk::Fixed`. Linux/Windows round the **native window** (GTK shape /
`SetWindowRgn`); page HTML is not `clip-path`'d — that blanks WKWebView/WebView2
after first paint. Windows keeps the host opaque (layered + WebView2 = white page).
Desktop-region capture (`screencapture -R` / grim) is **not** used — it composites
whatever is on screen, so a koma chat window covering the browser would appear in
the PNG. Grant screen-recording permission only if the macOS window-id fallback is used.

## Page data and input

| Capability | How |
|------------|-----|
| `screen_browser_read` | JS eval with return value (markdown/text/html/query/actions_inventory) |
| `screen_browser_eval` | JS eval with JSON return (Settings → allow eval) |
| `screen_browser_act` | click / type / scroll / key / drag / wait via in-page DOM events |
| `screen_browser_observe` | visible-tab PNG + observation_id |

All three desktop OSes share the same wry `evaluate_script_with_callback` path for
read/eval/act confirmation.

### Act details

All pointer actions run as in-page JS in the tab webview (`run_actions` → eval →
`pump_brief`). There is no OS cursor, Accessibility click, or CDP `Input.dispatch*`.

- **selectors** use `querySelectorAll` (invalid CSS throws `invalid CSS selector: …`).
  `:contains()` is rejected in Rust before eval. When several nodes match, the
  first **visible** one wins (`getBoundingClientRect` ≥ 1×1, not
  `visibility:hidden` / `display:none` / `opacity:0`); otherwise the first match.
- **click / dblclick** (selector or coordinates): `scrollIntoView`, then
  `pointerdown` → `mousedown` → `pointerup` → `mouseup` → `click`, then
  `el.click()` when it exists. Right-click fires `contextmenu`; double-click
  adds `dblclick`. Coordinate clicks use the same sequence as selector clicks.
- **hover**: `pointermove` + `mouseover` + `mouseenter`.
- **type**: focuses first, sets value via the native
  `HTMLInputElement` / `HTMLTextAreaElement` setter, then `beforeinput` +
  `InputEvent('input', {inputType:'insertText'})` + `change`. `contenteditable`
  / no `value` falls back to `document.execCommand('insertText')`.
- **key** `Enter` / `NumpadEnter`: `keydown`/`keyup` (`keyCode` 13), then
  `form.requestSubmit()`, else click the first visible
  `button[type=submit], input[type=submit]`, else `el.click()` if it is a
  button. Enter is **not** PageDown. `End` / `Home` / `PageDown` / `PageUp` /
  `Space` / arrows still apply real scroll. Other keys dispatch `KeyboardEvent`.
- **drag**: needs a start (`selector` and/or `x,y`) and an end (`to_selector`
  and/or `to_x,to_y`). `pointerdown` at start, a few `pointermove`/`mousemove`
  steps, `pointerup` at end.
- **wait** with no `selector` and no `url_contains` is a delay of `timeout_ms`
  (default 5s). It always succeeds; the GUI loop is pumped so the page stays
  live. With a selector and/or `url_contains`, it polls until true or times out
  (a timeout drops the rest of the act batch).
- **scroll** `delta` is CSS pixels (default 400). `to: "bottom"` / `"top"`
  (case-insensitive) scroll that axis; any other `to` is a CSS selector for
  `scrollIntoView`. If the document root does not overflow, the largest
  `overflow-y: auto|scroll|overlay` descendant is used (capped at ~500 nodes).

## Always on top

Default **off**. Agent control uses the webview API and does **not** raise the
window or steal focus. Turn on **Settings → Always on top** only if you want the
browser pinned above other apps.

## Profile (cookies / logins)

Site data is **per project workdir** (`pwd_hash`), not per session and not global:

`~/.koma/sessions/<pwd_hash>/screen-browser/webview`

- Same PWD → same cookies/localStorage across session reopen and brand-new sessions.
- Different PWD → separate profile (no cross-project login bleed).
- **Tabs are not persisted.** Closing/stopping the browser drops tabs; only site
  data remains. Re-open starts on a blank tab with your previous logins intact.
- **Clear browsing data** wipes cookies/storage for the **current** project
  profile only.

Window placement is stored beside the profile as `window.json` under the same
`screen-browser/` folder.

## Not this feature

- Headless automation → `browser_tabs` / `browser_interact` / …
- Whole-desktop control → Computer use (`computer_*`)
