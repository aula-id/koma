import { bytesToBase64 } from '../diagramNotes'
import { bytesToObjectUrl, requestFileBytes } from '../filePreview'
import { mintRequestId } from '../../store/coding'
import type { DesignDoc, DesignImageAsset, DesignImageScale, DesignNode } from './types'

const urls = new Map<string, string>()
const sizes = new Map<string, { w: number; h: number }>()

export function rememberImageSize(hash: string, w: number, h: number) {
  if (!hash || !(w > 0) || !(h > 0)) return
  sizes.set(hash, { w, h })
}

export function designImageSize(doc: DesignDoc | null | undefined, hash: string | null | undefined): { w: number; h: number } | null {
  if (!hash) return null
  const cached = sizes.get(hash)
  if (cached) return cached
  const asset = doc?.images?.[hash]
  if (asset?.w && asset.h && asset.w > 0 && asset.h > 0) return { w: asset.w, h: asset.h }
  const url = urls.get(hash)
  if (url && typeof Image !== 'undefined') {
    const image = new Image()
    image.src = url
    if (image.complete && image.naturalWidth > 0 && image.naturalHeight > 0) {
      rememberImageSize(hash, image.naturalWidth, image.naturalHeight)
      return { w: image.naturalWidth, h: image.naturalHeight }
    }
    image.onload = () => rememberImageSize(hash, image.naturalWidth, image.naturalHeight)
  }
  return null
}

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
  if (typeof Image !== 'undefined') {
    const image = new Image()
    image.onload = () => rememberImageSize(hash, image.naturalWidth, image.naturalHeight)
    image.src = url
  }
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

export async function hydrateDesignImages(
  doc: DesignDoc | null | undefined,
  req: (body: { r: 'FileDownloadBytes'; root: string; path: string; requestId: string }) => void,
  root: string,
): Promise<boolean> {
  if (!doc?.images) return false
  const jobs = Object.entries(doc.images).flatMap(([hash, asset]) => {
    if (!hash || !asset.path || urls.get(hash)) return []
    return [requestFileBytes(req, root, asset.path).then((bytes) => {
      cacheDesignImage(hash, bytes, asset.mime)
      return true
    }).catch(() => false)]
  })
  if (!jobs.length) return false
  const results = await Promise.all(jobs)
  return results.some(Boolean)
}

export function putDesignImage(doc: DesignDoc, hash: string, mime: string, path: string, size?: { w: number; h: number }): DesignDoc {
  const asset: DesignImageAsset = { mime, path }
  if (size && size.w > 0 && size.h > 0) {
    rememberImageSize(hash, size.w, size.h)
    asset.w = size.w
    asset.h = size.h
  }
  return { ...doc, version: 2, images: { ...(doc.images ?? {}), [hash]: asset } }
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
