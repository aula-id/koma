// GUI-side half of the W8/W9 extension panel bridge. Panel iframes all load
// off the SINGLE origin `koma://extension` (see ExtensionPanelFrame.tsx's
// no-sandbox comment) — the extId is a path segment, not a separate
// authority — so a hostile or buggy panel could claim to be ANY extId/panelId
// in its message payload. Attribution therefore NEVER trusts message
// content: every inbound `message` event is resolved through `bySource`,
// keyed by the iframe's actual `Window` object (captured at
// registerPanelFrame time, straight off `iframe.contentWindow`). `origin`
// checks below are defense-in-depth only, not the security boundary.
//
// Wire contract (mirrors the Rust GuiReq::ExtPanelMsg / PushEnvelope
// ExtPanelReply/ExtPanelPush in src-agent/src/app/runtime/gui/proto.rs and
// client/push_proto.rs):
//   panel -> host: { koma: 'panel', v: 1, kind: 'msg', reqId: string, payload: unknown }
//   host -> panel: { koma: 'host', v: 1, kind: 'reply', reqId: string | null, ok: boolean, payload?: unknown, error?: string }
//   host -> panel: { koma: 'host', v: 1, kind: 'push', payload: unknown }
//
// Theme sub-contract (see docs/EXTENSIONS.md "Theme"): a DISTINCT top-level
// `kind` (not 'push') so panels never confuse a host-initiated theme repaint
// with an extension-originated push. Broadcast on every palette change AND
// once on panel registration (a panel that mounts mid-session must not wait
// for the next theme switch to paint itself correctly); also answerable as a
// pure UI query, detached-tolerant (no active koma session required):
//   host -> panel: { koma: 'host', v: 1, kind: 'theme', payload: { palette, name, dark } }
//   panel -> host: { koma: 'panel', v: 1, kind: 'theme?', reqId: string }
//   host -> panel: { koma: 'host', v: 1, kind: 'reply', reqId, ok: true, payload: { palette, name, dark } }

import { useKoma, type PaletteColors } from '../store/koma'

const byKey = new Map<string, Window>()
const bySource = new Map<Window, { extId: string; panelId: string }>()

function keyOf(extId: string, panelId: string): string {
  return `${extId}:${panelId}`
}

export function registerPanelFrame(extId: string, panelId: string, win: Window): void {
  const key = keyOf(extId, panelId)
  const prev = byKey.get(key)
  // A reload gets a fresh `contentWindow` proxy — drop the stale reverse
  // mapping first so the old Window object never lingers in `bySource`.
  if (prev) bySource.delete(prev)
  byKey.set(key, win)
  bySource.set(win, { extId, panelId })
  // Deliver the current theme immediately — a panel registered mid-session
  // (opened after the last broadcast, or reloaded) must not wait for the
  // next palette change to paint itself with the right colours.
  postToPanel(extId, panelId, themePush())
}

export function unregisterPanelFrame(extId: string, panelId: string): void {
  const key = keyOf(extId, panelId)
  const win = byKey.get(key)
  if (win) bySource.delete(win)
  byKey.delete(key)
}

// Nominal targetOrigin for panel postMessage calls. WebKitGTK's handling of
// custom-scheme targetOrigin matching against `koma://extension/<extId>/…`
// frames is unverified — it may or may not match. '*' is the sanctioned
// fallback specifically BECAUSE the target Window comes out of our own
// registry (never off message content), so a wildcard target can't leak
// this postMessage to an attacker-controlled frame.
const PANEL_TARGET_ORIGIN = 'koma://extension'
// Once a `*` retry has succeeded, stop bothering with the scheme origin —
// remember it module-wide rather than re-probing every call.
let useWildcardTarget = false

// Shared postMessage-with-retry core — `postToPanel` looks a Window up by
// key first; `broadcastThemeToPanels` already has the live Window objects
// (from `byKey.values()`) and posts to them directly, skipping the
// key round-trip (and the ':' key-format assumption `keyOf` makes).
function sendToWindow(win: Window, msg: unknown): boolean {
  const targetOrigin = useWildcardTarget ? '*' : PANEL_TARGET_ORIGIN
  try {
    win.postMessage(msg, targetOrigin)
    return true
  } catch {
    if (targetOrigin === '*') return false
    try {
      win.postMessage(msg, '*')
      useWildcardTarget = true
      return true
    } catch {
      return false
    }
  }
}

export function postToPanel(extId: string, panelId: string, msg: unknown): boolean {
  const win = byKey.get(keyOf(extId, panelId))
  if (!win) return false
  return sendToWindow(win, msg)
}

// Builds the { palette, name, dark } payload shared by the 'theme' push and
// the 'theme?' query reply, reading whatever the store currently holds. Before
// the first palette ever lands, `state.palette` is still `initialPalette`
// (the store's dark default) — so an unpushed/never-attached panel gets a
// sane default instead of `undefined`.
function currentThemePayload(): { palette: PaletteColors; name: string; dark: boolean } {
  const { palette, config } = useKoma.getState()
  return { palette, name: config.theme, dark: palette.dark ?? true }
}

