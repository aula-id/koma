/** Load workspace file bytes via FileDownloadBytes without forcing a save-as. */

type Pending = {
  resolve: (bytes: Uint8Array) => void
  reject: (e: Error) => void
  timer: ReturnType<typeof setTimeout>
}

const PENDING_MS = 30_000
const pending = new Map<string, Pending>()

let seq = 0
function mintId(): string {
  seq = (seq + 1) % 1_000_000
  return `fp-${Date.now().toString(36)}-${seq}`
}

type ReqFn = (body: {
  r: 'FileDownloadBytes'
  root: string
  path: string
  requestId: string
}) => void

/** True when a preview waiter owns this requestId (store should not save-as). */
export function hasFilePreviewWaiter(requestId: string): boolean {
  return pending.has(requestId)
}

/** Called from the store FileDownloadBytes reducer. Returns true if consumed. */
export function resolveFilePreviewBytes(
  requestId: string,
  bytesB64: string | null | undefined,
  error: string | null | undefined,
  tooLarge?: boolean,
): boolean {
  const p = pending.get(requestId)
  if (!p) return false
  pending.delete(requestId)
  clearTimeout(p.timer)
  if (error) {
    p.reject(new Error(error))
    return true
  }
  if (tooLarge) {
    p.reject(new Error('file too large to preview'))
    return true
  }
  if (!bytesB64) {
    p.reject(new Error('empty download'))
    return true
  }
  try {
    const bin = atob(bytesB64)
    const bytes = new Uint8Array(bin.length)
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
    p.resolve(bytes)
  } catch (e) {
    p.reject(e instanceof Error ? e : new Error(String(e)))
  }
  return true
}

export function requestFileBytes(
  req: ReqFn,
  root: string,
  path: string,
): Promise<Uint8Array> {
  const requestId = mintId()
  return new Promise<Uint8Array>((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(requestId)
      reject(new Error('preview load timed out'))
    }, PENDING_MS)
    pending.set(requestId, { resolve, reject, timer })
    req({ r: 'FileDownloadBytes', root, path, requestId })
  })
}

export function bytesToObjectUrl(bytes: Uint8Array, mime: string): string {
  // Copy into a fresh ArrayBuffer-backed view — Blob rejects SharedArrayBuffer views.
  const copy = new Uint8Array(bytes.byteLength)
  copy.set(bytes)
  const blob = new Blob([copy.buffer], { type: mime })
  return URL.createObjectURL(blob)
}
