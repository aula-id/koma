import { bytesToBase64 } from '../diagramNotes'
import { bytesToObjectUrl } from '../filePreview'
import { mintRequestId } from '../../store/coding'
import type { DesignDoc, DesignImageScale, DesignNode } from './types'

const urls = new Map<string, string>()

export function designAssetDir(): string {
  return '.koma/assets'
}

export function designAssetPath(hash: string, mime: string): string {
  return `${designAssetDir()}/${hash}${extForMime(mime)}`
}

export function extForMime(mime: string): string {
  if (mime === 'image/jpeg') return '.jpg'
  if (mime === 'image/webp') return '.webp'
  if (mime === 'image/gif') return '.gif'
  if (mime === 'image/svg+xml') return '.svg'
  if (mime === 'image/bmp') return '.bmp'
  return '.png'
}

export function mimeForName(name: string): string | null {
  if (/\.png$/i.test(name)) return 'image/png'
  if (/\.jpe?g$/i.test(name)) return 'image/jpeg'
  if (/\.webp$/i.test(name)) return 'image/webp'
  if (/\.gif$/i.test(name)) return 'image/gif'
  if (/\.svg$/i.test(name)) return 'image/svg+xml'
  if (/\.bmp$/i.test(name)) return 'image/bmp'
  return null
}

export async function hashBytes(bytes: Uint8Array): Promise<string> {
  if (globalThis.crypto?.subtle) {
    const copy = new Uint8Array(bytes.byteLength)
    copy.set(bytes)
    const digest = await crypto.subtle.digest('SHA-256', copy.buffer)
    return [...new Uint8Array(digest)].map((item) => item.toString(16).padStart(2, '0')).join('')
  }
  let hash = 2166136261
  for (let i = 0; i < bytes.length; i++) hash = Math.imul(hash ^ bytes[i], 16777619)
  const hex = (hash >>> 0).toString(16).padStart(8, '0')
  return hex.repeat(8).slice(0, 64)
}

export function cacheDesignImage(hash: string, bytes: Uint8Array, mime: string): string {
  const prev = urls.get(hash)
  if (prev) URL.revokeObjectURL(prev)
  const url = bytesToObjectUrl(bytes, mime)
  urls.set(hash, url)
  return url
}

export function designImageUrl(hash: string | null | undefined): string {
  if (!hash) return ''
  return urls.get(hash) ?? ''
}

export function rememberDesignImages(doc: DesignDoc | null | undefined) {
  if (!doc?.images) return
  for (const hash of Object.keys(doc.images)) {
    if (!urls.has(hash)) urls.set(hash, '')
  }
}

export function putDesignImage(doc: DesignDoc, hash: string, mime: string, path: string): DesignDoc {
  return { ...doc, version: 2, images: { ...(doc.images ?? {}), [hash]: { mime, path } } }
}

export function applyImageFill(node: DesignNode, hash: string, scale: DesignImageScale = 'fill'): DesignNode {
  const paint = { type: 'image' as const, hash, scale }
  return { ...node, fills: [paint], fill: undefined }
}

export function createImageRect(id: string, x: number, y: number, w: number, h: number, hash: string): DesignNode {
  return applyImageFill({ id, kind: 'rect', name: 'Image', x, y, w: Math.max(1, w), h: Math.max(1, h), stroke: 'none' }, hash)
}

export function writeDesignAsset(
  req: (body: { r: 'FileWriteBytes'; root: string; path: string; bytesB64: string; overwrite?: boolean; requestId: string }) => void,
  root: string,
  path: string,
  bytes: Uint8Array,
) {
  req({
    r: 'FileWriteBytes',
    root,
    path,
    bytesB64: bytesToBase64(bytes),
    overwrite: true,
    requestId: mintRequestId(),
  })
}

export async function imageNaturalSize(bytes: Uint8Array, mime: string): Promise<{ w: number; h: number }> {
  if (typeof createImageBitmap === 'function') {
    const copy = new Uint8Array(bytes.byteLength)
    copy.set(bytes)
    const bitmap = await createImageBitmap(new Blob([copy], { type: mime }))
    const size = { w: bitmap.width, h: bitmap.height }
    bitmap.close()
    return size
  }
  return { w: 200, h: 200 }
}
