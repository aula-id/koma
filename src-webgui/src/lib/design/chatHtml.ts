// HTML snapshot of one design node for chat. The chat chip stays a name.
// The model receives this fragment. Image fills point at asset files.
import { designImageSize } from './assets'
import {
  cornerPixels,
  designClips,
  designLayerName,
  fontFamilyCss,
  layoutDesign,
  nodeChrome,
  resolveInstanceTree,
  textStyle,
} from './model'
import { firstVisiblePaint, nodePaints, resolveColor, resolvePaintCss } from './paint'
import type { DesignDoc, DesignNode, DesignPaint } from './types'

type Point = { x: number; y: number }
type Box = { x: number; y: number; w: number; h: number }

type Flat = {
  node: DesignNode
  box: Box
  origin: Point
  children: Flat[]
}

function rotate(x: number, y: number, degrees: number): Point {
  if (!degrees) return { x, y }
  const radians = (degrees * Math.PI) / 180
  const cos = Math.cos(radians)
  const sin = Math.sin(radians)
  return { x: x * cos - y * sin, y: x * sin + y * cos }
}

/** Content point of a node, in its parent's space. Matches the canvas box math. */
function spinToParent(node: DesignNode, localX: number, localY: number): Point {
  const cx = node.w / 2
  const cy = node.h / 2
  const scaledX = (localX - cx) * (node.flipX ? -1 : 1)
  const scaledY = (localY - cy) * (node.flipY ? -1 : 1)
  const turned = rotate(scaledX, scaledY, node.rotation ?? 0)
  return { x: node.x + cx + turned.x, y: node.y + cy + turned.y }
}

function axisBox(node: DesignNode, toCanvas: (x: number, y: number) => Point): Box {
  const corners = [
    spinToParent(node, 0, 0),
    spinToParent(node, node.w, 0),
    spinToParent(node, 0, node.h),
    spinToParent(node, node.w, node.h),
  ].map((point) => toCanvas(point.x, point.y))
  const minX = Math.min(...corners.map((point) => point.x))
  const minY = Math.min(...corners.map((point) => point.y))
  const maxX = Math.max(...corners.map((point) => point.x))
  const maxY = Math.max(...corners.map((point) => point.y))
  return { x: minX, y: minY, w: maxX - minX, h: maxY - minY }
}

function overlaps(a: Box, b: Box): boolean {
  return a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y
}

function px(value: number): string {
  return `${Math.round(value)}px`
}

export function escHtml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