function themePush(): { koma: 'host'; v: 1; kind: 'theme'; payload: ReturnType<typeof currentThemePayload> } {
  return { koma: 'host', v: 1, kind: 'theme', payload: currentThemePayload() }
}

// Broadcasts the current theme to every registered panel. Called from the
// store's `applyPaletteVars` choke point (store/koma.ts) right after the CSS
// vars are repainted, so live panels track the daemon's theme exactly like
// the chat chrome does.
export function broadcastThemeToPanels(palette: PaletteColors): void {
  if (byKey.size === 0) return
  const name = useKoma.getState().config.theme
  const msg = { koma: 'host', v: 1, kind: 'theme', payload: { palette, name, dark: palette.dark ?? true } }
  for (const win of byKey.values()) sendToWindow(win, msg)
}

// Panel request payloads are capped well under any reasonable IPC frame
// size — a panel with a legitimately large body should chunk it itself.
const MAX_PAYLOAD_CHARS = 262144

let listenerInstalled = false
const warnedOrigins = new Set<string>()
let loggedPanelOrigin = false

type PanelMessageEventLike = {
  source: unknown
  origin: string
  data: unknown
}

// Exported for testability — the real listener just forwards the native
// `MessageEvent` here.
export function handlePanelMessage(event: PanelMessageEventLike): void {
  const source = event.source as Window | null
  const attribution = source ? bySource.get(source) : undefined
  if (!attribution) return // not a registered panel — chrome gets lots of unrelated `message` traffic

  const { extId, panelId } = attribution

  // Defense-in-depth only (see module comment): attribution above already
  // comes from the registry, never from `event.origin` or `event.data`. This
  // just flags anomalies and drops obviously-wrong origins. Tolerate falsy
  // origins (observed on WebKitGTK for the custom `koma:` scheme) and the
  // chrome's own origin (not expected, but not evidence of spoofing either).
  const chromeOrigin = typeof window !== 'undefined' && window.location ? window.location.origin : ''
  if (event.origin && event.origin !== chromeOrigin) {
    if (!event.origin.startsWith('koma://extension')) {
      if (!warnedOrigins.has(event.origin)) {
        warnedOrigins.add(event.origin)
        console.warn(`panelBridge: dropped message with unexpected origin: ${event.origin}`)
      }
      return
    }
    if (!loggedPanelOrigin) {
      loggedPanelOrigin = true
      console.info(`panelBridge: observed panel origin: ${event.origin}`)
    }
  }

  const data = event.data as
    | { koma?: unknown; v?: unknown; kind?: unknown; reqId?: unknown; payload?: unknown }
    | null
    | undefined
  if (!data || data.koma !== 'panel' || data.v !== 1 || typeof data.reqId !== 'string') {
    return // not a recognized panel->host request — ignore
  }
  const reqId = data.reqId

  // Pure UI query for the current theme — answered synchronously off the
  // store, BEFORE the attached-session gate below: a panel on the detached
  // welcome screen still needs to paint itself, and this never touches the
  // daemon.
  if (data.kind === 'theme?') {
    postToPanel(extId, panelId, {
      koma: 'host',
      v: 1,
      kind: 'reply',
      reqId,
      ok: true,
      payload: currentThemePayload(),
    })
    return
  }

  if (data.kind !== 'msg') {
    return // not a recognized panel->host request — ignore
  }
  const payload = data.payload

  let payloadChars = 0
  try {
    payloadChars = JSON.stringify(payload)?.length ?? 0
  } catch {
    payloadChars = Number.POSITIVE_INFINITY
  }
  if (payloadChars > MAX_PAYLOAD_CHARS) {
    postToPanel(extId, panelId, {
      koma: 'host',
      v: 1,
      kind: 'reply',
      reqId,
      ok: false,
      error: 'payload too large',
    })
    return
  }

  // Mirrors the GuiReq::ExtPanelMsg host-side guard comment: attached-only,
  // with no attached daemon the GUI side replies locally instead of dropping
  // silently.
  if (useKoma.getState().session.id === null) {
    postToPanel(extId, panelId, {
      koma: 'host',
      v: 1,
      kind: 'reply',
      reqId,
      ok: false,
      error: 'no active koma session',
    })
    return
  }

  useKoma.getState().req({ r: 'ExtPanelMsg', extId, panelId, reqId, payload })
}

// Idempotent: installs the single window-level `message` listener that
// attributes + forwards panel traffic. Safe to call from more than one
// mount — only the first call actually installs it. Returns a cleanup that
// removes the listener and clears the installed flag.
export function installPanelBridgeListener(): () => void {
  if (listenerInstalled) return () => {}
  listenerInstalled = true
  const listener = (event: MessageEvent) => handlePanelMessage(event)
  window.addEventListener('message', listener)
  return () => {
    window.removeEventListener('message', listener)
    listenerInstalled = false
  }
}
