import type { DesignDoc, DesignNode, DesignPaint, DesignPaintStop, DesignRef } from './types'
import { designImageUrl } from './assets'

function tokenColor(doc: DesignDoc, ref: string): string {
  if (!ref || ref === 'none' || ref.startsWith('#')) return ref
  const token = doc.tokens?.find((item) => item.name === ref)
  if (!token) return ''
  return token.values[doc.mode] ?? token.values[doc.modes?.[0] ?? ''] ?? Object.values(token.values)[0] ?? ''
}

export function solidPaint(color: DesignRef, extras?: Partial<DesignPaint>): DesignPaint {
  return { type: 'solid', color, ...extras }
}

export function clonePaint(paint: DesignPaint): DesignPaint {
  const next: DesignPaint = { type: paint.type }
  if (paint.visible === false) next.visible = false
  if (paint.opacity != null) next.opacity = paint.opacity
  if (paint.blend) next.blend = paint.blend
  if (paint.color) next.color = paint.color
  if (paint.kind) next.kind = paint.kind
  if (paint.stops) next.stops = paint.stops.map((stop) => ({ color: stop.color, at: stop.at }))
  if (paint.transform) next.transform = paint.transform.slice()
  if (paint.hash) next.hash = paint.hash
  if (paint.scale) next.scale = paint.scale
  if (paint.imageX != null) next.imageX = paint.imageX
  if (paint.imageY != null) next.imageY = paint.imageY
  if (paint.imageW != null) next.imageW = paint.imageW
  if (paint.imageH != null) next.imageH = paint.imageH
  if (paint.width != null) next.width = paint.width
  if (paint.align) next.align = paint.align
  if (paint.dash != null) next.dash = paint.dash
  if (paint.gap != null) next.gap = paint.gap
  if (paint.capStart) next.capStart = paint.capStart
  if (paint.capEnd) next.capEnd = paint.capEnd
  if (paint.join) next.join = paint.join
  if (paint.markerStart) next.markerStart = paint.markerStart
  if (paint.markerEnd) next.markerEnd = paint.markerEnd
  if (paint.top != null) next.top = paint.top
  if (paint.right != null) next.right = paint.right
  if (paint.bottom != null) next.bottom = paint.bottom
  if (paint.left != null) next.left = paint.left
  return next
}

export function isPaintVisible(paint: DesignPaint | null | undefined): boolean {
  return !!paint && paint.visible !== false
}

export function nodePaints(node: DesignNode | null | undefined, field: 'fill' | 'stroke'): DesignPaint[] {
  if (!node) return []
  const list = field === 'fill' ? node.fills : node.strokes
  if (list?.length) return list.map(clonePaint)
  const alias = field === 'fill' ? node.fill : node.stroke
  if (alias == null) return []
  return [solidPaint(alias)]
}

export function nodeHasPaint(node: DesignNode | null | undefined, field: 'fill' | 'stroke'): boolean {
  return nodePaints(node, field).some((paint) => {
    if (paint.type === 'image' || paint.type === 'gradient') return true
    return !!(paint.color && paint.color !== 'none')
  })
}

export function firstVisiblePaint(paints: DesignPaint[] | null | undefined): DesignPaint | null {
  if (!paints?.length) return null
  return paints.find((paint) => paint.visible !== false) ?? null
}

/** String chrome for the first visible paint. Gradients and images return empty so the theme can fill in. */
export function paintAlias(paint: DesignPaint | null | undefined): string {
  if (!paint || paint.visible === false) return 'none'
  if (paint.type === 'solid') return paint.color && paint.color !== 'none' ? paint.color : 'none'
  return ''
}

export function syncPaintFields(node: DesignNode): DesignNode {
  const next: DesignNode = { ...node }
  if (node.fills?.length) {
    const first = firstVisiblePaint(node.fills)
    const alias = paintAlias(first)
    if (alias && alias !== 'none') next.fill = alias
    else if (alias === 'none') next.fill = 'none'
    else delete next.fill
  }
  if (node.strokes?.length) {
    const first = firstVisiblePaint(node.strokes)
    const alias = paintAlias(first)
    if (alias && alias !== 'none') next.stroke = alias
    else if (alias === 'none') next.stroke = 'none'
    else delete next.stroke
  }
  return next
}

export function setNodePaints(node: DesignNode, field: 'fill' | 'stroke', paints: DesignPaint[] | null): DesignNode {
  const next: DesignNode = { ...node }
  if (field === 'fill') {
    if (paints?.length) next.fills = paints.map(clonePaint)
    else delete next.fills
  } else if (paints?.length) next.strokes = paints.map(clonePaint)
  else delete next.strokes
  const alias = paintAlias(firstVisiblePaint(paints))
  if (field === 'fill') {
    if (alias && alias !== 'none' && alias !== '') next.fill = alias
    else if (alias === 'none') next.fill = 'none'
    else delete next.fill
  } else if (alias && alias !== 'none' && alias !== '') next.stroke = alias
  else if (alias === 'none') next.stroke = 'none'
  else delete next.stroke
  return next
}

