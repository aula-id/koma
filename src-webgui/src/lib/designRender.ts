// Raster of a design node for chat. The model sees this picture next to the coordinate list.
import {
  nodeChrome,
  resolveInstanceTree,
  resolveRef,
  textStyle,
  vectorSvgPath,
  type DesignDoc,
  type DesignNode,
} from './design'

function paintFill(doc: DesignDoc, node: DesignNode): string {
  const raw = nodeChrome(node).fill
  if (raw === 'none') return ''
  if (!raw) {
    if (node.kind === 'frame') return '#ffffff'
    if (node.kind === 'rect' || node.kind === 'ellipse') return '#d9d9d9'
    if (node.kind === 'vector' && node.vector?.regions.length) return '#d9d9d9'
    return ''
  }
  const resolved = resolveRef(doc, raw)
  return resolved.startsWith('#') ? resolved : ''
}

function paintStroke(doc: DesignDoc, node: DesignNode): string {
  const raw = nodeChrome(node).stroke
  if (!raw || raw === 'none') return node.kind === 'line' || node.kind === 'vector' ? '#1c1c1c' : ''
  const resolved = resolveRef(doc, raw)
  return resolved.startsWith('#') ? resolved : ''
}

function radiusOf(doc: DesignDoc, node: DesignNode): number {
  if (node.kind === 'ellipse') return Math.min(node.w, node.h) / 2
  const raw = nodeChrome(node).radius
  const value = typeof raw === 'number' ? raw : Number(resolveRef(doc, raw))
  if (!Number.isFinite(value) || value <= 0) return 0
  return Math.min(value, node.w / 2, node.h / 2)
}

function traceRound(ctx: CanvasRenderingContext2D, w: number, h: number, radius: number) {
  const r = Math.max(0, Math.min(radius, w / 2, h / 2))
  ctx.beginPath()
  ctx.moveTo(r, 0)
  ctx.arcTo(w, 0, w, h, r)
  ctx.arcTo(w, h, 0, h, r)
  ctx.arcTo(0, h, 0, 0, r)
  ctx.arcTo(0, 0, w, 0, r)
  ctx.closePath()
}

function drawNode(ctx: CanvasRenderingContext2D, doc: DesignDoc, node: DesignNode) {
  if (node.visible === false) return
  if (node.kind === 'instance') {
    const visual = resolveInstanceTree(doc, node)
    if (!visual) return
    drawNode(ctx, doc, { ...visual, x: node.x, y: node.y, rotation: node.rotation ?? visual.rotation, flipX: node.flipX ?? visual.flipX, flipY: node.flipY ?? visual.flipY })
    return
  }
  ctx.save()
  ctx.translate(node.x, node.y)
  ctx.translate(node.w / 2, node.h / 2)
  ctx.rotate(((node.rotation ?? 0) * Math.PI) / 180)
  ctx.scale(node.flipX ? -1 : 1, node.flipY ? -1 : 1)
  ctx.translate(-node.w / 2, -node.h / 2)
  ctx.globalAlpha *= nodeChrome(node).opacity
  const fill = paintFill(doc, node)
  const stroke = paintStroke(doc, node)
  const radius = radiusOf(doc, node)
  if (node.kind !== 'line' && node.kind !== 'vector' && node.kind !== 'text' && (fill || stroke)) {
    traceRound(ctx, node.w, node.h, radius)
    if (fill) {
      ctx.fillStyle = fill
      ctx.fill()
    }
    if (stroke) {
      ctx.strokeStyle = stroke
      ctx.lineWidth = nodeChrome(node).strokeWidth
      ctx.stroke()
    }
  }
  if (node.kind === 'line') {
    ctx.beginPath()
    ctx.moveTo(0, node.h / 2)
    ctx.lineTo(node.w, node.h / 2)
    ctx.strokeStyle = stroke || '#1c1c1c'
    ctx.lineWidth = nodeChrome(node).strokeWidth
    ctx.stroke()
  }
  if (node.kind === 'vector' && node.vector) {
    const path = new Path2D(vectorSvgPath(node.vector))
    if (fill) {
      ctx.fillStyle = fill
      ctx.fill(path)
    }
    if (stroke) {
      ctx.strokeStyle = stroke
      ctx.lineWidth = nodeChrome(node).strokeWidth
      ctx.stroke(path)
    }
  }
  if (node.kind === 'text') {
    const style = textStyle(node)
    const weight = style.weight === 'bold' ? 700 : style.weight === 'medium' ? 500 : 400
    ctx.fillStyle = paintOfColor(doc, style.color)
    ctx.font = `${weight} ${style.fontSize}px sans-serif`
    ctx.textBaseline = 'middle'
    ctx.textAlign = style.align === 'center' ? 'center' : style.align === 'right' ? 'right' : 'left'
    const x = style.align === 'center' ? node.w / 2 : style.align === 'right' ? node.w - 4 : 4
    ctx.fillText(style.text || 'Text', x, node.h / 2)
  }
  if (node.kind === 'frame') {
    traceRound(ctx, node.w, node.h, radius)
    ctx.clip()
  }
  for (const child of node.children ?? []) drawNode(ctx, doc, child)
  ctx.restore()
}

function paintOfColor(doc: DesignDoc, color: string): string {
  if (!color || color === 'none') return '#1c1c1c'
  const resolved = resolveRef(doc, color)
  return resolved.startsWith('#') ? resolved : '#1c1c1c'
}

/** PNG of the node, or null where there is no canvas. Longest side stays within 1280px. */
export function designPngBase64(doc: DesignDoc, node: DesignNode): string | null {
  if (typeof document === 'undefined') return null
  const canvas = document.createElement('canvas')
  const longest = Math.max(node.w, node.h, 1)
  const scale = Math.min(2, 1280 / longest)
  canvas.width = Math.max(1, Math.round(node.w * scale))
  canvas.height = Math.max(1, Math.round(node.h * scale))
  const ctx = canvas.getContext('2d')
  if (!ctx) return null
  ctx.setTransform(scale, 0, 0, scale, 0, 0)
  drawNode(ctx, doc, { ...node, x: 0, y: 0 })
  const url = canvas.toDataURL('image/png')
  const comma = url.indexOf(',')
  return comma >= 0 ? url.slice(comma + 1) : null
}
