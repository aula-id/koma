import { create } from 'zustand'

export const UI_SCALES = [1, 1.5, 2, 2.5] as const
export type UiScale = typeof UI_SCALES[number]
export const UI_SCALE_KEY = 'koma.ui-scale'
export type UiScaleReply = { requestId: string; scale: number; error: string | null }

export function isUiScale(value: unknown): value is UiScale {
  return typeof value === 'number' && UI_SCALES.includes(value as UiScale)
}
export function readUiScale(): UiScale {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(UI_SCALE_KEY) ?? '1')
    return isUiScale(value) ? value : 1
  } catch { return 1 }
}

// Installation-local state: never replaced by session snapshots or remote pushes.
export const useUiScale = create<{ scale: UiScale; pending: boolean; error: string | null }>(() => ({
  scale: 1, pending: false, error: null,
}))
let sequence = 0
let reply: ((result: UiScaleReply) => void) | undefined

function nativeZoom(scale: UiScale): Promise<void> {
  return new Promise((resolve, reject) => {
    const requestId = `ui-scale-${++sequence}`
    // Late replies are still accepted: a timeout could let native and local state
    // disagree if the window thread was busy applying the requested zoom.
    reply = result => {
      if (result.requestId !== requestId) return
      reply = undefined
      if (result.error || result.scale !== scale) reject(new Error(result.error ?? 'Unexpected UI scale acknowledgement.'))
      else resolve()
    }
    window.__komaUiScaleReply = result => reply?.(result)
    try {
      const request: GuiReq = { r: 'SetUiScale', scale, requestId }
      window.ipc!.postMessage(JSON.stringify({ t: 'req', ...request }))
    } catch (error) { reply = undefined; reject(error) }
  })
}

export function browserZoom(): number {
  return window.ipc ? 1 : useUiScale.getState().scale
}
/** Convert browser viewport pixels into the zoomed page's CSS coordinates. */
export function pagePoint(value: number): number { return value / browserZoom() }
/** Authored canvases cancel UI zoom; native viewport coordinates still need conversion. */
export function canvasPoint(value: number): number {
  return value * useUiScale.getState().scale / browserZoom()
}
export function pageRect(rect: DOMRect): DOMRect {
  const zoom = browserZoom()
  return new DOMRect(rect.x / zoom, rect.y / zoom, rect.width / zoom, rect.height / zoom)
}

export async function setUiScale(scale: UiScale): Promise<boolean> {
  if (!isUiScale(scale) || useUiScale.getState().pending) return false
  useUiScale.setState({ pending: true, error: null })
  try {
    if (window.ipc) await nativeZoom(scale)
    else {
      document.documentElement.style.setProperty('--koma-browser-zoom', String(scale))
      document.documentElement.style.zoom = String(scale)
      // CSS zoom does not shrink viewport units/percentage root height.
      document.documentElement.style.height = `${100 / scale}%`
    }
    document.documentElement.style.setProperty('--koma-ui-scale', String(scale))
    useUiScale.setState({ scale, pending: false })
    try { localStorage.setItem(UI_SCALE_KEY, String(scale)) } catch { /* In-memory preference stays usable. */ }
    requestAnimationFrame(() => {
      window.dispatchEvent(new Event('koma-ui-scale'))
      window.dispatchEvent(new Event('resize'))
    })
    return true
  } catch (error) {
    useUiScale.setState({ pending: false, error: `Could not apply UI scale: ${error instanceof Error ? error.message : String(error)}` })
    return false
  }
}

export async function restoreUiScale(): Promise<void> {
  if (window.location.hash === '#computer-preview') return
  await setUiScale(readUiScale())
}