export function setNodeSolid(node: DesignNode, field: 'fill' | 'stroke', color: string | null): DesignNode {
  if (color == null) {
    const next: DesignNode = { ...node }
    if (field === 'fill') {
      delete next.fill
      delete next.fills
    } else {
      delete next.stroke
      delete next.strokes
    }
    return next
  }
  return setNodePaints(node, field, [solidPaint(color)])
}

export function appendNodePaint(node: DesignNode, field: 'fill' | 'stroke', paint: DesignPaint, implicit?: DesignPaint): DesignNode {
  const existing = nodePaints(node, field)
  const base = existing.length ? existing : implicit ? [clonePaint(implicit)] : []
  return setNodePaints(node, field, [...base, clonePaint(paint)])
}

function hexRgba(hex: string, opacity: number): string {
  const body = hex.slice(1)
  const r = Number.parseInt(body.slice(0, 2), 16)
  const g = Number.parseInt(body.slice(2, 4), 16)
  const b = Number.parseInt(body.slice(4, 6), 16)
  return `rgba(${r}, ${g}, ${b}, ${Math.round(opacity * 1000) / 1000})`
}

function cssWithOpacity(css: string, opacity: number | undefined): string {
  if (opacity == null || opacity >= 1) return css
  if (/^#[0-9a-fA-F]{6}$/i.test(css)) return hexRgba(css, opacity)
  return css
}

export function resolveColor(doc: DesignDoc, ref: string | null | undefined, fallback = ''): string {
  if (ref == null || ref === '' || ref === 'none') return fallback
  const resolved = tokenColor(doc, ref)
  if (!resolved || resolved === 'none') return fallback
  if (resolved.startsWith('#')) return resolved
  if (/^\d+(\.\d+)?$/.test(resolved)) return fallback
  return resolved.startsWith('#') ? resolved : fallback
}

function stopCss(doc: DesignDoc, stop: DesignPaintStop): string {
  const color = resolveColor(doc, stop.color, '#000000')
  const at = Number.isFinite(stop.at) ? Math.max(0, Math.min(1, stop.at)) : 0
  return `${color} ${Math.round(at * 1000) / 10}%`
}

export function paintGradientAngle(paint: DesignPaint): number {
  const angle = paint.transform?.[0]
  return Number.isFinite(angle) ? ((angle % 360) + 360) % 360 : 180
}

export function paintGradientCenter(paint: DesignPaint): { x: number; y: number } {
  return {
    x: Number.isFinite(paint.transform?.[1]) ? paint.transform![1] : 0.5,
    y: Number.isFinite(paint.transform?.[2]) ? paint.transform![2] : 0.5,
  }
}

function gradientCss(doc: DesignDoc, paint: DesignPaint): string {
  const stops = (paint.stops?.length ? paint.stops : [{ color: paint.color ?? '#000000', at: 0 }, { color: paint.color ?? '#ffffff', at: 1 }])
    .slice()
    .sort((a, b) => a.at - b.at)
    .map((stop) => stopCss(doc, stop))
    .join(', ')
  const kind = paint.kind ?? 'linear'
  const center = paintGradientCenter(paint)
  const at = `${Math.round(center.x * 1000) / 10}% ${Math.round(center.y * 1000) / 10}%`
  if (kind === 'radial' || kind === 'diamond') return `radial-gradient(circle at ${at}, ${stops})`
  if (kind === 'angular') return `conic-gradient(from 0deg at ${at}, ${stops})`
  return `linear-gradient(${paintGradientAngle(paint)}deg, ${stops})`
}

export function imageAssetUrl(doc: DesignDoc, hash: string | null | undefined): string {
  if (!hash) return ''
  const cached = designImageUrl(hash)
  if (cached) return cached
  const asset = doc.images?.[hash]
  return asset?.path ? `koma-asset:${asset.path}` : ''
}

export function resolvePaintCss(doc: DesignDoc, paint: DesignPaint | null | undefined, fallback = 'transparent'): string {
  if (!paint || paint.visible === false) return 'transparent'
  if (paint.type === 'gradient') return cssWithOpacity(gradientCss(doc, paint), paint.opacity)
  if (paint.type === 'image') {
    const url = imageAssetUrl(doc, paint.hash)
    if (!url) return fallback
    return `url("${url}")`
  }
  return cssWithOpacity(resolveColor(doc, paint.color, fallback) || fallback, paint.opacity)
}

export function resolvePaintHex(doc: DesignDoc, paint: DesignPaint | null | undefined, fallback = ''): string {
  if (!paint || paint.visible === false) return fallback
  if (paint.type === 'gradient') {
    const stop = paint.stops?.find((item) => item.color) ?? paint.stops?.[0]
    return resolveColor(doc, stop?.color ?? paint.color, fallback)
  }
  if (paint.type === 'image') return fallback
  return resolveColor(doc, paint.color, fallback)
}

export function resolvePaintLayer(doc: DesignDoc, paint: DesignPaint, fallback: string): string {
  const css = resolvePaintCss(doc, paint, fallback)
  if (paint.type === 'image' || paint.type === 'gradient') return css
  return `linear-gradient(${css}, ${css})`
}

export function nodeFillLayers(doc: DesignDoc, node: DesignNode, fallback: string): string[] {
  return nodePaints(node, 'fill')
    .filter((paint) => isPaintVisible(paint) && paintAlias(paint) !== 'none')
    .map((paint) => resolvePaintLayer(doc, paint, fallback))
}

export function nodeFillCss(doc: DesignDoc, node: DesignNode, fallback: string): string {
  const layers = nodeFillLayers(doc, node, fallback)
  if (!layers.length) {
    const first = firstVisiblePaint(nodePaints(node, 'fill'))
    if (first && paintAlias(first) === 'none') return 'transparent'
    return fallback
  }
  return layers.join(', ')
}

export function nodeStrokeCss(doc: DesignDoc, node: DesignNode, fallback: string): string {
  const paints = nodePaints(node, 'stroke')
  const first = firstVisiblePaint(paints)
  if (!first) return fallback
  if (paintAlias(first) === 'none') return 'transparent'
  return resolvePaintCss(doc, first, fallback)
}

export function strokePaintWidth(paint: DesignPaint | null | undefined, fallback = 1): number {
  return paint?.width != null && paint.width > 0 ? paint.width : fallback
}

export function strokePaintDash(paint: DesignPaint | null | undefined, fallback?: number[]): string | undefined {
  if (paint?.dash != null && paint.dash > 0) return `${paint.dash} ${paint.gap ?? paint.dash}`
  if (fallback?.length) return fallback.join(' ')
  return undefined
}

export function hexToRgba(hex: string): { r: number; g: number; b: number; a: number } {
  const body = hex.startsWith('#') ? hex.slice(1) : hex
  const full = body.length === 3 ? body.split('').map((item) => item + item).join('') : body.slice(0, 8)
  return {
    r: Number.parseInt(full.slice(0, 2), 16) || 0,
    g: Number.parseInt(full.slice(2, 4), 16) || 0,
    b: Number.parseInt(full.slice(4, 6), 16) || 0,
    a: full.length >= 8 ? (Number.parseInt(full.slice(6, 8), 16) || 0) / 255 : 1,
  }
}

export function rgbaToHex(r: number, g: number, b: number): string {
  const hex = (n: number) => Math.max(0, Math.min(255, Math.round(n))).toString(16).padStart(2, '0')
  return `#${hex(r)}${hex(g)}${hex(b)}`
}

export function defaultGradient(kind: NonNullable<DesignPaint['kind']> = 'linear'): DesignPaint {
  return {
    type: 'gradient',
    kind,
    stops: [
      { color: '#4f46e5', at: 0 },
      { color: '#22d3ee', at: 1 },
    ],
  }
}

export function imagePaint(hash: string, scale: NonNullable<DesignPaint['scale']> = 'fill'): DesignPaint {
  return { type: 'image', hash, scale }
}

export function isImageCropPaint(paint: DesignPaint | null | undefined): boolean {
  return !!paint && paint.type === 'image' && paint.visible !== false && !!paint.hash && paint.scale === 'crop'
}

export function firstImageFill(node: DesignNode | null | undefined): DesignPaint | null {
  return nodePaints(node, 'fill').find((paint) => paint.type === 'image' && paint.visible !== false && !!paint.hash) ?? null
}

export function nodeHasImageFill(node: DesignNode | null | undefined): boolean {
  return firstImageFill(node) != null
}

export function imageCropRect(
  box: { w: number; h: number },
  paint: DesignPaint,
  natural?: { w: number; h: number } | null,
): { x: number; y: number; w: number; h: number } {
  if (paint.imageW != null && paint.imageH != null && paint.imageW > 0 && paint.imageH > 0) {
    return { x: paint.imageX ?? 0, y: paint.imageY ?? 0, w: paint.imageW, h: paint.imageH }
  }
  const nw = natural?.w ?? 0
  const nh = natural?.h ?? 0
  if (nw > 0 && nh > 0 && box.w > 0 && box.h > 0) {
    const scale = Math.max(box.w / nw, box.h / nh)
    const w = nw * scale
    const h = nh * scale
    return { x: (box.w - w) / 2, y: (box.h - h) / 2, w, h }
  }
  return { x: 0, y: 0, w: Math.max(1, box.w), h: Math.max(1, box.h) }
}

export function ensureImageCrop(
  paint: DesignPaint,
  box: { w: number; h: number },
  natural?: { w: number; h: number } | null,
): DesignPaint {
  if (paint.type !== 'image') return paint
  const rect = imageCropRect(box, paint, natural)
  if (paint.scale === 'crop' && paint.imageX === rect.x && paint.imageY === rect.y && paint.imageW === rect.w && paint.imageH === rect.h) return paint
  return { ...paint, scale: 'crop', imageX: rect.x, imageY: rect.y, imageW: rect.w, imageH: rect.h }
}

export function panImageCrop(paint: DesignPaint, dx: number, dy: number): DesignPaint {
  if (!isImageCropPaint(paint)) return paint
  return { ...paint, imageX: (paint.imageX ?? 0) + dx, imageY: (paint.imageY ?? 0) + dy }
}

export function keepImageCropWorldFixed(prev: DesignNode, next: DesignNode): DesignNode {
  const paints = nodePaints(next, 'fill')
  if (!paints.some(isImageCropPaint)) return next
  const dx = next.x - prev.x
  const dy = next.y - prev.y
  if (!dx && !dy) return next
  return setNodePaints(next, 'fill', paints.map((paint) => {
    if (!isImageCropPaint(paint)) return paint
    return { ...paint, imageX: (paint.imageX ?? 0) - dx, imageY: (paint.imageY ?? 0) - dy }
  }))
}

export function imageFillPlacement(
  box: { w: number; h: number },
  paint: DesignPaint,
  natural?: { w: number; h: number } | null,
): { size: string; position: string } {
  if (paint.type !== 'image') return { size: '100% 100%', position: 'center' }
  if (paint.scale === 'tile') return { size: 'auto', position: '0 0' }
  if (paint.scale === 'fit') return { size: 'contain', position: 'center' }
  if (paint.scale === 'crop') {
    const rect = imageCropRect(box, paint, natural)
    return { size: `${rect.w}px ${rect.h}px`, position: `${rect.x}px ${rect.y}px` }
  }
  return { size: 'cover', position: 'center' }
}

export function reorderNodePaint(node: DesignNode, field: 'fill' | 'stroke', from: number, to: number): DesignNode {
  const paints = nodePaints(node, field)
  if (from < 0 || to < 0 || from >= paints.length || to >= paints.length || from === to) return node
  const next = paints.slice()
  const [moved] = next.splice(from, 1)
  next.splice(to, 0, moved)
  return setNodePaints(node, field, next)
}

export function hexToHsb(hex: string): { h: number; s: number; b: number } {
  const body = hex.startsWith('#') ? hex.slice(1) : hex
  const n = parseInt(body.length === 3 ? body.split('').map((item) => item + item).join('') : body.slice(0, 6), 16)
  const r = ((n >> 16) & 255) / 255
  const g = ((n >> 8) & 255) / 255
  const bl = (n & 255) / 255
  const max = Math.max(r, g, bl)
  const min = Math.min(r, g, bl)
  const d = max - min
  let h = 0
  if (d) {
    if (max === r) h = ((g - bl) / d) % 6
    else if (max === g) h = (bl - r) / d + 2
    else h = (r - g) / d + 4
    h *= 60
    if (h < 0) h += 360
  }
  return { h, s: max ? (d / max) * 100 : 0, b: max * 100 }
}

export function hsbToHex(h: number, s: number, b: number): string {
  const sat = Math.max(0, Math.min(100, s)) / 100
  const val = Math.max(0, Math.min(100, b)) / 100
  const hue = ((h % 360) + 360) % 360
  const c = val * sat
  const x = c * (1 - Math.abs(((hue / 60) % 2) - 1))
  const m = val - c
  let r = 0
  let g = 0
  let bl = 0
  if (hue < 60) { r = c; g = x }
  else if (hue < 120) { r = x; g = c }
  else if (hue < 180) { g = c; bl = x }
  else if (hue < 240) { g = x; bl = c }
  else if (hue < 300) { r = x; bl = c }
  else { r = c; bl = x }
  const hex = (n: number) => Math.round((n + m) * 255).toString(16).padStart(2, '0')
  return `#${hex(r)}${hex(g)}${hex(bl)}`
}