/** Absolute file URL for an asset path under the workspace root. */
export function designFileUrl(root: string, path: string): string {
  const rel = path.replace(/^\.\//, '')
  const abs = rel.startsWith('/') ? rel : `${root.replace(/\/+$/, '')}/${rel.replace(/^\/+/, '')}`
  const encoded = abs.split('/').map((part) => encodeURIComponent(part)).join('/')
  return `file://${encoded}`
}

function expandInstances(doc: DesignDoc, node: DesignNode, stack: string[]): DesignNode {
  if (node.kind === 'instance') {
    if (!node.component || stack.includes(node.component)) return { ...node, children: undefined }
    const visual = resolveInstanceTree(doc, node)
    if (!visual) return node
    return expandInstances(doc, {
      ...visual,
      x: node.x,
      y: node.y,
      rotation: node.rotation ?? visual.rotation,
      flipX: node.flipX ?? visual.flipX,
      flipY: node.flipY ?? visual.flipY,
      clip: node.clip ?? visual.clip,
      name: node.name || visual.name,
    }, [...stack, node.component])
  }
  if (!node.children?.length) return node
  return { ...node, children: node.children.map((child) => expandInstances(doc, child, stack)) }
}

function laidRoot(doc: DesignDoc, node: DesignNode): DesignNode {
  const laid = layoutDesign({ ...doc, screens: [{ ...node, x: 0, y: 0 }] }).screens[0] ?? { ...node, x: 0, y: 0 }
  return expandInstances(doc, laid, [])
}

function flatten(node: DesignNode, toCanvas: (x: number, y: number) => Point, into: Flat[]) {
  if (node.visible === false) return
  into.push({ node, box: axisBox(node, toCanvas), origin: toCanvas(node.x, node.y), children: [] })
  const nested = (x: number, y: number) => {
    const spun = spinToParent(node, x, y)
    return toCanvas(spun.x, spun.y)
  }
  for (const child of node.children ?? []) flatten(child, nested, into)
}

/** Back to front. A node joins the front-most placed box it overlaps, or the root. */
function nestByOverlap(nodes: Flat[]): Flat {
  const root = nodes[0]
  if (!root) return { node: { id: '', kind: 'frame', x: 0, y: 0, w: 0, h: 0 }, box: { x: 0, y: 0, w: 0, h: 0 }, origin: { x: 0, y: 0 }, children: [] }
  const placed: Flat[] = [root]
  for (const item of nodes.slice(1)) {
    let parent = root
    for (let index = placed.length - 1; index >= 0; index--) {
      const earlier = placed[index]
      if (earlier && overlaps(item.box, earlier.box)) {
        parent = earlier
        break
      }
    }
    parent.children.push(item)
    parentOf.set(item, parent)
    placed.push(item)
  }
  return root
}

function weightCss(weight: string): number {
  if (weight === 'bold') return 700
  if (weight === 'medium') return 500
  return 400
}

function radiusCss(doc: DesignDoc, node: DesignNode): string | null {
  if (node.kind === 'ellipse') return '50%'
  const corner = cornerPixels(doc, node)
  if (corner.tl === 0 && corner.tr === 0 && corner.br === 0 && corner.bl === 0) return null
  const tl = Math.round(corner.tl)
  const tr = Math.round(corner.tr)
  const br = Math.round(corner.br)
  const bl = Math.round(corner.bl)
  if (tl === tr && tr === br && br === bl) return `${tl}px`
  return `${tl}px ${tr}px ${br}px ${bl}px`
}

function strokeCss(doc: DesignDoc, node: DesignNode): string {
  const paint = firstVisiblePaint(nodePaints(node, 'stroke'))
  const raw = paint?.type === 'solid' ? paint.color : paint ? null : nodeChrome(node).stroke
  if (raw == null && !paint) return ''
  if (paint?.type === 'gradient') return resolvePaintCss(doc, paint, 'transparent')
  const color = resolveColor(doc, raw ?? paint?.color, '')
  return color && color !== 'none' ? color : ''
}

function imageSrc(doc: DesignDoc, root: string, paint: DesignPaint): string {
  const hash = paint.hash
  if (!hash) return ''
  const path = doc.images?.[hash]?.path
  if (!path) return ''
  return designFileUrl(root, path)
}

function imageStyle(doc: DesignDoc, paint: DesignPaint): string {
  if (paint.scale === 'fit') return 'position:absolute;left:0;top:0;width:100%;height:100%;object-fit:contain'
  if (paint.scale === 'crop' && paint.imageW != null && paint.imageH != null && paint.imageW > 0 && paint.imageH > 0) {
    return `position:absolute;left:${px(paint.imageX ?? 0)};top:${px(paint.imageY ?? 0)};width:${px(paint.imageW)};height:${px(paint.imageH)};object-fit:fill`
  }
  if (paint.scale === 'tile') {
    const size = designImageSize(doc, paint.hash)
    const box = size ? `width:${px(size.w)};height:${px(size.h)}` : 'width:auto;height:auto'
    return `position:absolute;left:0;top:0;${box}`
  }
  return 'position:absolute;left:0;top:0;width:100%;height:100%;object-fit:cover'
}

function styleOf(doc: DesignDoc, item: Flat, rootItem: Flat, fileRoot: string): { css: string; image: string } {
  const node = item.node
  const atRoot = item === rootItem
  const rules = [
    atRoot ? 'position:relative' : 'position:absolute',
    'box-sizing:border-box',
  ]
  if (!atRoot) {
    const origin = parentOf.get(item)?.origin ?? { x: 0, y: 0 }
    rules.push(`left:${px(item.origin.x - origin.x)}`)
    rules.push(`top:${px(item.origin.y - origin.y)}`)
  }
  rules.push(`width:${px(node.w)}`, `height:${px(node.h)}`)
  const fill = firstVisiblePaint(nodePaints(node, 'fill'))
  const image = fill?.type === 'image' ? imageSrc(doc, fileRoot, fill) : ''
  const clipImage = !!image && fill?.scale === 'crop'
  if (fill && fill.type !== 'image') {
    const background = resolvePaintCss(doc, fill, 'transparent')
    if (background && background !== 'transparent') rules.push(`background:${background}`)
  } else if (!fill && (node.kind === 'line' || node.kind === 'vector')) {
    const stroke = strokeCss(doc, node)
    if (stroke) rules.push(`background:${stroke}`)
  }
  if (node.kind !== 'line' && node.kind !== 'text') {
    const stroke = strokeCss(doc, node)
    const width = nodeChrome(node).strokeWidth
    if (stroke && width > 0 && !(node.kind === 'vector' && (!fill || fill.type === 'image'))) {
      rules.push(`border:${Math.max(1, Math.round(width))}px solid ${stroke}`)
    }
  }
  const radius = radiusCss(doc, node)
  if (radius) rules.push(`border-radius:${radius}`)
  const opacity = nodeChrome(node).opacity
  if (opacity < 1) rules.push(`opacity:${Math.round(opacity * 1000) / 1000}`)
  if (designClips(node) || clipImage) rules.push('overflow:hidden')
  const turn = node.rotation ?? 0
  const flipX = node.flipX ? -1 : 1
  const flipY = node.flipY ? -1 : 1
  if (turn || flipX !== 1 || flipY !== 1) {
    const parts = [`rotate(${Math.round(turn * 100) / 100}deg)`]
    if (flipX !== 1 || flipY !== 1) parts.push(`scale(${flipX}, ${flipY})`)
    rules.push(`transform:${parts.join(' ')}`, 'transform-origin:center')
  }
  if (node.kind === 'text') {
    const style = textStyle(node)
    const color = resolveColor(doc, style.color, '#1c1c1c')
    rules.push(`color:${color || '#1c1c1c'}`)
    rules.push(`font-size:${px(style.fontSize)}`)
    rules.push(`font-weight:${weightCss(style.weight)}`)
    const family = fontFamilyCss(node)
    if (family) rules.push(`font-family:"${family}"`)
    rules.push(`text-align:${style.align}`)
    if (style.lineHeight > 0) rules.push(`line-height:${px(style.lineHeight)}`)
    else if (style.vertical === 'center') rules.push(`line-height:${px(node.h)}`)
    if (style.letterSpacing) rules.push(`letter-spacing:${px(style.letterSpacing)}`)
    if (style.text.includes('\n')) rules.push('white-space:pre-wrap')
  }
  return { css: rules.join(';'), image }
}

const parentOf = new WeakMap<Flat, Flat>()

function emit(doc: DesignDoc, item: Flat, rootItem: Flat, fileRoot: string, indent: number): string {
  const pad = '  '.repeat(indent)
  const name = escHtml(designLayerName(item.node))
  const styled = styleOf(doc, item, rootItem, fileRoot)
  const kids = item.children.map((child) => emit(doc, child, rootItem, fileRoot, indent + 1)).join('\n')
  const text = item.node.kind === 'text' ? escHtml(textStyle(item.node).text) : ''
  const img = styled.image
    ? `${'  '.repeat(indent + 1)}<img alt="${name}" src="${escHtml(styled.image)}" style="${imageStyle(doc, firstVisiblePaint(nodePaints(item.node, 'fill'))!)}">`
    : ''
  const open = `<div data-koma-name="${name}" style="${styled.css}">`
  if (!kids && !img) return text ? `${pad}${open}${text}</div>` : `${pad}${open}</div>`
  const body = [img, text ? `${'  '.repeat(indent + 1)}${text}` : '', kids].filter(Boolean).join('\n')
  return `${pad}${open}\n${body}\n${pad}</div>`
}

/** One pretty-printed fragment. `root` is the workspace directory for file URLs. */
export function designChatHtml(doc: DesignDoc, node: DesignNode, root = ''): string {
  const laid = laidRoot(doc, node)
  const flat: Flat[] = []
  flatten(laid, (x, y) => ({ x, y }), flat)
  const tree = nestByOverlap(flat)
  return emit(doc, tree, tree, root, 0)
}
