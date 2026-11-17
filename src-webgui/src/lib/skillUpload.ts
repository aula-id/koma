// Keep these limits aligned with src-agent/src/model/skill/persistence.rs
// (MAX_SKILL_ZIP_BYTES, MAX_SKILL_UNPACKED_BYTES, MAX_SKILL_ZIP_COMPANION_BYTES).
export const MAX_SKILL_ZIP_BYTES = 256 * 1024 * 1024
export const MAX_SKILL_ZIP_LABEL = '256 MiB'
export const UPLOAD_CHUNK_BYTES = 4 * 1024 * 1024
export const INSTALL_CONFIRM_MS = 180_000

/** Byte ranges `[offset, end)` that cover `size` in order; a zero-byte file has none. */
export function uploadChunks(size: number, chunk = UPLOAD_CHUNK_BYTES): Array<{ offset: number; end: number }> {
  const out: Array<{ offset: number; end: number }> = []
  for (let offset = 0; offset < size; offset += chunk) out.push({ offset, end: Math.min(offset + chunk, size) })
  return out
}

/** Whole-number percentage 0..100 of `total` already sent. */
export function uploadProgress(sentBytes: number, total: number): number {
  if (total <= 0) return 0
  return Math.max(0, Math.min(100, Math.round((sentBytes / total) * 100)))
}
