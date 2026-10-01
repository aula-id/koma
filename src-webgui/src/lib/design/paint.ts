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

function gradientCss(doc: DesignDoc, paint: DesignPaint): string {
  const stops = (paint.stops?.length ? paint.stops : [{ color: paint.color ?? '#000000', at: 0 }, { color: paint.color ?? '#ffffff', at: 1 }])
    .slice()
    .sort((a, b) => a.at - b.at)
    .map((stop) => stopCss(doc, stop))
    .join(', ')
  const kind = paint.kind ?? 'linear'
  if (kind === 'radial' || kind === 'diamond') return `radial-gradient(circle at 50% 50%, ${stops})`
  if (kind === 'angular') return `conic-gradient(from 0deg at 50% 50%, ${stops})`
  return `linear-gradient(180deg, ${stops})`
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
  if (paint.type === 'gradient') return gradientCss(doc, paint)
  if (paint.type === 'image') {
    const url = imageAssetUrl(doc, paint.hash)
    if (!url) return fallback
    const scale = paint.scale ?? 'fill'
    if (scale === 'tile') return `url("${url}")`
    return `url("${url}")`
  }
  return resolveColor(doc, paint.color, fallback) || fallback
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

export function nodeFillCss(doc: DesignDoc, node: DesignNode, fallback: string): string {
  const paints = nodePaints(node, 'fill')
  const first = firstVisiblePaint(paints)
  if (!first) return fallback
  if (paintAlias(first) === 'none') return 'transparent'
  return resolvePaintCss(doc, first, fallback)
}

export function nodeStrokeCss(doc: DesignDoc, node: DesignNode, fallback: string): string {
  const paints = nodePaints(node, 'stroke')
  const first = firstVisiblePaint(paints)
  if (!first) return fallback
  if (paintAlias(first) === 'none') return 'transparent'
  return resolvePaintCss(doc, first, fallback)
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
