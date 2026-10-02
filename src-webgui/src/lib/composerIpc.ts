// Composer draft ↔ IPC wire text (TUI-parity markers and @ refs).

import { imageMarker } from './composerMarkers'
import { pasteMarker } from './pasteText'

export type ComposerChipKind = 'file' | 'image' | 'paste'

export type ComposerChipPayload = {
  kind: ComposerChipKind
  wireText: string
  displayLabel: string
  markerN?: number | null
  queueId?: string | null
}

const IMAGE_WIRE = /^\[Image #(\d+)\]$/
const PASTE_WIRE = /^\[Pasted Text #(\d+)\]$/
const FILE_SENTINEL = /^@\[\d+\]\S+$/
const FILE_PLAIN = /^@\S+$/

export function chipKindFromWire(wire: string): ComposerChipKind | null {
  if (IMAGE_WIRE.test(wire)) return 'image'
  if (PASTE_WIRE.test(wire)) return 'paste'
  if (FILE_SENTINEL.test(wire) || FILE_PLAIN.test(wire)) return 'file'
  return null
}

export function markerNFromWire(wire: string): number | null {
  const image = wire.match(IMAGE_WIRE)
  if (image) return Number(image[1])
  const paste = wire.match(PASTE_WIRE)
  if (paste) return Number(paste[1])
  return null
}

/** Human-facing chip label (no `@` or bracket markers). */
export function displayLabelForWire(wire: string): string {
  const kind = chipKindFromWire(wire)
  if (kind === 'image') {
    const n = markerNFromWire(wire)
    return n != null ? `Image ${n}` : 'Image'
  }
  if (kind === 'paste') {
    const n = markerNFromWire(wire)
    return n != null ? `Paste ${n}` : 'Paste'
  }
  if (kind === 'file') {
    const body = wire.startsWith('@') ? wire.slice(1) : wire
    const withoutRoot = body.replace(/^\[\d+\]/, '')
    const path = withoutRoot.replace(/\\/g, '/')
    const base = path.split('/').filter(Boolean).pop()
    return base || withoutRoot || wire
  }
  return wire
}

export function chipPayloadFromWire(wire: string, opts?: { queueId?: string | null }): ComposerChipPayload {
  const kind = chipKindFromWire(wire) ?? 'file'
  return {
    kind,
    wireText: wire,
    displayLabel: displayLabelForWire(wire),
    markerN: markerNFromWire(wire),
    queueId: opts?.queueId ?? null,
  }
}

export function chipPayloadForFileRef(wireText: string): ComposerChipPayload {
  const trimmed = wireText.trim()
  return chipPayloadFromWire(trimmed.startsWith('@') ? trimmed : `@${trimmed}`)
}

export function chipPayloadForAttachMarker(
  kind: 'image' | 'paste' | 'pasted_text',
  markerN: number,
  queueId?: string | null,
): ComposerChipPayload {
  const chipKind = kind === 'image' ? 'image' : 'paste'
  const wireText = chipKind === 'image' ? imageMarker(markerN) : pasteMarker(markerN)
  return {
    kind: chipKind,
    wireText,
    displayLabel: displayLabelForWire(wireText),
    markerN,
    queueId: queueId ?? null,
  }
}

export const COMPOSER_ATTACHMENT_MIME = 'application/x-koma-attachment'
export const COMPOSER_ATTACHMENT_PLAIN_PREFIX = 'koma-attachment:'

export type ComposerAttachmentDrag = {
  kind: 'image' | 'pasted_text'
  markerN: number
}

export function writeComposerAttachmentDrag(data: ComposerAttachmentDrag): string {
  return JSON.stringify(data)
}

export function readComposerAttachmentDrag(
  raw: string | null | undefined,
  plain?: string | null,
): ComposerAttachmentDrag | null {
  if (raw) {
    try {
      const parsed = JSON.parse(raw) as Partial<ComposerAttachmentDrag>
      if ((parsed.kind === 'image' || parsed.kind === 'pasted_text') && Number.isFinite(parsed.markerN)) {
        return { kind: parsed.kind, markerN: Number(parsed.markerN) }
      }
    } catch {
      /* fall through to plain */
    }
  }
  if (plain?.startsWith(COMPOSER_ATTACHMENT_PLAIN_PREFIX)) {
    const rest = plain.slice(COMPOSER_ATTACHMENT_PLAIN_PREFIX.length)
    const colon = rest.lastIndexOf(':')
    if (colon > 0) {
      const kind = rest.slice(0, colon)
      const markerN = Number(rest.slice(colon + 1))
      if ((kind === 'image' || kind === 'pasted_text') && Number.isFinite(markerN)) {
        return { kind, markerN }
      }
    }
  }
  return null
}

export function composerAttachmentPlain(data: ComposerAttachmentDrag): string {
  return `${COMPOSER_ATTACHMENT_PLAIN_PREFIX}${data.kind}:${data.markerN}`
}

export function hasComposerAttachmentDrag(types: readonly string[]): boolean {
  return types.includes(COMPOSER_ATTACHMENT_MIME)
}

/** After markdown import, promote plain @ refs in text nodes to chips. */
export function findFileRefWireInText(text: string): string[] {
  const found: string[] = []
  const sentinel = /@\[\d+\]\S+/g
  let m: RegExpExecArray | null
  while ((m = sentinel.exec(text))) found.push(m[0])
  const plain = /@\S+/g
  while ((m = plain.exec(text))) {
    const hit = m[0]
    if (found.some((item) => item.includes(hit) || hit.includes(item))) continue
    if (!found.includes(hit)) found.push(hit)
  }
  return found.sort((a, b) => b.length - a.length)
}
