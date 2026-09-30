// Design documents stored as `<workspace>/.koma/<name>.kdsgn`.
// A file is one design system: tokens, components, and screens.
// Child x/y are parent-relative. A screen's x/y are canvas coordinates.
// Pan and zoom are view state and are not stored here.

export const DESIGN_GRID = 8
export const DESIGN_MIME = 'application/x-koma-design'
export const COMPONENT_MIME = 'application/x-koma-component'
export const DESIGN_MIN_W = 8
export const DESIGN_MIN_H = 8

export type DesignKind = 'frame' | 'group' | 'rect' | 'ellipse' | 'line' | 'vector' | 'text' | 'instance'
export type DesignLayout = 'row' | 'column'
export type DesignAlign = 'start' | 'center' | 'end' | 'space' | 'stretch'
export type DesignSize = 'hug' | 'fill' | 'fixed'
export type DesignWeight = 'regular' | 'medium' | 'bold'
export type DesignTextAlign = 'left' | 'center' | 'right'
export type DesignTextVertical = 'top' | 'center' | 'bottom'
/** `height` hugs the block height. `width` hugs both axes. */
export type DesignTextHug = 'height' | 'width'
export type DesignTokenKind = 'color' | 'space' | 'type' | 'radius'
export type DesignOrder = 'front' | 'forward' | 'backward' | 'back'
export type DesignDrawKind = 'frame' | 'group' | 'rect' | 'ellipse' | 'line' | 'vector' | 'text'

export type DesignVectorPoint = { x: number; y: number }

export type DesignVectorSegment = {
  start: number
  end: number
  tangentStart: DesignVectorPoint
  tangentEnd: DesignVectorPoint
}

export type DesignVectorRegion = {
  winding: 'nonzero' | 'evenodd'
  loops: number[][]
}

/** Vertices are local to the node box. Tangents are offsets from their vertex. */
export type DesignVector = {
  vertices: DesignVectorPoint[]
  segments: DesignVectorSegment[]
  regions: DesignVectorRegion[]
}

/** A pen point in the parent's coordinate space. Handles are relative to the point. */
export type DesignPenPoint = {
  x: number
  y: number
  incoming: DesignVectorPoint
  outgoing: DesignVectorPoint
}

const KINDS: readonly DesignKind[] = ['frame', 'group', 'rect', 'ellipse', 'line', 'vector', 'text', 'instance']
const LAYOUTS: readonly DesignLayout[] = ['row', 'column']
const ALIGNS: readonly DesignAlign[] = ['start', 'center', 'end', 'stretch']
const JUSTIFIES: readonly DesignAlign[] = ['start', 'center', 'end', 'space']
const SIZES: readonly DesignSize[] = ['hug', 'fill', 'fixed']
const WEIGHTS: readonly DesignWeight[] = ['regular', 'medium', 'bold']
const TEXT_ALIGNS: readonly DesignTextAlign[] = ['left', 'center', 'right']
const TEXT_VERTICAL: readonly DesignTextVertical[] = ['top', 'center', 'bottom']
const TEXT_HUGS: readonly DesignTextHug[] = ['height', 'width']
const FONT_FAMILY = /^[\w][\w\s,-]{0,80}$/
const TOKEN_KINDS: readonly DesignTokenKind[] = ['color', 'space', 'type', 'radius']
const TOKEN_NAME = /^[a-zA-Z][a-zA-Z0-9._-]*$/

/** A `#rrggbb`, a token name, or `none` when paint is explicitly off. */
export type DesignRef = string

export type DesignNode = {
  id: string
  kind: DesignKind
  name?: string
  x: number
  y: number
  w: number
  h: number
  /** Main-axis size inside an auto-layout parent. Omitted means fixed. */
  wMode?: DesignSize
  hMode?: DesignSize
  /** Kept out of the parent's flow. Omitted means in flow. */
  absolute?: boolean
  /** Omitted means a free frame. Ignored on other kinds. */
  layout?: DesignLayout
  gap?: number
  pad?: number
  /** Cross-axis alignment. `stretch` sizes non-hug, non-fixed children to the inner cross size. Omitted means start. */
  align?: DesignAlign
  /** Main-axis alignment. Omitted means start. */
  justify?: DesignAlign
  /** Extra inset on one side. Omitted sides use `pad`. */
  padTop?: number
  padRight?: number
  padBottom?: number
  padLeft?: number
  /** Flow onto the next line once the main axis is full. Omitted means one line. A hugging main axis stays one line. */
  wrap?: boolean
  minW?: number
  maxW?: number
  minH?: number
  maxH?: number
  /** Frames and instances clip. `false` shows overflow. Omitted means clip. */
  clip?: boolean
  fill?: DesignRef
  stroke?: DesignRef
  strokeWidth?: number
  radius?: number | string
  /** One corner. Omitted uses `radius`. `0` is a square corner. */
  radiusTL?: number | string
  radiusTR?: number | string
  radiusBR?: number | string
  radiusBL?: number | string
  opacity?: number
  text?: string
  fontSize?: number
  /** CSS family. Omitted uses the UI font. */
  fontFamily?: string
  weight?: DesignWeight
  textAlign?: DesignTextAlign
  /** Omitted means centered in the text box, matching the canvas. */
  textVertical?: DesignTextVertical
  textHug?: DesignTextHug
  /** Pixels. Omitted means the font’s own line height. */
  lineHeight?: number
  /** Pixels. Omitted means 0. Negative values tighten. */
  letterSpacing?: number
  color?: DesignRef
  /** Degrees clockwise around the center. Omitted means 0. */
  rotation?: number
  flipX?: boolean
  flipY?: boolean
  /** Omitted means visible. */
  visible?: boolean
  /** Omitted means unlocked. A locked node is the canvas hit and does not walk its children. */
  locked?: boolean
  /** Local vector network. Required when kind is vector. */
  vector?: DesignVector
  /** Instance target. Required when kind is instance. */
  component?: string
  variant?: Record<string, string>
  /** Per-child instance overrides, keyed by the component node's id. */
  overrides?: DesignOverride[]
  children?: DesignNode[]
}

export type DesignOverride = {
  id: string
  text?: string
  fill?: DesignRef
  visible?: boolean
}

export type DesignVariant = {
  props: Record<string, string>
  node: DesignNode
}

export type DesignComponent = {
  id: string
  name: string
  axes?: Record<string, string[]>
  variants: DesignVariant[]
}

export type DesignToken = {
  name: string
  kind: DesignTokenKind
  values: Record<string, string>
}

export type DesignDoc = {
  version: 1
  modes: string[]
  mode: string
  snap: boolean
  grid: number
  tokens: DesignToken[]
  components: DesignComponent[]
  screens: DesignNode[]
}

export function emptyDesign(): DesignDoc {
  return {
    version: 1,
    modes: ['light', 'dark'],
    mode: 'light',
    snap: false,
    grid: DESIGN_GRID,
    tokens: [],
    components: [],
    screens: [],
  }
}

export function designTabId(root: string, path: string): string {
  return `design:${root}:${path}`
}

export function isDesignPath(path: string): boolean {
  return path.startsWith('.koma/') && path.toLowerCase().endsWith('.kdsgn') && !path.slice('.koma/'.length).includes('/')
}

/** Accept a typed file name and force a single `.kdsgn` suffix. */
export function designFileName(raw: string): string | null {
  const cleaned = raw.trim().replace(/^\/+|\/+$/g, '')
  if (!cleaned || cleaned.includes('..') || cleaned.includes('/') || cleaned.includes('\\')) return null
  const stem = cleaned.replace(/\.kdsgn$/i, '')
  if (!stem) return null
  return `${stem}.kdsgn`
}

export function createNode(kind: DesignDrawKind, id: string, x: number, y: number): DesignNode {
  if (kind === 'frame') return { id, kind, name: 'Frame', x, y, w: 360, h: 240, fill: '#ffffff' }
  if (kind === 'group') return { id, kind, name: 'Group', x, y, w: 8, h: 8, fill: 'none', stroke: 'none' }
  if (kind === 'text') return { id, kind, name: 'Text', x, y, w: 200, h: 24, text: 'Text', fontSize: 14 }
  if (kind === 'ellipse') return { id, kind, name: 'Ellipse', x, y, w: 100, h: 100, fill: '#d9d9d9', stroke: 'none' }
  if (kind === 'line') return { id, kind, name: 'Line', x, y, w: 160, h: 2, stroke: '#1c1c1c', strokeWidth: 2 }
  if (kind === 'vector') {
    return { id, kind, name: 'Vector', x, y, w: 1, h: 1, fill: 'none', stroke: '#1c1c1c', strokeWidth: 2, vector: { vertices: [{ x: 0, y: 0 }], segments: [], regions: [] } }
  }
  // Click-create matches Figma: a sharp 100×100 gray square with no stroke.
  return { id, kind, name: 'Rectangle', x, y, w: 100, h: 100, fill: '#d9d9d9', stroke: 'none' }
}

export function isDesignContainer(kind: DesignKind): boolean {
  return kind === 'frame' || kind === 'group'
}

export function designLayerName(node: DesignNode): string {
  if (node.name?.trim()) return node.name
  if (node.kind === 'text') return node.text?.trim() || 'Text'
  if (node.kind === 'rect') return 'Rectangle'
  if (node.kind === 'ellipse') return 'Ellipse'
  if (node.kind === 'line') return 'Line'
  if (node.kind === 'vector') return 'Vector'
  if (node.kind === 'group') return 'Group'
  if (node.kind === 'instance') return 'Instance'
  return 'Frame'
}

export function variantKey(props: Record<string, string> | undefined): string {
  if (!props) return ''
  return Object.keys(props)
    .sort()
    .map((key) => `${key}=${props[key]}`)
    .join('&')
}

export function pickVariant(component: DesignComponent, props?: Record<string, string>): DesignVariant | null {
  if (!component.variants.length) return null
  const want = props ?? {}
  const key = variantKey(want)
  const exact = component.variants.find((variant) => variantKey(variant.props) === key)
  if (exact) return exact
  let best = component.variants[0]
  let score = -1
  for (const variant of component.variants) {
    let next = 0
    for (const [name, value] of Object.entries(want)) {
      if (variant.props[name] === value) next += 1
    }
    if (next > score) {
      score = next
      best = variant
    }
  }
  return best
}

/** Paint the root fill, the first text node, and any per-child override. */
export function applyOverrides(root: DesignNode, override: { text?: string; fill?: string; overrides?: DesignOverride[] }): DesignNode {
  const byId = new Map((override.overrides ?? []).map((item) => [item.id, item]))
  let textUsed = override.text == null
  const walk = (node: DesignNode, isRoot: boolean): DesignNode => {
    let next = node
    const row = byId.get(node.id)
    if (isRoot && override.fill) next = { ...next, fill: override.fill }
    if (row?.fill) next = { ...next, fill: row.fill }
    if (row?.visible === false) next = { ...next, visible: false }
    else if (row?.visible === true) {
      const shown = { ...next }
      delete shown.visible
      next = shown
    }
    if (row?.text != null && node.kind === 'text') {
      textUsed = true
      next = { ...next, text: row.text }
    } else if (!textUsed && node.kind === 'text') {
      textUsed = true
      next = { ...next, text: override.text }
    }
    if (next.children?.length) next = { ...next, children: next.children.map((child) => walk(child, false)) }
    return next
  }
  return walk(root, true)
}

/** Write one child override on an instance. A null field clears that key. An empty row drops the override. */
export function mergeDesignOverride(
  node: DesignNode,
  targetId: string,
  patch: { text?: string | null; fill?: string | null; visible?: boolean | null },
): DesignNode {
  if (node.kind !== 'instance' || !targetId) return node
  const current = (node.overrides ?? []).map((item) => ({ ...item }))
  const index = current.findIndex((item) => item.id === targetId)
  const row: DesignOverride = index >= 0 ? { ...current[index] } : { id: targetId }
  if (patch.text === null) delete row.text
  else if (patch.text != null) row.text = patch.text
  if (patch.fill === null) delete row.fill
  else if (patch.fill != null) row.fill = patch.fill
  if (patch.visible === null) delete row.visible
  else if (patch.visible != null) row.visible = patch.visible
  const empty = row.text == null && (row.fill == null || row.fill === '') && row.visible == null
  if (empty && index >= 0) current.splice(index, 1)
  else if (empty) return node
  else if (index >= 0) current[index] = row
  else current.push(row)
  const next: DesignNode = { ...node }
  if (current.length) next.overrides = current
  else delete next.overrides
  return next
}

export function findDesignOverride(instance: DesignNode, childId: string): DesignOverride | undefined {
  if (instance.kind !== 'instance') return undefined
  return instance.overrides?.find((item) => item.id === childId)
}

function findInComponentTree(node: DesignNode, id: string): DesignNode | null {
  if (node.id === id) return node
  for (const child of node.children ?? []) {
    const found = findInComponentTree(child, id)
    if (found) return found
  }
  return null
}

/** Variant defaults merged with instance override rows — for the overrides panel. */
export function effectiveInstanceChild(
  doc: DesignDoc,
  instance: DesignNode,
  childId: string,
): {
  text: string
  fill: DesignRef | undefined
  visible: boolean
  hasTextOverride: boolean
  hasFillOverride: boolean
  hasVisibleOverride: boolean
} | null {
  if (instance.kind !== 'instance' || !instance.component) return null
  const component = doc.components.find((item) => item.id === instance.component)
  if (!component) return null
  const variant = pickVariant(component, instance.variant)
  if (!variant) return null
  const base = findInComponentTree(variant.node, childId)
  if (!base) return null
  const row = findDesignOverride(instance, childId)
  let visible = base.visible !== false
  if (row?.visible === false) visible = false
  else if (row?.visible === true) visible = true
  return {
    text: row?.text ?? base.text ?? '',
    fill: row?.fill ?? base.fill,
    visible,
    hasTextOverride: row?.text != null,
    hasFillOverride: row?.fill != null && row.fill !== '',
    hasVisibleOverride: row?.visible != null,
  }
}

export function resolveRef(doc: DesignDoc, ref: string): string {
  if (!ref || ref === 'none' || ref.startsWith('#')) return ref
  const token = doc.tokens.find((item) => item.name === ref)
  if (!token) return ''
  return token.values[doc.mode] ?? token.values[doc.modes[0] ?? ''] ?? Object.values(token.values)[0] ?? ''
}

function defaultTokenValue(kind: DesignTokenKind, mode: string): string {
  if (kind === 'color') return mode === 'dark' ? '#c8d3f5' : '#1a1d27'
  if (kind === 'type') return '13/regular'
  return '8'
}

/** Switch the saved mode. Unknown modes leave the document alone. */
export function setDesignMode(doc: DesignDoc, mode: string): DesignDoc {
  if (!doc.modes.includes(mode) || doc.mode === mode) return doc
  return { ...doc, mode }
}

/** Append a token with a value for every mode. A bad or duplicate name returns null. */
export function addDesignToken(doc: DesignDoc, rawName: string, kind: DesignTokenKind): DesignDoc | null {
  const name = rawName.trim()
  if (!TOKEN_NAME.test(name) || doc.tokens.some((token) => token.name === name)) return null
  const values: Record<string, string> = {}
  for (const mode of doc.modes) values[mode] = defaultTokenValue(kind, mode)
  return { ...doc, tokens: [...doc.tokens, { name, kind, values }] }
}

/** Write one mode's value. A bad value returns null. */
export function setDesignTokenValue(doc: DesignDoc, name: string, mode: string, raw: string): DesignDoc | null {
  if (!doc.modes.includes(mode)) return null
  const index = doc.tokens.findIndex((token) => token.name === name)
  if (index < 0) return null
  const token = doc.tokens[index]
  const parsed = parseTokenValue(token.kind, raw.trim())
  if (!parsed) return null
  if (token.values[mode] === parsed) return doc
  const tokens = doc.tokens.slice()
  tokens[index] = { ...token, values: { ...token.values, [mode]: parsed } }
  return { ...doc, tokens }
}

export function dropDesignToken(doc: DesignDoc, name: string): DesignDoc {
  if (!doc.tokens.some((token) => token.name === name)) return doc
  return { ...doc, tokens: doc.tokens.filter((token) => token.name !== name) }
}

/** Resolved chrome. An empty fill or stroke means the theme color. `none` is off. */
export function nodeChrome(node: DesignNode): { fill: string; stroke: string; radius: number | string; opacity: number; strokeWidth: number } {
  const shaped = node.kind === 'frame' || node.kind === 'rect' || node.kind === 'ellipse' || (node.kind === 'vector' && !!node.vector?.regions.length)
  const stroked = shaped || node.kind === 'line' || node.kind === 'vector'
  return {
    fill: node.fill ?? (shaped ? '' : 'none'),
    stroke: node.stroke ?? (stroked ? '' : 'none'),
    radius: node.radius ?? 0,
    opacity: node.opacity ?? 1,
    strokeWidth: node.strokeWidth ?? (node.kind === 'line' ? 2 : 1),
  }
}

export function textStyle(node: DesignNode): { text: string; fontSize: number; weight: DesignWeight; align: DesignTextAlign; color: string; lineHeight: number; letterSpacing: number; fontFamily: string; vertical: DesignTextVertical; hug: DesignTextHug | 'fixed' } {
  return {
    text: node.text ?? '',
    fontSize: node.fontSize && node.fontSize > 0 ? node.fontSize : 13,
    weight: node.weight ?? 'regular',
    align: node.textAlign ?? 'left',
    color: node.color ?? '',
    lineHeight: node.lineHeight && node.lineHeight > 0 ? node.lineHeight : 0,
    letterSpacing: node.letterSpacing ?? 0,
    fontFamily: fontFamilyCss(node),
    vertical: node.textVertical ?? 'center',
    hug: node.textHug ?? 'fixed',
  }
}

/** A safe CSS family, or empty when the node uses the UI font. */
export function fontFamilyCss(node: DesignNode): string {
  const raw = node.fontFamily?.trim()
  if (!raw || !FONT_FAMILY.test(raw)) return ''
  return raw
}

/**
 * Text box from a half-em estimate. Layout stays the same in the browser and in tests.
 * `width` hug uses this box. `height` hug wraps that width into the node's width.
 */
export function measureTextBox(node: DesignNode): { w: number; h: number } {
  const style = textStyle(node)
  const lineH = style.lineHeight > 0 ? style.lineHeight : Math.round(style.fontSize * 1.2)
  const lines = style.text.split('\n')
  const charW = style.fontSize * 0.5
  let width = style.fontSize
  for (const line of lines) {
    const next = Math.ceil(line.length * charW + style.letterSpacing * Math.max(0, line.length - 1))
    if (next > width) width = next
  }
  return { w: Math.max(1, width), h: Math.max(lineH, Math.max(1, lines.length) * lineH) }
}

/** Resolved corner radii in pixels. An omitted corner uses `radius`. */
export function cornerPixels(doc: DesignDoc, node: DesignNode): { tl: number; tr: number; br: number; bl: number } {
  const base = node.radius
  const px = (value: number | string | undefined): number => {
    const raw = value !== undefined ? value : base
    if (raw == null) return 0
    const n = typeof raw === 'number' ? raw : Number(resolveRef(doc, raw))
    return Number.isFinite(n) && n > 0 ? n : 0
  }
  return { tl: px(node.radiusTL), tr: px(node.radiusTR), br: px(node.radiusBR), bl: px(node.radiusBL) }
}

/** Frames and instances clip unless `clip` is false. */
export function designClips(node: DesignNode): boolean {
  return (node.kind === 'frame' || node.kind === 'instance') && node.clip !== false
}

/** The one value every entry shares. An empty list, or any difference, is null. */
export function sharedValue<T>(values: readonly T[]): T | null {
  if (!values.length) return null
  const first = values[0]
  for (let index = 1; index < values.length; index++) if (!Object.is(values[index], first)) return null
  return first
}

/** Deep copy. The root moves by dx/dy. Children stay parent-relative. */
export function copyTree(node: DesignNode, mint: () => string, dx = 0, dy = 0): DesignNode {
  const next = cloneNode(node, mint)
  next.x += dx
  next.y += dy
  return next
}

function cloneNode(node: DesignNode, mint: () => string): DesignNode {
  const next: DesignNode = { id: mint(), kind: node.kind, x: node.x, y: node.y, w: node.w, h: node.h }
  if (node.name) next.name = node.name
  if (node.wMode) next.wMode = node.wMode
  if (node.hMode) next.hMode = node.hMode
  if (node.absolute) next.absolute = true
  if (node.layout) next.layout = node.layout
  if (node.gap) next.gap = node.gap
  if (node.pad) next.pad = node.pad
  if (node.padTop != null) next.padTop = node.padTop
  if (node.padRight != null) next.padRight = node.padRight
  if (node.padBottom != null) next.padBottom = node.padBottom
  if (node.padLeft != null) next.padLeft = node.padLeft
  if (node.wrap) next.wrap = true
  if (node.minW != null) next.minW = node.minW
  if (node.maxW != null) next.maxW = node.maxW
  if (node.minH != null) next.minH = node.minH
  if (node.maxH != null) next.maxH = node.maxH
  if (node.clip === false) next.clip = false
  if (node.align) next.align = node.align
  if (node.justify) next.justify = node.justify
  if (node.fill) next.fill = node.fill
  if (node.stroke) next.stroke = node.stroke
  if (node.strokeWidth != null) next.strokeWidth = node.strokeWidth
  if (node.radius != null) next.radius = node.radius
  if (node.radiusTL != null) next.radiusTL = node.radiusTL
  if (node.radiusTR != null) next.radiusTR = node.radiusTR
  if (node.radiusBR != null) next.radiusBR = node.radiusBR
  if (node.radiusBL != null) next.radiusBL = node.radiusBL
  if (node.opacity != null) next.opacity = node.opacity
  if (node.text != null) next.text = node.text
  if (node.fontSize != null) next.fontSize = node.fontSize
  if (node.fontFamily) next.fontFamily = node.fontFamily
  if (node.weight) next.weight = node.weight
  if (node.textAlign) next.textAlign = node.textAlign
  if (node.textVertical) next.textVertical = node.textVertical
  if (node.textHug) next.textHug = node.textHug
  if (node.lineHeight != null) next.lineHeight = node.lineHeight
  if (node.letterSpacing) next.letterSpacing = node.letterSpacing
  if (node.color) next.color = node.color
  if (node.rotation) next.rotation = node.rotation
  if (node.flipX) next.flipX = true
  if (node.flipY) next.flipY = true
  if (node.visible === false) next.visible = false
  if (node.locked) next.locked = true
  if (node.vector) next.vector = cloneVector(node.vector)
  if (node.component) next.component = node.component
  if (node.variant) next.variant = { ...node.variant }
  if (node.overrides?.length) next.overrides = node.overrides.map((item) => ({ ...item }))
  if (node.children?.length) next.children = node.children.map((child) => cloneNode(child, mint))
  return next
}

function cloneVector(vector: DesignVector): DesignVector {
  return {
    vertices: vector.vertices.map((point) => ({ ...point })),
    segments: vector.segments.map((segment) => ({
      ...segment,
      tangentStart: { ...segment.tangentStart },
      tangentEnd: { ...segment.tangentEnd },
    })),
    regions: vector.regions.map((region) => ({ winding: region.winding, loops: region.loops.map((loop) => loop.slice()) })),
  }
}

export type DesignHandle = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w'

/** Grid snap when enabled. Otherwise the nearest document pixel. */
export function snapDesign(n: number, grid: number, enabled: boolean): number {
  if (!Number.isFinite(n)) return n
  if (enabled && Number.isFinite(grid) && grid > 0) return Math.round(n / grid) * grid
  return Math.round(n)
}

export function resizeDesignNode(
  node: DesignNode,
  handle: DesignHandle,
  dx: number,
  dy: number,
  grid: number,
  enabled: boolean,
): DesignNode {
  const right = node.x + node.w
  const bottom = node.y + node.h
  let x = node.x
  let y = node.y
  let w = node.w
  let h = node.h
  if (handle.includes('w')) {
    x = snapDesign(node.x + dx, grid, enabled)
    w = right - x
  } else if (handle.includes('e')) {
    w = snapDesign(right + dx, grid, enabled) - x
  }
  if (handle.includes('n')) {
    y = snapDesign(node.y + dy, grid, enabled)
    h = bottom - y
  } else if (handle.includes('s')) {
    h = snapDesign(bottom + dy, grid, enabled) - y
  }
  const minW = node.kind === 'line' || node.kind === 'vector' ? 1 : DESIGN_MIN_W
  const minH = node.kind === 'line' || node.kind === 'vector' ? 1 : DESIGN_MIN_H
  if (w < minW) {
    if (handle.includes('w')) x = right - minW
    w = minW
  }
  if (h < minH) {
    if (handle.includes('n')) y = bottom - minH
    h = minH
  }
  const next: DesignNode = { ...node, x, y, w, h }
  if (node.kind === 'vector' && node.vector && node.w > 0 && node.h > 0 && (w !== node.w || h !== node.h)) {
    next.vector = scaleVector(node.vector, w / node.w, h / node.h)
  }
  if (node.kind === 'group' && node.children?.length && node.w > 0 && node.h > 0 && (w !== node.w || h !== node.h)) {
    const sx = w / node.w
    const sy = h / node.h
    const ax = handle.includes('w') ? node.w : 0
    const ay = handle.includes('n') ? node.h : 0
    next.children = node.children.map((child) => scaleNodeBox(child, sx, sy, ax, ay, handle.includes('w') ? w : 0, handle.includes('n') ? h : 0))
  }
  return next
}

/** Scale a subtree about an anchor, then shift that anchor onto its new content point. */
function scaleNodeBox(node: DesignNode, sx: number, sy: number, ax: number, ay: number, newAx: number, newAy: number): DesignNode {
  const next: DesignNode = {
    ...node,
    x: ax + (node.x - ax) * sx + (newAx - ax),
    y: ay + (node.y - ay) * sy + (newAy - ay),
    w: Math.max(1, node.w * sx),
    h: Math.max(1, node.h * sy),
  }
  if (node.vector && node.w > 0 && node.h > 0) next.vector = scaleVector(node.vector, next.w / node.w, next.h / node.h)
  if (node.fontSize) next.fontSize = Math.max(1, node.fontSize * sy)
  if (node.children?.length) next.children = node.children.map((child) => scaleNodeBox(child, sx, sy, 0, 0, 0, 0))
  return next
}

export function findDesignNode(doc: DesignDoc, id: string): DesignNode | null {
  for (const screen of doc.screens) {
    const found = findInNode(screen, id)
    if (found) return found
  }
  return null
}

export function locateDesign(doc: DesignDoc, id: string): { node: DesignNode; parentId: string | null } | null {
  for (const screen of doc.screens) {
    if (screen.id === id) return { node: screen, parentId: null }
    const found = locateIn(screen, id)
    if (found) return found
  }
  return null
}

/** Canvas position of a node's top-left, after ancestor rotation and flip. */
export function nodeOrigin(doc: DesignDoc, id: string): { x: number; y: number } | null {
  const path = pathToNode(doc, id)
  if (!path) return null
  let toCanvas = (x: number, y: number) => ({ x, y })
  for (let i = 0; i < path.length - 1; i++) {
    const node = path[i]
    const outer = toCanvas
    toCanvas = (x, y) => {
      const parent = spinToParent(node, x, y)
      return outer(parent.x, parent.y)
    }
  }
  const anchor = spinToParent(path[path.length - 1], 0, 0)
  return toCanvas(anchor.x, anchor.y)
}

/** Canvas position of the border box. Flip and rotation stay on the node, so the box does not jump when it changes parent. */
export function nodeBoxOrigin(doc: DesignDoc, id: string): { x: number; y: number } | null {
  const path = pathToNode(doc, id)
  if (!path) return null
  const node = path[path.length - 1]
  let x = node.x
  let y = node.y
  for (let i = path.length - 2; i >= 0; i--) {
    const spun = spinToParent(path[i], x, y)
    x = spun.x
    y = spun.y
  }
  return { x, y }
}

/** Ancestors, then the node. */
export function designPath(doc: DesignDoc, id: string): DesignNode[] | null {
  return pathToNode(doc, id)
}

export function pointInDesign(doc: DesignDoc, id: string, x: number, y: number): boolean {
  const path = pathToNode(doc, id)
  if (!path) return false
  let px = x
  let py = y
  for (const node of path) {
    const local = parentPointToContent(node, px, py)
    px = local.x
    py = local.y
  }
  return insideNode(path[path.length - 1], { x: px, y: py }, 0)
}

export type DesignDrop = { kind: 'stay' } | { kind: 'move'; parentId: string | null; x: number; y: number }

/** A locked ancestor keeps its descendants. A null next parent is the canvas. */
export function canLeaveParent(doc: DesignDoc, id: string, nextParentId: string | null): boolean {
  const path = pathToNode(doc, id)
  if (!path) return false
  for (let index = path.length - 2; index >= 0; index--) {
    const ancestor = path[index]
    if (!ancestor?.locked) continue
    if (nextParentId == null) return false
    const nextPath = pathToNode(doc, nextParentId)
    if (!nextPath?.some((node) => node.id === ancestor.id)) return false
  }
  return true
}

/**
 * Where a dragged node lands. x/y are the border box in the new parent's space.
 * A locked ancestor stays. The pointer over another container joins that container.
 * A frame child whose box is completely outside moves one level, to that frame's parent.
 * A group child, and a frame child that still overlaps its frame, stay.
 */
export function designDrop(doc: DesignDoc, id: string, pointerX: number, pointerY: number): DesignDrop {
  const located = locateDesign(doc, id)
  if (!located) return { kind: 'stay' }
  if (!canLeaveParent(doc, id, null)) return { kind: 'stay' }
  const target = frameAtPoint(doc, pointerX, pointerY, id)
  const path = target ? pathToNode(doc, target) : null
  const nested = !!(located.parentId && path?.some((node, index) => node.id === located.parentId && index < path.length - 1))
  const inside = located.parentId ? pointInDesign(doc, located.parentId, pointerX, pointerY) : false
  if (target === located.parentId || (inside && !nested)) return { kind: 'stay' }
  const origin = nodeBoxOrigin(doc, id)
  if (!origin) return { kind: 'stay' }
  const nextParent = target && target !== located.parentId ? target : outsideParent(doc, located)
  if (nextParent === undefined || nextParent === located.parentId) return { kind: 'stay' }
  const local = canvasToContent(doc, nextParent, origin.x, origin.y)
  if (!local) return { kind: 'stay' }
  return { kind: 'move', parentId: nextParent, x: local.x, y: local.y }
}

/** The frame's parent when the child's border box is completely outside that frame. Undefined means stay. */
function outsideParent(doc: DesignDoc, located: { node: DesignNode; parentId: string | null }): string | null | undefined {
  if (!located.parentId) return undefined
  const parent = findDesignNode(doc, located.parentId)
  if (!parent || parent.kind !== 'frame') return undefined
  const node = located.node
  const clear = node.x + node.w < 0 || node.x > parent.w || node.y + node.h < 0 || node.y > parent.h
  if (!clear) return undefined
  const parentPath = pathToNode(doc, located.parentId)
  if (!parentPath || parentPath.length < 2) return null
  return parentPath[parentPath.length - 2]?.id ?? null
}

/**
 * Topmost node under a canvas point.
 * A group or instance hit returns that node. Pass deep to step one level into it.
 * A frame hit returns the child under the pointer. A clipping frame ignores points outside itself.
 * A locked node that contains the point returns itself.
 */
export function hitDesign(doc: DesignDoc, x: number, y: number, deep = false): DesignNode | null {
  for (let i = doc.screens.length - 1; i >= 0; i--) {
    const found = hitIn(doc.screens[i], x, y, deep)
    if (found) return found
  }
  return null
}

/**
 * The node a click should select. A selected group or instance, or one that contains the selection,
 * drills one level. Otherwise that hit stays on the group or instance, and deep steps into it once.
 */
export function selectDesignHit(doc: DesignDoc, x: number, y: number, selected: readonly string[], deep: boolean): string | null {
  const entered = enteredGroup(doc, selected, x, y)
  if (entered) return hitGroupChild(doc, entered, x, y)?.id ?? entered
  return hitDesign(doc, x, y, deep)?.id ?? null
}

function enteredGroup(doc: DesignDoc, selected: readonly string[], x: number, y: number): string | null {
  let bestId: string | null = null
  let bestDepth = -1
  for (const id of selected) {
    const path = pathToNode(doc, id)
    if (!path) continue
    for (let index = 0; index < path.length; index++) {
      const node = path[index]
      if (!node || (node.kind !== 'group' && node.kind !== 'instance') || !pointInDesign(doc, node.id, x, y)) continue
      if (index >= bestDepth) {
        bestId = node.id
        bestDepth = index
      }
    }
  }
  return bestId
}

/** One level inside a group or instance. A nested group or instance is returned whole. */
export function hitGroupChild(doc: DesignDoc, groupId: string, x: number, y: number): DesignNode | null {
  const group = findDesignNode(doc, groupId)
  const local = canvasToContent(doc, groupId, x, y)
  if (!group || (group.kind !== 'group' && group.kind !== 'instance') || !local) return null
  const children = group.children ?? []
  for (let i = children.length - 1; i >= 0; i--) {
    const found = hitIn(children[i], local.x, local.y, false)
    if (found) return found
  }
  return null
}

const FLOW_BREAK = 16

/** True when the point is inside the node or within 16 document pixels of its border box. */
export function inFlowBand(doc: DesignDoc, id: string, x: number, y: number): boolean {
  const path = pathToNode(doc, id)
  if (!path?.length) return false
  let px = x
  let py = y
  for (const node of path) {
    const local = parentPointToContent(node, px, py)
    px = local.x
    py = local.y
  }
  const parent = path[path.length - 1]
  if (!parent) return false
  return px >= -FLOW_BREAK && py >= -FLOW_BREAK && px <= parent.w + FLOW_BREAK && py <= parent.h + FLOW_BREAK
}

/** Insert index among non-absolute siblings. `local` is the pointer on the main axis. */
export function flowInsertIndex(parent: DesignNode, id: string, local: number): number {
  let index = 0
  for (const sibling of parent.children ?? []) {
    if (sibling.absolute || sibling.id === id) continue
    const center = parent.layout === 'row' ? sibling.x + sibling.w / 2 : sibling.y + sibling.h / 2
    if (local > center) index += 1
  }
  return index
}

export type DesignFlowBar = { x: number; y: number; w: number; h: number }

/** A 2px bar in the gap where an in-flow drag would insert, in canvas space. */
export function flowBreakBar(doc: DesignDoc, parentId: string, id: string, pointerX: number, pointerY: number): DesignFlowBar | null {
  const parent = findDesignNode(doc, parentId)
  const localPoint = canvasToContent(doc, parentId, pointerX, pointerY)
  const origin = nodeBoxOrigin(doc, parentId)
  if (!parent?.layout || !localPoint || !origin) return null
  const along = parent.layout === 'row' ? localPoint.x : localPoint.y
  const index = flowInsertIndex(parent, id, along)
  const flow = (parent.children ?? []).filter((child) => !child.absolute && child.id !== id)
  const before = index > 0 ? flow[index - 1] : null
  const after = flow[index]
  const pad = parent.pad ?? 0
  if (parent.layout === 'row') {
    const left = before ? before.x + before.w : pad
    const right = after ? after.x : parent.w - pad
    const mid = (left + right) / 2
    return { x: origin.x + mid - 1, y: origin.y + pad, w: 2, h: Math.max(2, parent.h - pad * 2) }
  }
  const top = before ? before.y + before.h : pad
  const bottom = after ? after.y : parent.h - pad
  const mid = (top + bottom) / 2
  return { x: origin.x + pad, y: origin.y + mid - 1, w: Math.max(2, parent.w - pad * 2), h: 2 }
}

/** Deepest frame under a canvas point, skipping a node and its descendants. */
export function frameAtPoint(doc: DesignDoc, x: number, y: number, ignoreId: string): string | null {
  for (let i = doc.screens.length - 1; i >= 0; i--) {
    const found = frameIn(doc.screens[i], x, y, ignoreId)
    if (found) return found
  }
  return null
}

export function updateDesignNode(doc: DesignDoc, id: string, fn: (node: DesignNode) => DesignNode): DesignDoc {
  let changed = false
  const screens = doc.screens.map((screen) => {
    const next = replaceNode(screen, id, fn)
    if (next !== screen) changed = true
    return next
  })
  return changed ? { ...doc, screens } : doc
}

export function deleteDesignNode(doc: DesignDoc, id: string): DesignDoc {
  let changed = false
  const screens: DesignNode[] = []
  for (const screen of doc.screens) {
    const next = withoutNode(screen, id)
    if (next !== screen) changed = true
    if (next) screens.push(next)
  }
  return changed ? { ...doc, screens } : doc
}

export function insertDesignNode(doc: DesignDoc, parentId: string | null, node: DesignNode): DesignDoc {
  return insertDesignNodeAt(doc, parentId, node, Number.MAX_SAFE_INTEGER)
}

export function insertDesignNodeAt(doc: DesignDoc, parentId: string | null, node: DesignNode, index: number): DesignDoc {
  if (parentId == null) {
    const screens = doc.screens.slice()
    screens.splice(Math.max(0, Math.min(index, screens.length)), 0, node)
    return { ...doc, screens }
  }
  let changed = false
  const screens = doc.screens.map((screen) => {
    const next = addChild(screen, parentId, node, index)
    if (next !== screen) changed = true
    return next
  })
  return changed ? { ...doc, screens } : doc
}

/** Move a node under a new parent. A null parent promotes it to a screen. */
export function placeDesignNode(doc: DesignDoc, id: string, parentId: string | null, x: number, y: number): DesignDoc {
  const located = locateDesign(doc, id)
  if (!located) return doc
  if (parentId === located.parentId && located.node.x === x && located.node.y === y) return doc
  if (parentId === id) return doc
  if (parentId && containsNode(located.node, parentId)) return doc
  if (parentId != null && !findDesignNode(doc, parentId)) return doc
  const removed = deleteDesignNode(doc, id)
  return insertDesignNode(removed, parentId, { ...located.node, x, y })
}

/** Place auto-layout children. A frozen id keeps the position a drag is previewing. */
export function layoutDesign(doc: DesignDoc, frozenId?: string): DesignDoc {
  let changed = false
  const screens = doc.screens.map((screen) => {
    const next = layoutNode(screen, frozenId)
    if (next !== screen) changed = true
    return next
  })
  const components = doc.components.map((component) => {
    let variantChanged = false
    const variants = component.variants.map((variant) => {
      const node = layoutNode(variant.node, frozenId)
      if (node === variant.node) return variant
      variantChanged = true
      return { ...variant, node }
    })
    if (!variantChanged) return component
    changed = true
    return { ...component, variants }
  })
  return changed ? { ...doc, screens, components } : doc
}

const BOARD_GAP = 80

function syncAxes(component: DesignComponent): DesignComponent {
  const axes: Record<string, string[]> = {}
  for (const variant of component.variants) {
    for (const [key, value] of Object.entries(variant.props)) {
      const name = key.trim()
      const next = value.trim()
      if (!name || !next) continue
      const list = axes[name] ?? []
      if (!list.includes(next)) list.push(next)
      axes[name] = list
    }
  }
  const synced: DesignComponent = { ...component, variants: component.variants }
  if (Object.keys(axes).length) synced.axes = axes
  else delete synced.axes
  return synced
}

function cleanProps(props: Record<string, string>): Record<string, string> {
  const clean: Record<string, string> = {}
  for (const [key, value] of Object.entries(props)) {
    const name = key.trim()
    const next = value.trim()
    if (name && next) clean[name] = next
  }
  return clean
}

function zeroRoot(node: DesignNode): DesignNode {
  return node.x === 0 && node.y === 0 ? node : { ...node, x: 0, y: 0 }
}

/** A prop set that does not collide with the variants already present. */
export function nextVariantProps(variants: { props: Record<string, string> }[]): Record<string, string> {
  const names = new Set<string>()
  for (const variant of variants) {
    for (const key of Object.keys(variant.props)) names.add(key)
  }
  const name = [...names][0] ?? 'variant'
  const seen = new Set(variants.map((variant) => variantKey(variant.props)))
  let i = variants.length + 1
  let props = { [name]: `v${i}` }
  while (seen.has(variantKey(props))) {
    i += 1
    props = { [name]: `v${i}` }
  }
  return props
}

/** Copy a frame into the component list. A nested frame is replaced by an instance. */
export function createComponentFromFrame(doc: DesignDoc, frameId: string, componentId: string, mint: () => string): DesignDoc | null {
  const located = locateDesign(doc, frameId)
  if (!located || located.node.kind !== 'frame') return null
  if (doc.components.some((component) => component.id === componentId)) return null
  const source = copyTree(located.node, mint)
  source.x = 0
  source.y = 0
  const component: DesignComponent = {
    id: componentId,
    name: located.node.name?.trim() || 'Component',
    variants: [{ props: {}, node: source }],
  }
  const next: DesignDoc = { ...doc, components: [...doc.components, component] }
  if (!located.parentId) return next
  const instance: DesignNode = {
    id: mint(),
    kind: 'instance',
    x: located.node.x,
    y: located.node.y,
    w: located.node.w,
    h: located.node.h,
    component: componentId,
  }
  return updateDesignNode(next, located.parentId, (parent) => ({
    ...parent,
    children: (parent.children ?? []).map((child) => (child.id === frameId ? instance : child)),
  }))
}

export function addComponentVariant(doc: DesignDoc, componentId: string, props: Record<string, string>, mint: () => string): DesignDoc | null {
  const index = doc.components.findIndex((component) => component.id === componentId)
  if (index < 0) return null
  const component = doc.components[index]
  const source = component.variants[component.variants.length - 1]
  if (!source) return null
  const clean = cleanProps(props)
  if (component.variants.some((variant) => variantKey(variant.props) === variantKey(clean))) return null
  const node = copyTree(source.node, mint)
  node.x = 0
  node.y = 0
  const variants = [...component.variants, { props: clean, node }]
  const components = doc.components.slice()
  components[index] = syncAxes({ ...component, variants })
  return { ...doc, components }
}

export function setVariantProps(doc: DesignDoc, componentId: string, rootId: string, props: Record<string, string>): DesignDoc | null {
  const index = doc.components.findIndex((component) => component.id === componentId)
  if (index < 0) return null
  const component = doc.components[index]
  const at = component.variants.findIndex((variant) => variant.node.id === rootId)
  if (at < 0) return null
  const clean = cleanProps(props)
  if (component.variants.some((variant, i) => i !== at && variantKey(variant.props) === variantKey(clean))) return null
  const variants = component.variants.slice()
  variants[at] = { ...variants[at], props: clean }
  const components = doc.components.slice()
  components[index] = syncAxes({ ...component, variants })
  return { ...doc, components }
}

export function deleteDesignComponent(doc: DesignDoc, componentId: string): DesignDoc | null {
  if (!doc.components.some((component) => component.id === componentId)) return null
  return { ...doc, components: doc.components.filter((component) => component.id !== componentId) }
}

export function renameComponent(doc: DesignDoc, componentId: string, raw: string): DesignDoc | null {
  const name = raw.trim()
  if (!name) return null
  const index = doc.components.findIndex((component) => component.id === componentId)
  if (index < 0) return null
  if (doc.components[index].name === name) return doc
  const components = doc.components.slice()
  components[index] = { ...components[index], name }
  return { ...doc, components }
}

export function makeInstance(doc: DesignDoc, componentId: string, id: string, x: number, y: number): DesignNode | null {
  const component = doc.components.find((item) => item.id === componentId)
  if (!component) return null
  const variant = pickVariant(component)
  if (!variant) return null
  return { id, kind: 'instance', x, y, w: variant.node.w, h: variant.node.h, component: componentId }
}

export function setInstanceVariant(doc: DesignDoc, id: string, props: Record<string, string>): DesignDoc | null {
  const located = locateDesign(doc, id)
  if (!located || located.node.kind !== 'instance' || !located.node.component) return null
  const component = doc.components.find((item) => item.id === located.node.component)
  if (!component) return null
  const clean = cleanProps(props)
  const picked = pickVariant(component, clean)
  if (!picked) return null
  return updateDesignNode(doc, id, (node) => {
    const next: DesignNode = { ...node, w: picked.node.w, h: picked.node.h }
    if (Object.keys(clean).length) next.variant = clean
    else delete next.variant
    return next
  })
}

export function resetInstanceOverrides(doc: DesignDoc, id: string): DesignDoc {
  return updateDesignNode(doc, id, (node) => {
    if (node.kind !== 'instance') return node
    const next: DesignNode = { ...node }
    delete next.text
    delete next.fill
    delete next.overrides
    return next
  })
}

/** Lay out the chosen variant and paint this instance's text and fill overrides. */
export function resolveInstanceTree(doc: DesignDoc, node: DesignNode): DesignNode | null {
  if (node.kind !== 'instance' || !node.component) return null
  const component = doc.components.find((item) => item.id === node.component)
  if (!component) return null
  const variant = pickVariant(component, node.variant)
  if (!variant) return null
  const overridden = applyOverrides(variant.node, { text: node.text, fill: node.fill, overrides: node.overrides })
  return layoutDesign({ ...doc, screens: [zeroRoot(overridden)] }).screens[0] ?? null
}

/** Variant frames placed side by side so the canvas can edit them like screens. */
export function componentView(doc: DesignDoc, componentId: string): DesignDoc | null {
  const component = doc.components.find((item) => item.id === componentId)
  if (!component?.variants.length) return null
  let x = 0
  const screens = component.variants.map((variant) => {
    const screen = { ...variant.node, x, y: 0 }
    x += Math.max(variant.node.w, DESIGN_MIN_W) + BOARD_GAP
    return screen
  })
  return { ...doc, screens }
}

/** Write canvas edits back onto one component. Real screens stay put. An empty board deletes the component. */
export function writeComponentView(doc: DesignDoc, componentId: string, view: DesignDoc): DesignDoc | null {
  const index = doc.components.findIndex((item) => item.id === componentId)
  if (index < 0) return null
  const component = doc.components[index]
  const known = new Map(component.variants.map((variant) => [variant.node.id, variant]))
  const variants: DesignVariant[] = []
  for (const screen of view.screens) {
    if (screen.kind !== 'frame') continue
    const node = zeroRoot(screen)
    const prev = known.get(screen.id)
    if (prev) variants.push({ ...prev, node })
    else variants.push({ props: nextVariantProps([...component.variants, ...variants]), node })
  }
  const components = view.components.slice()
  if (!variants.length) components.splice(index, 1)
  else components[index] = syncAxes({ ...component, name: view.components[index]?.name ?? component.name, variants })
  return { ...view, screens: doc.screens, components }
}

/** Move a node forward or back among its siblings, screens included. */
export function orderDesignNode(doc: DesignDoc, id: string, order: DesignOrder): DesignDoc {
  const located = locateDesign(doc, id)
  if (!located) return doc
  if (!located.parentId) {
    const index = doc.screens.findIndex((screen) => screen.id === id)
    const screens = moveIndex(doc.screens, index, order)
    return screens === doc.screens ? doc : { ...doc, screens }
  }
  return updateDesignNode(doc, located.parentId, (parent) => {
    const children = parent.children ?? []
    const next = moveIndex(children, children.findIndex((child) => child.id === id), order)
    return next === children ? parent : { ...parent, children: next }
  })
}

export function setDesignVisible(doc: DesignDoc, id: string, visible: boolean): DesignDoc {
  return updateDesignNode(doc, id, (node) => {
    const next: DesignNode = { ...node }
    if (visible) delete next.visible
    else next.visible = false
    return next
  })
}

export function setDesignLocked(doc: DesignDoc, id: string, locked: boolean): DesignDoc {
  return updateDesignNode(doc, id, (node) => {
    const next: DesignNode = { ...node }
    if (locked) next.locked = true
    else delete next.locked
    return next
  })
}

export function flipDesignNode(doc: DesignDoc, id: string, axis: 'x' | 'y'): DesignDoc {
  return updateDesignNode(doc, id, (node) => {
    const next: DesignNode = { ...node }
    if (axis === 'x') {
      if (next.flipX) delete next.flipX
      else next.flipX = true
    } else if (next.flipY) delete next.flipY
    else next.flipY = true
    return next
  })
}

/**
 * Wrap sibling nodes in a group or a frame. The wrapper takes the union of
 * their boxes and keeps their stacking order. Mixed parents return null.
 */
export function wrapDesignNodes(doc: DesignDoc, ids: string[], kind: 'group' | 'frame', id: string): DesignDoc | null {
  const unique = [...new Set(ids)]
  if (!unique.length) return null
  const located = unique.map((item) => locateDesign(doc, item))
  if (located.some((item) => !item)) return null
  const rows = located as { node: DesignNode; parentId: string | null }[]
  const parentId = rows[0].parentId
  if (rows.some((item) => item.parentId !== parentId)) return null
  if (rows.some((item) => rows.some((other) => other.node.id !== item.node.id && containsNode(item.node, other.node.id)))) return null
  const siblings = parentId == null ? doc.screens : findDesignNode(doc, parentId)?.children ?? []
  const selected = new Set(unique)
  const ordered = siblings.filter((item) => selected.has(item.id))
  if (ordered.length !== unique.length) return null
  const boxes = ordered.map(nodeBounds)
  const minX = Math.min(...boxes.map((box) => box.x))
  const minY = Math.min(...boxes.map((box) => box.y))
  const maxX = Math.max(...boxes.map((box) => box.x + box.w))
  const maxY = Math.max(...boxes.map((box) => box.y + box.h))
  const wrapper = createNode(kind, id, minX, minY)
  wrapper.w = Math.max(1, maxX - minX)
  wrapper.h = Math.max(1, maxY - minY)
  wrapper.children = ordered.map((node) => ({ ...node, x: node.x - minX, y: node.y - minY }))
  let front = -1
  for (let i = 0; i < siblings.length; i++) if (selected.has(siblings[i].id)) front = i
  let insertAt = 0
  for (let i = 0; i < front; i++) if (!selected.has(siblings[i].id)) insertAt += 1
  const removed = unique.reduce((current, item) => deleteDesignNode(current, item), doc)
  return insertDesignNodeAt(removed, parentId, wrapper, insertAt)
}

/** Replace a group with its children. Positions land in the parent's space, including the group's flip and rotation. */
export function unwrapDesignNode(doc: DesignDoc, id: string): DesignDoc | null {
  const located = locateDesign(doc, id)
  if (!located || located.node.kind !== 'group') return null
  const group = located.node
  const children = group.children ?? []
  const siblings = located.parentId == null ? doc.screens : findDesignNode(doc, located.parentId)?.children ?? []
  const index = siblings.findIndex((item) => item.id === id)
  if (index < 0) return null
  let next = deleteDesignNode(doc, id)
  children.forEach((child, offset) => {
    const spun = spinToParent(group, child.x, child.y)
    next = insertDesignNodeAt(next, located.parentId, { ...child, x: spun.x, y: spun.y }, index + offset)
  })
  return next
}

/** Index in document order for a drop on the reversed layer list. Above a row is a higher index. */
export function layerDropIndex(siblingIds: readonly string[], targetId: string, place: 'before' | 'after'): number | null {
  const index = siblingIds.indexOf(targetId)
  if (index < 0) return null
  return place === 'before' ? index + 1 : index
}

/** Add auto layout. Several nodes are framed first. A lone non-frame is framed too. */
export function autoLayoutDesign(doc: DesignDoc, ids: string[], wrapId: string): DesignDoc | null {
  const unique = [...new Set(ids)]
  if (!unique.length) return null
  let next: DesignDoc | null = doc
  let target = unique[0]
  if (unique.length === 1) {
    const node = findDesignNode(doc, unique[0])
    if (!node) return null
    if (node.kind !== 'frame') {
      next = wrapDesignNodes(doc, unique, 'frame', wrapId)
      target = wrapId
    }
  } else {
    next = wrapDesignNodes(doc, unique, 'frame', wrapId)
    target = wrapId
  }
  if (!next) return null
  return updateDesignNode(next, target, (node) => {
    if (node.kind !== 'frame') return node
    if (node.layout) return node
    return { ...node, layout: 'row', gap: node.gap ?? 8, pad: node.pad ?? 8 }
  })
}

/** Reorder within the same parent, or move under a frame or group at an index. */
export function moveDesignNode(doc: DesignDoc, id: string, parentId: string | null, index: number): DesignDoc {
  const located = locateDesign(doc, id)
  if (!located) return doc
  if (parentId === id || (parentId && containsNode(located.node, parentId))) return doc
  if (parentId != null) {
    const parent = findDesignNode(doc, parentId)
    if (!parent || !isDesignContainer(parent.kind)) return doc
  }
  const siblings = parentId == null ? doc.screens : findDesignNode(doc, parentId)?.children ?? []
  if (parentId === located.parentId) {
    const from = siblings.findIndex((item) => item.id === id)
    if (from < 0) return doc
    let to = Math.max(0, Math.min(index, siblings.length))
    if (from < to) to -= 1
    if (to === from) return doc
    const next = siblings.slice()
    const [item] = next.splice(from, 1)
    next.splice(to, 0, item)
    if (parentId == null) return { ...doc, screens: next }
    return updateDesignNode(doc, parentId, (parent) => ({ ...parent, children: next }))
  }
  const anchor = nodeBoxOrigin(doc, id)
  const local = anchor ? canvasToContent(doc, parentId, anchor.x, anchor.y) : null
  if (!local) return doc
  const removed = deleteDesignNode(doc, id)
  return insertDesignNodeAt(removed, parentId, { ...located.node, x: local.x, y: local.y }, index)
}

/** Build a vector node from pen points in a parent's coordinate space. */
export function nodeFromPen(id: string, points: DesignPenPoint[], closed: boolean): DesignNode | null {
  if (!points.length) return null
  const segments: DesignVectorSegment[] = []
  const last = closed ? points.length : points.length - 1
  for (let i = 0; i < last; i++) {
    const start = points[i]
    const end = points[(i + 1) % points.length]
    segments.push({
      start: i,
      end: (i + 1) % points.length,
      tangentStart: start.outgoing,
      tangentEnd: end.incoming,
    })
  }
  const regions: DesignVectorRegion[] = closed && points.length >= 3 ? [{ winding: 'nonzero', loops: [segments.map((_, index) => index)] }] : []
  return fitVectorNode({
    id,
    kind: 'vector',
    name: 'Vector',
    x: 0,
    y: 0,
    w: 1,
    h: 1,
    fill: closed ? '#d9d9d9' : 'none',
    stroke: '#1c1c1c',
    strokeWidth: 2,
    vector: {
      vertices: points.map((point) => ({ x: point.x, y: point.y })),
      segments,
      regions,
    },
  })
}

/** Shift a canvas-space network so its controls sit inside the node box. */
export function fitVectorNode(node: DesignNode): DesignNode {
  const vector = node.vector
  if (!vector?.vertices.length) return node
  let minX = Infinity
  let minY = Infinity
  let maxX = -Infinity
  let maxY = -Infinity
  const consider = (x: number, y: number) => {
    minX = Math.min(minX, x)
    minY = Math.min(minY, y)
    maxX = Math.max(maxX, x)
    maxY = Math.max(maxY, y)
  }
  for (const vertex of vector.vertices) consider(vertex.x, vertex.y)
  for (const segment of vector.segments) {
    const start = vector.vertices[segment.start]
    const end = vector.vertices[segment.end]
    if (!start || !end) continue
    consider(start.x + segment.tangentStart.x, start.y + segment.tangentStart.y)
    consider(end.x + segment.tangentEnd.x, end.y + segment.tangentEnd.y)
  }
  if (!Number.isFinite(minX)) return node
  const shift = (point: DesignVectorPoint) => ({ x: point.x - minX, y: point.y - minY })
  return {
    ...node,
    x: minX,
    y: minY,
    w: Math.max(1, maxX - minX),
    h: Math.max(1, maxY - minY),
    vector: { ...vector, vertices: vector.vertices.map(shift) },
  }
}

export function vectorSvgPath(vector: DesignVector): string {
  if (vector.regions.length) {
    return vector.regions.map((region) => region.loops.map((loop) => loopPath(vector, loop)).join(' ')).join(' ')
  }
  if (!vector.segments.length) {
    const vertex = vector.vertices[0]
    return vertex ? `M ${fmt(vertex.x)} ${fmt(vertex.y)}` : ''
  }
  return vector.segments.map((segment) => segmentPath(vector, segment, true)).join(' ')
}

/** Nodes under a canvas point, front to back, skipping hidden and locked nodes. */
export function stackDesign(doc: DesignDoc, x: number, y: number): DesignNode[] {
  const found: DesignNode[] = []
  for (let i = doc.screens.length - 1; i >= 0; i--) stackIn(doc.screens[i], x, y, found)
  return found
}

/** Ids touched by a canvas rectangle. A fully covered parent stands in for its children. */
export function selectDesignRect(doc: DesignDoc, x: number, y: number, w: number, h: number): string[] {
  const rect = { x: Math.min(x, x + w), y: Math.min(y, y + h), w: Math.abs(w), h: Math.abs(h) }
  if (rect.w < 1 && rect.h < 1) return []
  const hits: { id: string; full: boolean; ancestors: string[] }[] = []
  const walk = (node: DesignNode, ancestors: string[], toCanvas: (px: number, py: number) => DesignVectorPoint) => {
    if (node.visible === false) return
    const box = canvasBox(node, toCanvas)
    if (node.locked) {
      if (boxesIntersect(box, rect)) hits.push({ id: node.id, full: boxInside(box, rect), ancestors: [...ancestors] })
      return
    }
    if (boxesIntersect(box, rect)) hits.push({ id: node.id, full: boxInside(box, rect), ancestors: [...ancestors] })
    const nested = (px: number, py: number) => {
      const parent = spinToParent(node, px, py)
      return toCanvas(parent.x, parent.y)
    }
    for (const child of node.children ?? []) walk(child, [...ancestors, node.id], nested)
  }
  for (const screen of doc.screens) walk(screen, [], (px, py) => ({ x: px, y: py }))
  const full = new Set(hits.filter((hit) => hit.full).map((hit) => hit.id))
  return hits
    .filter((hit) => {
      if (hit.ancestors.some((id) => full.has(id))) return false
      const hasChild = hits.some((other) => other.ancestors.includes(hit.id))
      return !(hasChild && !hit.full)
    })
    .map((hit) => hit.id)
}

/** Canvas point into a node's content space. A null id keeps canvas coordinates. */
export function canvasToContent(doc: DesignDoc, nodeId: string | null, canvasX: number, canvasY: number): DesignVectorPoint | null {
  if (nodeId == null) return { x: canvasX, y: canvasY }
  const path = pathToNode(doc, nodeId)
  if (!path) return null
  let x = canvasX
  let y = canvasY
  for (const node of path) {
    const local = parentPointToContent(node, x, y)
    x = local.x
    y = local.y
  }
  return { x, y }
}

/** Screen angle into the content space of a parent, so a new line stays visually aligned. */
export function contentAngle(doc: DesignDoc, parentId: string | null, degrees: number): number {
  if (!parentId) return degrees
  const path = pathToNode(doc, parentId)
  if (!path) return degrees
  let angle = degrees
  for (const node of path) {
    angle -= node.rotation ?? 0
    if (node.flipX) angle = 180 - angle
    if (node.flipY) angle = -angle
  }
  return angle
}

export type DesignAlignAxis = 'horizontal' | 'vertical'
export type DesignAlignEdge = 'min' | 'center' | 'max' | 'spread'

/** Move nodes so one edge of each canvas box meets the selection’s box. One node meets its parent. Positions stay parent-relative. */
export function alignDesignNodes(doc: DesignDoc, ids: readonly string[], axis: DesignAlignAxis, edge: DesignAlignEdge): DesignDoc {
  if (ids.length === 1 && edge !== 'spread') {
    const located = locateDesign(doc, ids[0])
    if (!located?.parentId) return doc
    const parent = findDesignNode(doc, located.parentId)
    if (!parent) return doc
    const node = located.node
    const x = axis === 'horizontal' ? edge === 'min' ? 0 : edge === 'max' ? parent.w - node.w : (parent.w - node.w) / 2 : node.x
    const y = axis === 'vertical' ? edge === 'min' ? 0 : edge === 'max' ? parent.h - node.h : (parent.h - node.h) / 2 : node.y
    return updateDesignNode(doc, node.id, (current) => {
      const moved: DesignNode = { ...current, x, y }
      if (parent.layout && !current.absolute) moved.absolute = true
      return moved
    })
  }
  const boxes: { id: string; parentId: string | null; box: { x: number; y: number; w: number; h: number }; stick: boolean }[] = []
  for (const id of ids) {
    const path = pathToNode(doc, id)
    const located = locateDesign(doc, id)
    if (!path || !located) continue
    let toCanvas = (x: number, y: number) => ({ x, y })
    for (let index = 0; index < path.length - 1; index++) {
      const node = path[index]
      const outer = toCanvas
      toCanvas = (x, y) => {
        const parent = spinToParent(node, x, y)
        return outer(parent.x, parent.y)
      }
    }
    const parent = located.parentId ? findDesignNode(doc, located.parentId) : null
    boxes.push({
      id,
      parentId: located.parentId,
      box: canvasBox(located.node, toCanvas),
      stick: !!parent?.layout && !located.node.absolute,
    })
  }
  if (boxes.length < 2) return doc
  if (edge === 'spread') {
    if (boxes.length < 3) return doc
    const ordered = boxes.slice().sort((a, b) => axis === 'horizontal' ? a.box.x - b.box.x : a.box.y - b.box.y)
    const first = ordered[0]
    const last = ordered[ordered.length - 1]
    const span = axis === 'horizontal' ? last.box.x + last.box.w - first.box.x : last.box.y + last.box.h - first.box.y
    const used = ordered.reduce((sum, item) => sum + (axis === 'horizontal' ? item.box.w : item.box.h), 0)
    const gap = (span - used) / (ordered.length - 1)
    let cursor = axis === 'horizontal' ? first.box.x : first.box.y
    const moves: { id: string; x: number; y: number; stick: boolean }[] = []
    for (const item of ordered) {
      const dx = axis === 'horizontal' ? cursor - item.box.x : 0
      const dy = axis === 'vertical' ? cursor - item.box.y : 0
      const delta = canvasDeltaToSpace(doc, item.parentId, dx, dy)
      const node = locateDesign(doc, item.id)?.node
      if (node) moves.push({ id: item.id, x: node.x + delta.x, y: node.y + delta.y, stick: item.stick })
      cursor += (axis === 'horizontal' ? item.box.w : item.box.h) + gap
    }
    let next = doc
    for (const move of moves) {
      next = updateDesignNode(next, move.id, (node) => {
        const moved: DesignNode = { ...node, x: move.x, y: move.y }
        if (move.stick) moved.absolute = true
        return moved
      })
    }
    return next
  }
  const minX = Math.min(...boxes.map((item) => item.box.x))
  const minY = Math.min(...boxes.map((item) => item.box.y))
  const maxX = Math.max(...boxes.map((item) => item.box.x + item.box.w))
  const maxY = Math.max(...boxes.map((item) => item.box.y + item.box.h))
  const moves: { id: string; x: number; y: number; stick: boolean }[] = []
  for (const item of boxes) {
    const dx = axis === 'horizontal' ? (edge === 'min' ? minX : edge === 'max' ? maxX - item.box.w : (minX + maxX) / 2 - item.box.w / 2) - item.box.x : 0
    const dy = axis === 'vertical' ? (edge === 'min' ? minY : edge === 'max' ? maxY - item.box.h : (minY + maxY) / 2 - item.box.h / 2) - item.box.y : 0
    const delta = canvasDeltaToSpace(doc, item.parentId, dx, dy)
    const node = locateDesign(doc, item.id)?.node
    if (!node) continue
    moves.push({ id: item.id, x: node.x + delta.x, y: node.y + delta.y, stick: item.stick })
  }
  let next = doc
  for (const move of moves) {
    next = updateDesignNode(next, move.id, (node) => {
      const moved: DesignNode = { ...node, x: move.x, y: move.y }
      if (move.stick) moved.absolute = true
      return moved
    })
  }
  return next
}

/** Screen delta into the content space of a node. A null id keeps the screen delta. */
export function canvasDeltaToSpace(doc: DesignDoc, nodeId: string | null, dx: number, dy: number): DesignVectorPoint {
  if (!nodeId) return { x: dx, y: dy }
  const path = pathToNode(doc, nodeId)
  if (!path) return { x: dx, y: dy }
  let x = dx
  let y = dy
  for (const node of path) {
    const unrotated = rotateAround(x, y, 0, 0, -(node.rotation ?? 0))
    x = unrotated.x * (node.flipX ? -1 : 1)
    y = unrotated.y * (node.flipY ? -1 : 1)
  }
  return { x, y }
}

function moveIndex<T>(items: T[], index: number, order: DesignOrder): T[] {
  if (index < 0) return items
  const to = order === 'front' ? items.length - 1 : order === 'back' ? 0 : order === 'forward' ? Math.min(items.length - 1, index + 1) : Math.max(0, index - 1)
  if (to === index) return items
  const next = items.slice()
  const [item] = next.splice(index, 1)
  next.splice(to, 0, item)
  return next
}

function nodeBounds(node: DesignNode): { x: number; y: number; w: number; h: number } {
  const rotation = node.rotation ?? 0
  if (!rotation && !node.flipX && !node.flipY) return { x: node.x, y: node.y, w: node.w, h: node.h }
  const corners = [spinToParent(node, 0, 0), spinToParent(node, node.w, 0), spinToParent(node, 0, node.h), spinToParent(node, node.w, node.h)]
  const minX = Math.min(...corners.map((point) => point.x))
  const minY = Math.min(...corners.map((point) => point.y))
  const maxX = Math.max(...corners.map((point) => point.x))
  const maxY = Math.max(...corners.map((point) => point.y))
  return { x: minX, y: minY, w: maxX - minX, h: maxY - minY }
}

function scaleVector(vector: DesignVector, sx: number, sy: number): DesignVector {
  const scale = (point: DesignVectorPoint) => ({ x: point.x * sx, y: point.y * sy })
  return {
    vertices: vector.vertices.map(scale),
    segments: vector.segments.map((segment) => ({
      ...segment,
      tangentStart: scale(segment.tangentStart),
      tangentEnd: scale(segment.tangentEnd),
    })),
    regions: vector.regions.map((region) => ({ winding: region.winding, loops: region.loops.map((loop) => loop.slice()) })),
  }
}

function loopPath(vector: DesignVector, loop: number[]): string {
  let path = ''
  for (const index of loop) {
    const segment = vector.segments[index]
    if (!segment) continue
    path += segmentPath(vector, segment, path === '')
  }
  return path ? `${path} Z` : ''
}

function segmentPath(vector: DesignVector, segment: DesignVectorSegment, move: boolean): string {
  const start = vector.vertices[segment.start]
  const end = vector.vertices[segment.end]
  if (!start || !end) return ''
  const head = move ? `M ${fmt(start.x)} ${fmt(start.y)} ` : ''
  return `${head}C ${fmt(start.x + segment.tangentStart.x)} ${fmt(start.y + segment.tangentStart.y)} ${fmt(end.x + segment.tangentEnd.x)} ${fmt(end.y + segment.tangentEnd.y)} ${fmt(end.x)} ${fmt(end.y)}`
}

function fmt(value: number): string {
  return String(Math.round(value * 100) / 100)
}

function stackIn(node: DesignNode, x: number, y: number, into: DesignNode[]) {
  if (node.visible === false) return
  const local = parentPointToContent(node, x, y)
  if (!insideNode(node, local, node.kind === 'line' ? 6 : 0)) return
  const children = node.children ?? []
  for (let i = children.length - 1; i >= 0; i--) stackIn(children[i], local.x, local.y, into)
  if (!node.locked) into.push(node)
}

function pathToNode(doc: DesignDoc, id: string): DesignNode[] | null {
  for (const screen of doc.screens) {
    const path = pathIn(screen, id)
    if (path) return path
  }
  return null
}

function pathIn(node: DesignNode, id: string): DesignNode[] | null {
  if (node.id === id) return [node]
  for (const child of node.children ?? []) {
    const path = pathIn(child, id)
    if (path) return [node, ...path]
  }
  return null
}

/** A content-space point of a node, expressed in its parent's space. */
function spinToParent(node: DesignNode, localX: number, localY: number): DesignVectorPoint {
  const cx = node.w / 2
  const cy = node.h / 2
  const scaledX = (localX - cx) * (node.flipX ? -1 : 1)
  const scaledY = (localY - cy) * (node.flipY ? -1 : 1)
  const rotated = rotateAround(scaledX, scaledY, 0, 0, node.rotation ?? 0)
  return { x: node.x + cx + rotated.x, y: node.y + cy + rotated.y }
}

/** A parent-space point, expressed in the node's content space. */
function parentPointToContent(node: DesignNode, px: number, py: number): DesignVectorPoint {
  const cx = node.w / 2
  const cy = node.h / 2
  const unrotated = rotateAround(px - node.x - cx, py - node.y - cy, 0, 0, -(node.rotation ?? 0))
  return { x: unrotated.x * (node.flipX ? -1 : 1) + cx, y: unrotated.y * (node.flipY ? -1 : 1) + cy }
}

function rotateAround(x: number, y: number, cx: number, cy: number, degrees: number): DesignVectorPoint {
  if (!degrees) return { x, y }
  const radians = (degrees * Math.PI) / 180
  const cos = Math.cos(radians)
  const sin = Math.sin(radians)
  const dx = x - cx
  const dy = y - cy
  return { x: cx + dx * cos - dy * sin, y: cy + dx * sin + dy * cos }
}

function insideNode(node: DesignNode, content: DesignVectorPoint, pad: number): boolean {
  return content.x >= -pad && content.y >= -pad && content.x <= node.w + pad && content.y <= node.h + pad
}

function canvasBox(node: DesignNode, toCanvas: (px: number, py: number) => DesignVectorPoint): { x: number; y: number; w: number; h: number } {
  const corners = [spinToParent(node, 0, 0), spinToParent(node, node.w, 0), spinToParent(node, 0, node.h), spinToParent(node, node.w, node.h)].map((point) => toCanvas(point.x, point.y))
  const minX = Math.min(...corners.map((point) => point.x))
  const minY = Math.min(...corners.map((point) => point.y))
  const maxX = Math.max(...corners.map((point) => point.x))
  const maxY = Math.max(...corners.map((point) => point.y))
  return { x: minX, y: minY, w: maxX - minX, h: maxY - minY }
}

function boxesIntersect(a: { x: number; y: number; w: number; h: number }, b: { x: number; y: number; w: number; h: number }): boolean {
  return a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y
}

function boxInside(inner: { x: number; y: number; w: number; h: number }, outer: { x: number; y: number; w: number; h: number }): boolean {
  return inner.x >= outer.x && inner.y >= outer.y && inner.x + inner.w <= outer.x + outer.w && inner.y + inner.h <= outer.y + outer.h
}

export type DesignRect = { x: number; y: number; w: number; h: number }

export type DesignGuide = { axis: 'x' | 'y'; at: number; from: number; to: number }

export type DesignMeasure = { axis: 'x' | 'y'; x: number; y: number; length: number; label: string }

export type DesignSnap = {
  dx: number
  dy: number
  snappedX: boolean
  snappedY: boolean
  guides: DesignGuide[]
  measures: DesignMeasure[]
}

export type DesignStyle = {
  fill?: string
  stroke?: string
  strokeWidth?: number
  radius?: number | string
  radiusTL?: number | string
  radiusTR?: number | string
  radiusBR?: number | string
  radiusBL?: number | string
  opacity?: number
  fontSize?: number
  fontFamily?: string
  weight?: DesignWeight
  textAlign?: DesignTextAlign
  textVertical?: DesignTextVertical
  textHug?: DesignTextHug
  lineHeight?: number
  letterSpacing?: number
  color?: string
}

/** Axis-aligned canvas box, including this node's flip and rotation. */
export function designCanvasBox(doc: DesignDoc, id: string): DesignRect | null {
  const path = pathToNode(doc, id)
  if (!path?.length) return null
  let toCanvas = (x: number, y: number) => ({ x, y })
  for (let index = 0; index < path.length - 1; index++) {
    const node = path[index]
    const outer = toCanvas
    toCanvas = (x, y) => {
      const spun = spinToParent(node, x, y)
      return outer(spun.x, spun.y)
    }
  }
  const node = path[path.length - 1]
  if (!node) return null
  return canvasBox(node, toCanvas)
}

/** Union canvas box for a selection, ignoring nodes nested under another selected node. */
export function designSelectionCanvasBox(doc: DesignDoc, ids: string[]): DesignRect | null {
  if (!ids.length) return null
  const set = new Set(ids)
  const top = ids.filter((id) => {
    const path = pathToNode(doc, id)
    if (!path || path.length < 2) return true
    for (let index = 0; index < path.length - 1; index++) {
      if (set.has(path[index].id)) return false
    }
    return true
  })
  const boxes = top.flatMap((id) => {
    const box = designCanvasBox(doc, id)
    return box ? [box] : []
  })
  return unionRects(boxes)
}

/** The moving union, and every other visible box. A moving node's descendants are not targets. */
export function designSnapScene(doc: DesignDoc, ids: string[]): { moving: DesignRect; targets: DesignRect[] } | null {
  const movingBoxes: DesignRect[] = []
  for (const id of ids) {
    const box = designCanvasBox(doc, id)
    if (box) movingBoxes.push(box)
  }
  const moving = unionRects(movingBoxes)
  if (!moving) return null
  const targets: DesignRect[] = []
  const skip = new Set(ids)
  for (const screen of doc.screens) collectSnapBoxes(screen, (x, y) => ({ x, y }), skip, targets)
  return { moving, targets }
}

/**
 * Snap a canvas move to other boxes' edges and centers.
 * dx/dy are the proposed canvas delta. The result is the delta that lands on the snap.
 */
export function designObjectSnap(moving: DesignRect, targets: DesignRect[], dx: number, dy: number, threshold: number): DesignSnap {
  const proposed = { x: moving.x + dx, y: moving.y + dy, w: moving.w, h: moving.h }
  const xSnap = snapAxis(proposed.x, proposed.x + proposed.w, targets, 'x', threshold)
  const ySnap = snapAxis(proposed.y, proposed.y + proposed.h, targets, 'y', threshold)
  const landed = {
    x: proposed.x + (xSnap?.delta ?? 0),
    y: proposed.y + (ySnap?.delta ?? 0),
    w: proposed.w,
    h: proposed.h,
  }
  const guides: DesignGuide[] = []
  if (xSnap) guides.push({ axis: 'x', at: xSnap.at, ...guideSpan(xSnap.at, 'x', landed, targets) })
  if (ySnap) guides.push({ axis: 'y', at: ySnap.at, ...guideSpan(ySnap.at, 'y', landed, targets) })
  return {
    dx: dx + (xSnap?.delta ?? 0),
    dy: dy + (ySnap?.delta ?? 0),
    snappedX: xSnap != null,
    snappedY: ySnap != null,
    guides,
    measures: designMeasures(landed, targets),
  }
}

/** Fit boxes in a viewport. The canvas rulers occupy the top and left 16px. */
export function frameDesignView(
  boxes: DesignRect[],
  width: number,
  height: number,
  minZoom: number,
  maxZoom: number,
): { panX: number; panY: number; zoom: number } | null {
  const union = unionRects(boxes.filter((box) => box.w > 0 && box.h > 0))
  if (!union || width < 1 || height < 1) return null
  const pad = 48
  const origin = 16
  const viewW = Math.max(1, width - origin - pad)
  const viewH = Math.max(1, height - origin - pad)
  const zoom = Math.min(maxZoom, Math.max(minZoom, Math.min(viewW / union.w, viewH / union.h)))
  return {
    zoom,
    panX: origin + (viewW - union.w * zoom) / 2 - union.x * zoom,
    panY: origin + (viewH - union.h * zoom) / 2 - union.y * zoom,
  }
}

/**
 * Move a selection by a canvas pixel step.
 * An in-flow auto-layout child reorders on the main axis and ignores the cross axis.
 * A locked node stays. A selected descendant of another selected node stays with its parent.
 */
export function nudgeDesignNodes(doc: DesignDoc, ids: string[], dx: number, dy: number): DesignDoc {
  if (!dx && !dy) return doc
  const roots = designRoots(doc, ids).flatMap((id) => {
    const row = locateDesign(doc, id)
    return row && !row.node.locked ? [row] : []
  })
  if (!roots.length) return doc
  let next = doc
  const flow: { id: string; sign: number; parentId: string }[] = []
  const flowing = new Set<string>()
  for (const row of roots) {
    const parent = row.parentId ? findDesignNode(doc, row.parentId) : null
    if (parent?.layout && !row.node.absolute && row.parentId) {
      const main = parent.layout === 'row' ? dx : dy
      if (!main) continue
      flow.push({ id: row.node.id, sign: main > 0 ? 1 : -1, parentId: row.parentId })
      flowing.add(row.node.id)
      continue
    }
    next = updateDesignNode(next, row.node.id, (node) => ({ ...node, x: node.x + dx, y: node.y + dy }))
  }
  const byParent = new Map<string, { id: string; sign: number }[]>()
  for (const item of flow) {
    const list = byParent.get(item.parentId) ?? []
    list.push({ id: item.id, sign: item.sign })
    byParent.set(item.parentId, list)
  }
  for (const items of byParent.values()) {
    const sign = items[0]?.sign ?? 0
    const pending = items.slice()
    pending.sort((a, b) => flowIndex(next, b.id) - flowIndex(next, a.id))
    if (sign < 0) pending.reverse()
    for (const item of pending) {
      const located = locateDesign(next, item.id)
      const parent = located?.parentId ? findDesignNode(next, located.parentId) : null
      const siblings = (parent?.children ?? []).filter((child) => !child.absolute)
      const index = siblings.findIndex((child) => child.id === item.id)
      const dest = index + item.sign
      const neighbor = siblings[dest]
      if (index < 0 || !neighbor || flowing.has(neighbor.id)) continue
      next = reorderDesignNode(next, item.id, dest)
    }
  }
  return next
}

/** Copy a selection into the same parents, one slot in front, shifted by dx/dy. Copies are unlocked. */
export function duplicateDesignNodes(
  doc: DesignDoc,
  ids: string[],
  dx: number,
  dy: number,
  mint: () => string,
): { doc: DesignDoc; ids: string[] } | null {
  const roots = designRoots(doc, ids)
  const made: { source: string; id: string }[] = []
  let next = doc
  const byParent = new Map<string | null, string[]>()
  for (const id of roots) {
    const row = locateDesign(next, id)
    if (!row) continue
    const list = byParent.get(row.parentId) ?? []
    list.push(id)
    byParent.set(row.parentId, list)
  }
  for (const [parentId, list] of byParent) {
    const ordered = list.slice().sort((a, b) => siblingIndex(next, b) - siblingIndex(next, a))
    for (const id of ordered) {
      const row = locateDesign(next, id)
      if (!row) continue
      const index = siblingIndex(next, id)
      if (index < 0) continue
      const copy = copyTree(row.node, mint, dx, dy)
      delete copy.locked
      next = insertDesignNodeAt(next, parentId, copy, index + 1)
      made.push({ source: id, id: copy.id })
    }
  }
  if (!made.length) return null
  const rank = new Map(ids.map((id, index) => [id, index]))
  made.sort((a, b) => (rank.get(a.source) ?? 0) - (rank.get(b.source) ?? 0))
  return { doc: next, ids: made.map((item) => item.id) }
}

/** Every sibling of the selection. An empty or mixed selection selects the screens. */
export function selectAllDesign(doc: DesignDoc, ids: string[]): string[] {
  const rows = ids.flatMap((id) => {
    const row = locateDesign(doc, id)
    return row ? [row] : []
  })
  const parentId = rows[0]?.parentId
  const same = rows.length > 0 && rows.every((row) => row.parentId === parentId)
  if (!same || parentId == null) return doc.screens.map((screen) => screen.id)
  return findDesignNode(doc, parentId)?.children?.map((child) => child.id) ?? []
}

/** Paint and text fields that are actually set. Omitted fields are left alone on paste. */
export function designStyle(node: DesignNode): DesignStyle {
  const style: DesignStyle = {}
  if (node.fill !== undefined) style.fill = node.fill
  if (node.stroke !== undefined) style.stroke = node.stroke
  if (node.strokeWidth !== undefined) style.strokeWidth = node.strokeWidth
  if (node.radius !== undefined) style.radius = node.radius
  if (node.radiusTL !== undefined) style.radiusTL = node.radiusTL
  if (node.radiusTR !== undefined) style.radiusTR = node.radiusTR
  if (node.radiusBR !== undefined) style.radiusBR = node.radiusBR
  if (node.radiusBL !== undefined) style.radiusBL = node.radiusBL
  if (node.opacity !== undefined) style.opacity = node.opacity
  if (node.fontSize !== undefined) style.fontSize = node.fontSize
  if (node.fontFamily !== undefined) style.fontFamily = node.fontFamily
  if (node.weight !== undefined) style.weight = node.weight
  if (node.textAlign !== undefined) style.textAlign = node.textAlign
  if (node.textVertical !== undefined) style.textVertical = node.textVertical
  if (node.textHug !== undefined) style.textHug = node.textHug
  if (node.lineHeight !== undefined) style.lineHeight = node.lineHeight
  if (node.letterSpacing !== undefined) style.letterSpacing = node.letterSpacing
  if (node.color !== undefined) style.color = node.color
  return style
}

/** Write a copied style. Text fields land on text nodes. Locked nodes stay. */
export function applyDesignStyle(doc: DesignDoc, ids: string[], style: DesignStyle): DesignDoc {
  let next = doc
  for (const id of ids) {
    next = updateDesignNode(next, id, (node) => {
      if (node.locked) return node
      const copy: DesignNode = { ...node }
      if (style.fill !== undefined) copy.fill = style.fill
      if (style.stroke !== undefined) copy.stroke = style.stroke
      if (style.strokeWidth !== undefined) copy.strokeWidth = style.strokeWidth
      if (style.radius !== undefined) copy.radius = style.radius
      if (style.radiusTL !== undefined) copy.radiusTL = style.radiusTL
      if (style.radiusTR !== undefined) copy.radiusTR = style.radiusTR
      if (style.radiusBR !== undefined) copy.radiusBR = style.radiusBR
      if (style.radiusBL !== undefined) copy.radiusBL = style.radiusBL
      if (style.opacity !== undefined) copy.opacity = style.opacity
      if (node.kind !== 'text') return copy
      if (style.fontSize !== undefined) copy.fontSize = style.fontSize
      if (style.fontFamily !== undefined) copy.fontFamily = style.fontFamily
      if (style.weight !== undefined) copy.weight = style.weight
      if (style.textAlign !== undefined) copy.textAlign = style.textAlign
      if (style.textVertical !== undefined) copy.textVertical = style.textVertical
      if (style.textHug !== undefined) copy.textHug = style.textHug
      if (style.lineHeight !== undefined) copy.lineHeight = style.lineHeight
      if (style.letterSpacing !== undefined) copy.letterSpacing = style.letterSpacing
      if (style.color !== undefined) copy.color = style.color
      return copy
    })
  }
  return next
}

function designRoots(doc: DesignDoc, ids: string[]): string[] {
  const chosen = new Set(ids)
  return ids.filter((id) => {
    const path = pathToNode(doc, id)
    if (!path) return false
    return !path.slice(0, -1).some((node) => chosen.has(node.id))
  })
}

function siblingIndex(doc: DesignDoc, id: string): number {
  const row = locateDesign(doc, id)
  if (!row) return -1
  const siblings = row.parentId == null ? doc.screens : findDesignNode(doc, row.parentId)?.children ?? []
  return siblings.findIndex((node) => node.id === id)
}

function flowIndex(doc: DesignDoc, id: string): number {
  const row = locateDesign(doc, id)
  const parent = row?.parentId ? findDesignNode(doc, row.parentId) : null
  return (parent?.children ?? []).filter((child) => !child.absolute).findIndex((child) => child.id === id)
}

function unionRects(rects: DesignRect[]): DesignRect | null {
  if (!rects.length) return null
  let x0 = Infinity
  let y0 = Infinity
  let x1 = -Infinity
  let y1 = -Infinity
  for (const rect of rects) {
    x0 = Math.min(x0, rect.x)
    y0 = Math.min(y0, rect.y)
    x1 = Math.max(x1, rect.x + rect.w)
    y1 = Math.max(y1, rect.y + rect.h)
  }
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

function collectSnapBoxes(
  node: DesignNode,
  toCanvas: (x: number, y: number) => DesignVectorPoint,
  skip: Set<string>,
  into: DesignRect[],
) {
  if (node.visible === false || skip.has(node.id)) return
  into.push(canvasBox(node, toCanvas))
  if (!node.children?.length) return
  const outer = toCanvas
  const next = (x: number, y: number) => {
    const spun = spinToParent(node, x, y)
    return outer(spun.x, spun.y)
  }
  for (const child of node.children) collectSnapBoxes(child, next, skip, into)
}

function snapAxis(
  min: number,
  max: number,
  targets: DesignRect[],
  axis: 'x' | 'y',
  threshold: number,
): { delta: number; at: number } | null {
  const mid = (min + max) / 2
  const moving = [min, mid, max]
  let best: { abs: number; delta: number; at: number; rank: number } | null = null
  for (const target of targets) {
    const start = axis === 'x' ? target.x : target.y
    const end = start + (axis === 'x' ? target.w : target.h)
    const points = [start, (start + end) / 2, end]
    for (let movingIndex = 0; movingIndex < moving.length; movingIndex++) {
      for (let targetIndex = 0; targetIndex < points.length; targetIndex++) {
        const at = points[targetIndex]
        const from = moving[movingIndex]
        if (at == null || from == null) continue
        const delta = at - from
        const abs = Math.abs(delta)
        if (abs > threshold) continue
        const rank = movingIndex === 1 || targetIndex === 1 ? 1 : 0
        if (!best || abs < best.abs - 0.01 || (Math.abs(abs - best.abs) <= 0.01 && rank < best.rank)) {
          best = { abs, delta, at, rank }
        }
      }
    }
  }
  return best ? { delta: best.delta, at: best.at } : null
}

function guideSpan(at: number, axis: 'x' | 'y', moving: DesignRect, targets: DesignRect[]): { from: number; to: number } {
  let from = axis === 'x' ? moving.y : moving.x
  let to = from + (axis === 'x' ? moving.h : moving.w)
  for (const target of targets) {
    const start = axis === 'x' ? target.x : target.y
    const end = start + (axis === 'x' ? target.w : target.h)
    const mid = (start + end) / 2
    if (Math.abs(start - at) > 0.5 && Math.abs(end - at) > 0.5 && Math.abs(mid - at) > 0.5) continue
    const cross = axis === 'x' ? target.y : target.x
    const crossEnd = cross + (axis === 'x' ? target.h : target.w)
    from = Math.min(from, cross)
    to = Math.max(to, crossEnd)
  }
  return { from, to }
}

function designMeasures(moved: DesignRect, targets: DesignRect[]): DesignMeasure[] {
  const measures: DesignMeasure[] = []
  const overlap = (a0: number, a1: number, b0: number, b1: number) => Math.min(a1, b1) - Math.max(a0, b0)
  let left: { gap: number; target: DesignRect } | null = null
  let right: { gap: number; target: DesignRect } | null = null
  let above: { gap: number; target: DesignRect } | null = null
  let below: { gap: number; target: DesignRect } | null = null
  for (const target of targets) {
    const vertical = overlap(moved.y, moved.y + moved.h, target.y, target.y + target.h)
    const horizontal = overlap(moved.x, moved.x + moved.w, target.x, target.x + target.w)
    const gapLeft = moved.x - (target.x + target.w)
    const gapRight = target.x - (moved.x + moved.w)
    const gapAbove = moved.y - (target.y + target.h)
    const gapBelow = target.y - (moved.y + moved.h)
    if (vertical > 0 && gapLeft >= 0.5 && (!left || gapLeft < left.gap)) left = { gap: gapLeft, target }
    if (vertical > 0 && gapRight >= 0.5 && (!right || gapRight < right.gap)) right = { gap: gapRight, target }
    if (horizontal > 0 && gapAbove >= 0.5 && (!above || gapAbove < above.gap)) above = { gap: gapAbove, target }
    if (horizontal > 0 && gapBelow >= 0.5 && (!below || gapBelow < below.gap)) below = { gap: gapBelow, target }
  }
  if (left) {
    const top = Math.max(moved.y, left.target.y)
    const bottom = Math.min(moved.y + moved.h, left.target.y + left.target.h)
    measures.push({ axis: 'x', x: left.target.x + left.target.w, y: (top + bottom) / 2, length: left.gap, label: String(Math.round(left.gap)) })
  }
  if (right) {
    const top = Math.max(moved.y, right.target.y)
    const bottom = Math.min(moved.y + moved.h, right.target.y + right.target.h)
    measures.push({ axis: 'x', x: moved.x + moved.w, y: (top + bottom) / 2, length: right.gap, label: String(Math.round(right.gap)) })
  }
  if (above) {
    const start = Math.max(moved.x, above.target.x)
    const end = Math.min(moved.x + moved.w, above.target.x + above.target.w)
    measures.push({ axis: 'y', x: (start + end) / 2, y: above.target.y + above.target.h, length: above.gap, label: String(Math.round(above.gap)) })
  }
  if (below) {
    const start = Math.max(moved.x, below.target.x)
    const end = Math.min(moved.x + moved.w, below.target.x + below.target.w)
    measures.push({ axis: 'y', x: (start + end) / 2, y: moved.y + moved.h, length: below.gap, label: String(Math.round(below.gap)) })
  }
  return measures
}

/** Move a flow child to a new index among its non-absolute siblings. */
export function reorderDesignNode(doc: DesignDoc, id: string, index: number): DesignDoc {
  const located = locateDesign(doc, id)
  if (!located?.parentId || located.node.absolute) return doc
  return updateDesignNode(doc, located.parentId, (parent) => {
    const children = parent.children ?? []
    const item = children.find((child) => child.id === id)
    if (!item) return parent
    const absolute = children.filter((child) => child.absolute && child.id !== id)
    const flow = children.filter((child) => !child.absolute && child.id !== id)
    const at = Math.max(0, Math.min(index, flow.length))
    flow.splice(at, 0, item)
    return { ...parent, children: [...flow, ...absolute] }
  })
}

function layoutNode(node: DesignNode, frozenId?: string): DesignNode {
  let next = node
  if (node.children?.length) {
    let changed = false
    const children = node.children.map((child) => {
      const laid = layoutNode(child, frozenId)
      if (laid !== child) changed = true
      return laid
    })
    if (changed) next = { ...node, children }
  }
  if (next.kind === 'text') next = hugText(next)
  if (next.kind === 'group') next = fitGroup(next)
  if (next.kind === 'frame' && next.layout) next = placeFlow(next, frozenId)
  return clampNodeBox(next)
}

/** Hug a text node to the half-em estimate. Width hug sets both axes. Height hug wraps into the current width. */
function hugText(node: DesignNode): DesignNode {
  if (!node.textHug) return node
  const box = measureTextBox(node)
  const style = textStyle(node)
  const lineH = style.lineHeight > 0 ? style.lineHeight : Math.round(style.fontSize * 1.2)
  let w = node.w
  let h = node.h
  if (node.textHug === 'width') {
    w = box.w
    h = box.h
  } else {
    const rows = Math.max(style.text.split('\n').length, Math.ceil(box.w / Math.max(1, node.w)))
    h = Math.max(lineH, rows * lineH)
  }
  w = Math.max(1, limitSize(w, node.minW, node.maxW))
  h = Math.max(1, limitSize(h, node.minH, node.maxH))
  if (w === node.w && h === node.h) return node
  return { ...node, w, h }
}

function clampNodeBox(node: DesignNode): DesignNode {
  const w = limitSize(node.w, node.minW, node.maxW)
  const h = limitSize(node.h, node.minH, node.maxH)
  if (w === node.w && h === node.h) return node
  return { ...node, w: Math.max(1, w), h: Math.max(1, h) }
}

/** Pull a group's box onto the union of its children without moving them on the canvas. */
function fitGroup(group: DesignNode): DesignNode {
  const children = group.children
  if (!children?.length) return group
  const boxes = children.map(nodeBounds)
  const minX = Math.min(...boxes.map((box) => box.x))
  const minY = Math.min(...boxes.map((box) => box.y))
  const maxX = Math.max(...boxes.map((box) => box.x + box.w))
  const maxY = Math.max(...boxes.map((box) => box.y + box.h))
  const w = Math.max(1, maxX - minX)
  const h = Math.max(1, maxY - minY)
  if (minX === 0 && minY === 0 && w === group.w && h === group.h) return group
  const shifted = children.map((child) => child.x === child.x - minX && child.y === child.y - minY ? child : { ...child, x: child.x - minX, y: child.y - minY })
  const origin = groupOrigin(group, minX, minY, w, h)
  return { ...group, x: origin.x, y: origin.y, w, h, children: shifted }
}

/** Parent-space origin after the content union slides to (0, 0). Flip and rotation keep each child put. */
function groupOrigin(group: DesignNode, minX: number, minY: number, w: number, h: number): DesignVectorPoint {
  const spun = (x: number, y: number) => {
    const scaledX = x * (group.flipX ? -1 : 1)
    const scaledY = y * (group.flipY ? -1 : 1)
    return rotateAround(scaledX, scaledY, 0, 0, group.rotation ?? 0)
  }
  const oldCenter = { x: group.w / 2, y: group.h / 2 }
  const newCenter = { x: w / 2, y: h / 2 }
  const spunOld = spun(oldCenter.x, oldCenter.y)
  const spunNew = spun(newCenter.x, newCenter.y)
  const spunMin = spun(minX, minY)
  return {
    x: group.x + (oldCenter.x - spunOld.x) - (newCenter.x - spunNew.x) + spunMin.x,
    y: group.y + (oldCenter.y - spunOld.y) - (newCenter.y - spunNew.y) + spunMin.y,
  }
}

function framePad(frame: DesignNode): { top: number; right: number; bottom: number; left: number } {
  const pad = frame.pad ?? 0
  return {
    top: frame.padTop ?? pad,
    right: frame.padRight ?? pad,
    bottom: frame.padBottom ?? pad,
    left: frame.padLeft ?? pad,
  }
}

function limitSize(size: number, min?: number, max?: number): number {
  let next = size
  if (min != null && min > 0) next = Math.max(next, min)
  if (max != null && max > 0) next = Math.min(next, max)
  return next
}

function placeFlow(frame: DesignNode, frozenId?: string): DesignNode {
  const pad = framePad(frame)
  const gap = frame.gap ?? 0
  const horizontal = frame.layout === 'row'
  const mainStart = horizontal ? pad.left : pad.top
  const mainEnd = horizontal ? pad.right : pad.bottom
  const crossStart = horizontal ? pad.top : pad.left
  const crossEnd = horizontal ? pad.bottom : pad.right
  const children = frame.children ?? []
  const flow = children.filter((child) => !child.absolute && child.id !== frozenId)
  const mainSize = (child: DesignNode) => (horizontal ? child.w : child.h)
  const crossSize = (child: DesignNode) => (horizontal ? child.h : child.w)
  const mainMode = (child: DesignNode) => (horizontal ? child.wMode : child.hMode)
  const crossMode = (child: DesignNode) => (horizontal ? child.hMode : child.wMode)
  const mainMin = (child: DesignNode) => (horizontal ? child.minW : child.minH)
  const mainMax = (child: DesignNode) => (horizontal ? child.maxW : child.maxH)
  const crossMin = (child: DesignNode) => (horizontal ? child.minH : child.minW)
  const crossMax = (child: DesignNode) => (horizontal ? child.maxH : child.maxW)
  const hugsMain = horizontal ? frame.wMode === 'hug' : frame.hMode === 'hug'
  const hugsCross = horizontal ? frame.hMode === 'hug' : frame.wMode === 'hug'
  const justify = frame.justify ?? 'start'
  const align = frame.align ?? 'start'
  const wrapping = frame.wrap === true && !hugsMain
  const sizedMain = (child: DesignNode) => limitSize(mainSize(child), mainMin(child), mainMax(child))
  const gaps = Math.max(0, flow.length - 1) * gap
  const contentMain = flow.reduce((sum, child) => sum + (mainMode(child) === 'fill' && !hugsMain ? 0 : sizedMain(child)), 0) + gaps
  const contentCross = flow.reduce((max, child) => Math.max(max, limitSize(crossSize(child), crossMin(child), crossMax(child))), 0)
  let width = frame.w
  let height = frame.h
  if (hugsMain && !wrapping) {
    const size = limitSize(Math.max(horizontal ? DESIGN_MIN_W : DESIGN_MIN_H, mainStart + mainEnd + contentMain), horizontal ? frame.minW : frame.minH, horizontal ? frame.maxW : frame.maxH)
    if (horizontal) width = size
    else height = size
  }
  if (hugsCross && !wrapping) {
    const size = limitSize(Math.max(horizontal ? DESIGN_MIN_H : DESIGN_MIN_W, crossStart + crossEnd + contentCross), horizontal ? frame.minH : frame.minW, horizontal ? frame.maxH : frame.maxW)
    if (horizontal) height = size
    else width = size
  }
  width = Math.max(1, limitSize(width, frame.minW, frame.maxW))
  height = Math.max(1, limitSize(height, frame.minH, frame.maxH))
  const innerMain = Math.max(0, (horizontal ? width : height) - mainStart - mainEnd)
  const innerCross = Math.max(0, (horizontal ? height : width) - crossStart - crossEnd)
  const stretches = (child: DesignNode) => {
    const mode = crossMode(child)
    return mode !== 'hug' && mode !== 'fixed' && (mode === 'fill' || align === 'stretch') && !hugsCross
  }
  const finish = (size: number, min?: number, max?: number) => {
    const limited = limitSize(Math.max(min == null && max == null ? 8 : 1, size), min, max)
    return limited
  }
  if (!wrapping) {
    const fills = hugsMain ? [] : flow.filter((child) => mainMode(child) === 'fill')
    const usedFixed = flow.reduce((sum, child) => sum + (fills.includes(child) ? 0 : sizedMain(child)), 0)
    const fillMain = fills.length ? Math.max(8, (innerMain - usedFixed - gaps) / fills.length) : 0
    const measured = flow.map((child) => {
      const main = fills.includes(child) ? fillMain : sizedMain(child)
      const cross = stretches(child) ? innerCross : crossSize(child)
      return { child, main: finish(main, mainMin(child), mainMax(child)), cross: finish(cross, crossMin(child), crossMax(child)) }
    })
    const used = measured.reduce((sum, item) => sum + item.main, 0) + gaps
    const space = justify === 'space' && measured.length > 1
    const between = space ? Math.max(0, (innerMain - measured.reduce((sum, item) => sum + item.main, 0)) / (measured.length - 1)) : gap
    let cursor = mainStart
    if (!space && justify === 'center') cursor = mainStart + Math.max(0, innerMain - used) / 2
    if (!space && justify === 'end') cursor = mainStart + Math.max(0, innerMain - used)
    const placed = new Map<string, DesignNode>()
    for (const item of measured) {
      const crossPos = align === 'center' ? crossStart + (innerCross - item.cross) / 2 : align === 'end' ? crossStart + innerCross - item.cross : crossStart
      const x = horizontal ? cursor : crossPos
      const y = horizontal ? crossPos : cursor
      const w = horizontal ? item.main : item.cross
      const h = horizontal ? item.cross : item.main
      placed.set(item.child.id, item.child.x === x && item.child.y === y && item.child.w === w && item.child.h === h ? item.child : { ...item.child, x, y, w, h })
      cursor += item.main + between
    }
    const nextChildren = children.map((child) => placed.get(child.id) ?? child)
    if (width === frame.w && height === frame.h && nextChildren.every((child, index) => child === children[index])) return frame
    return { ...frame, w: width, h: height, children: nextChildren }
  }
  const items = flow.map((child) => ({
    child,
    main: finish(sizedMain(child), mainMin(child), mainMax(child)),
    cross: finish(crossSize(child), crossMin(child), crossMax(child)),
  }))
  const lines: { child: DesignNode; main: number; cross: number }[][] = []
  let line: { child: DesignNode; main: number; cross: number }[] = []
  let usedLine = 0
  for (const item of items) {
    const nextUsed = line.length ? usedLine + gap + item.main : item.main
    if (line.length && nextUsed > innerMain) {
      lines.push(line)
      line = [item]
      usedLine = item.main
    } else {
      line.push(item)
      usedLine = nextUsed
    }
  }
  if (line.length) lines.push(line)
  const placed = new Map<string, DesignNode>()
  let crossCursor = crossStart
  for (const row of lines) {
    const lineCross = row.reduce((max, item) => Math.max(max, item.cross), 0)
    const rowMain = row.reduce((sum, item) => sum + item.main, 0)
    const space = justify === 'space' && row.length > 1
    const between = space ? Math.max(0, (innerMain - rowMain) / (row.length - 1)) : gap
    const used = rowMain + (space ? 0 : Math.max(0, row.length - 1) * gap)
    let cursor = mainStart
    if (!space && justify === 'center') cursor = mainStart + Math.max(0, innerMain - used) / 2
    if (!space && justify === 'end') cursor = mainStart + Math.max(0, innerMain - used)
    for (const item of row) {
      const cross = stretches(item.child) ? Math.max(item.cross, lineCross) : item.cross
      const crossPos = align === 'center' ? crossCursor + (lineCross - cross) / 2 : align === 'end' ? crossCursor + lineCross - cross : crossCursor
      const x = horizontal ? cursor : crossPos
      const y = horizontal ? crossPos : cursor
      const w = horizontal ? item.main : cross
      const h = horizontal ? cross : item.main
      placed.set(item.child.id, item.child.x === x && item.child.y === y && item.child.w === w && item.child.h === h ? item.child : { ...item.child, x, y, w, h })
      cursor += item.main + between
    }
    crossCursor += lineCross + gap
  }
  if (hugsCross && lines.length) {
    const size = Math.max(horizontal ? DESIGN_MIN_H : DESIGN_MIN_W, crossCursor - gap + crossEnd)
    const limited = Math.max(1, limitSize(size, horizontal ? frame.minH : frame.minW, horizontal ? frame.maxH : frame.maxW))
    if (horizontal) height = limited
    else width = limited
  }
  const nextChildren = children.map((child) => placed.get(child.id) ?? child)
  if (width === frame.w && height === frame.h && nextChildren.every((child, index) => child === children[index])) return frame
  return { ...frame, w: width, h: height, children: nextChildren }
}

function findInNode(node: DesignNode, id: string): DesignNode | null {
  if (node.id === id) return node
  for (const child of node.children ?? []) {
    const found = findInNode(child, id)
    if (found) return found
  }
  return null
}

function locateIn(node: DesignNode, id: string): { node: DesignNode; parentId: string } | null {
  for (const child of node.children ?? []) {
    if (child.id === id) return { node: child, parentId: node.id }
    const found = locateIn(child, id)
    if (found) return found
  }
  return null
}

function hitIn(node: DesignNode, x: number, y: number, deep: boolean): DesignNode | null {
  if (node.visible === false) return null
  const local = parentPointToContent(node, x, y)
  const inside = insideNode(node, local, node.kind === 'line' ? 6 : 0)
  if (node.locked) return inside ? node : null
  if (node.kind === 'group' || node.kind === 'instance') {
    if (!inside) return null
    if (deep) {
      const children = node.children ?? []
      for (let i = children.length - 1; i >= 0; i--) {
        const found = hitIn(children[i], local.x, local.y, false)
        if (found) return found
      }
    }
    return node
  }
  if (node.kind === 'frame' && node.clip !== false && !inside) return null
  const children = node.children ?? []
  for (let i = children.length - 1; i >= 0; i--) {
    const found = hitIn(children[i], local.x, local.y, deep)
    if (found) return found
  }
  if (!inside) return null
  return node
}

function frameIn(node: DesignNode, x: number, y: number, ignoreId: string): string | null {
  if (node.id === ignoreId || node.visible === false || node.locked) return null
  const local = parentPointToContent(node, x, y)
  if (!insideNode(node, local, 0)) return null
  if (isDesignContainer(node.kind)) {
    const children = node.children ?? []
    for (let i = children.length - 1; i >= 0; i--) {
      const found = frameIn(children[i], local.x, local.y, ignoreId)
      if (found) return found
    }
    return node.id
  }
  return null
}

function containsNode(node: DesignNode, id: string): boolean {
  if (node.id === id) return true
  return (node.children ?? []).some((child) => containsNode(child, id))
}

function replaceNode(node: DesignNode, id: string, fn: (node: DesignNode) => DesignNode): DesignNode {
  if (node.id === id) return fn(node)
  if (!node.children?.length) return node
  let changed = false
  const children = node.children.map((child) => {
    const next = replaceNode(child, id, fn)
    if (next !== child) changed = true
    return next
  })
  return changed ? { ...node, children } : node
}

function withoutNode(node: DesignNode, id: string): DesignNode | null {
  if (node.id === id) return null
  if (!node.children?.length) return node
  let changed = false
  const children: DesignNode[] = []
  for (const child of node.children) {
    const next = withoutNode(child, id)
    if (next !== child) changed = true
    if (next) children.push(next)
  }
  if (!changed) return node
  return { ...node, children: children.length ? children : undefined }
}

function addChild(node: DesignNode, parentId: string, child: DesignNode, index: number): DesignNode {
  if (node.id === parentId && isDesignContainer(node.kind)) {
    const children = (node.children ?? []).slice()
    children.splice(Math.max(0, Math.min(index, children.length)), 0, child)
    return { ...node, children }
  }
  if (!node.children?.length) return node
  let changed = false
  const children = node.children.map((item) => {
    const next = addChild(item, parentId, child, index)
    if (next !== item) changed = true
    return next
  })
  return changed ? { ...node, children } : node
}

function num(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function oneOf<T extends string>(value: unknown, allowed: readonly T[]): T | undefined {
  return typeof value === 'string' && (allowed as readonly string[]).includes(value) ? (value as T) : undefined
}

function parseColor(value: string): string | undefined {
  return /^#[0-9a-fA-F]{6}$/.test(value) ? value.toLowerCase() : undefined
}

function parseTokenName(value: unknown): string | undefined {
  return typeof value === 'string' && TOKEN_NAME.test(value) ? value : undefined
}

/** `none`, a hex color, or a token name. */
function parsePaint(value: unknown): string | undefined {
  if (value === 'none') return 'none'
  if (typeof value !== 'string' || !value) return undefined
  return parseColor(value) ?? parseTokenName(value)
}

function parseRadius(value: unknown): number | string | undefined {
  const size = num(value)
  if (size != null && size >= 0) return size
  return parseTokenName(value)
}

function parseProps(value: unknown): Record<string, string> | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined
  const props: Record<string, string> = {}
  for (const [key, item] of Object.entries(value)) {
    if (!TOKEN_NAME.test(key) || typeof item !== 'string' || !item) return undefined
    props[key] = item
  }
  return props
}

function parseAxes(value: unknown): Record<string, string[]> | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined
  const axes: Record<string, string[]> = {}
  for (const [key, item] of Object.entries(value)) {
    if (!TOKEN_NAME.test(key) || !Array.isArray(item) || item.length === 0) return undefined
    const values: string[] = []
    for (const entry of item) {
      if (typeof entry !== 'string' || !entry) return undefined
      values.push(entry)
    }
    axes[key] = values
  }
  return axes
}

function parseNode(value: unknown): DesignNode | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const row = value as Record<string, unknown>
  const kind = oneOf(row.kind, KINDS)
  const x = num(row.x)
  const y = num(row.y)
  const w = num(row.w)
  const h = num(row.h)
  if (!kind || typeof row.id !== 'string' || !row.id || x == null || y == null || w == null || h == null || w <= 0 || h <= 0) return null
  const node: DesignNode = { id: row.id, kind, x, y, w, h }
  if (typeof row.name === 'string' && row.name) node.name = row.name
  if (row.absolute === true) node.absolute = true
  const wMode = oneOf(row.wMode, SIZES)
  const hMode = oneOf(row.hMode, SIZES)
  if (wMode) node.wMode = wMode
  if (hMode) node.hMode = hMode
  const fill = parsePaint(row.fill)
  const stroke = parsePaint(row.stroke)
  if (fill) node.fill = fill
  if (stroke) node.stroke = stroke
  const radius = parseRadius(row.radius)
  if (typeof radius === 'number' ? radius > 0 : radius != null) node.radius = radius
  const opacity = num(row.opacity)
  if (opacity != null && opacity >= 0 && opacity < 1) node.opacity = opacity
  const strokeWidth = num(row.strokeWidth)
  if (strokeWidth != null && strokeWidth > 0 && strokeWidth !== 1) node.strokeWidth = strokeWidth
  const rotation = num(row.rotation)
  if (rotation != null && rotation !== 0) node.rotation = rotation
  if (row.flipX === true) node.flipX = true
  if (row.flipY === true) node.flipY = true
  if (row.visible === false) node.visible = false
  if (row.locked === true) node.locked = true
  if (kind === 'frame') {
    const layout = oneOf(row.layout, LAYOUTS)
    if (layout) node.layout = layout
    const gap = num(row.gap)
    const pad = num(row.pad)
    if (gap != null && gap > 0) node.gap = gap
    if (pad != null && pad > 0) node.pad = pad
    const align = oneOf(row.align, ALIGNS)
    const justify = oneOf(row.justify, JUSTIFIES)
    if (align && align !== 'start') node.align = align
    if (justify && justify !== 'start') node.justify = justify
    const padTop = num(row.padTop)
    const padRight = num(row.padRight)
    const padBottom = num(row.padBottom)
    const padLeft = num(row.padLeft)
    if (padTop != null && padTop >= 0) node.padTop = padTop
    if (padRight != null && padRight >= 0) node.padRight = padRight
    if (padBottom != null && padBottom >= 0) node.padBottom = padBottom
    if (padLeft != null && padLeft >= 0) node.padLeft = padLeft
    if (row.wrap === true) node.wrap = true
  }
  if ((kind === 'frame' || kind === 'instance') && row.clip === false) node.clip = false
  const minW = num(row.minW)
  const maxW = num(row.maxW)
  const minH = num(row.minH)
  const maxH = num(row.maxH)
  if (minW != null && minW > 0) node.minW = minW
  if (maxW != null && maxW > 0) node.maxW = maxW
  if (minH != null && minH > 0) node.minH = minH
  if (maxH != null && maxH > 0) node.maxH = maxH
  const radiusTL = parseRadius(row.radiusTL)
  const radiusTR = parseRadius(row.radiusTR)
  const radiusBR = parseRadius(row.radiusBR)
  const radiusBL = parseRadius(row.radiusBL)
  if (radiusTL != null) node.radiusTL = radiusTL
  if (radiusTR != null) node.radiusTR = radiusTR
  if (radiusBR != null) node.radiusBR = radiusBR
  if (radiusBL != null) node.radiusBL = radiusBL
  if (kind === 'vector') {
    const vector = parseVector(row.vector)
    if (!vector) return null
    node.vector = vector
  }
  if (isDesignContainer(kind) && Array.isArray(row.children) && row.children.length) {
    const children: DesignNode[] = []
    for (const child of row.children) {
      const parsed = parseNode(child)
      if (!parsed) return null
      children.push(parsed)
    }
    node.children = children
  }
  if ((kind === 'text' || kind === 'instance') && typeof row.text === 'string' && row.text) node.text = row.text
  if (kind === 'text') {
    const fontSize = num(row.fontSize)
    if (fontSize != null && fontSize > 0 && fontSize !== 13) node.fontSize = fontSize
    const weight = oneOf(row.weight, WEIGHTS)
    if (weight && weight !== 'regular') node.weight = weight
    const textAlign = oneOf(row.textAlign, TEXT_ALIGNS)
    if (textAlign && textAlign !== 'left') node.textAlign = textAlign
    const lineHeight = num(row.lineHeight)
    if (lineHeight != null && lineHeight > 0) node.lineHeight = lineHeight
    const letterSpacing = num(row.letterSpacing)
    if (letterSpacing != null && letterSpacing !== 0) node.letterSpacing = letterSpacing
    const color = parsePaint(row.color)
    if (color && color !== 'none') node.color = color
    if (typeof row.fontFamily === 'string' && FONT_FAMILY.test(row.fontFamily.trim())) node.fontFamily = row.fontFamily.trim()
    const textVertical = oneOf(row.textVertical, TEXT_VERTICAL)
    if (textVertical && textVertical !== 'center') node.textVertical = textVertical
    const textHug = oneOf(row.textHug, TEXT_HUGS)
    if (textHug) node.textHug = textHug
  }
  if (kind === 'instance') {
    if (typeof row.component !== 'string' || !row.component) return null
    node.component = row.component
    const variant = parseProps(row.variant)
    if (variant && Object.keys(variant).length) node.variant = variant
    if (row.overrides !== undefined) {
      const overrides = parseOverrides(row.overrides)
      if (!overrides) return null
      if (overrides.length) node.overrides = overrides
    }
  }
  return node
}

function parseOverrides(value: unknown): DesignOverride[] | null {
  if (!Array.isArray(value)) return null
  const rows: DesignOverride[] = []
  for (const item of value) {
    if (!item || typeof item !== 'object' || Array.isArray(item)) return null
    const row = item as Record<string, unknown>
    if (typeof row.id !== 'string' || !row.id) return null
    const next: DesignOverride = { id: row.id }
    if (typeof row.text === 'string') next.text = row.text
    const fill = parsePaint(row.fill)
    if (fill) next.fill = fill
    if (row.visible === false || row.visible === true) next.visible = row.visible
    if (next.text == null && next.fill == null && next.visible == null) continue
    rows.push(next)
  }
  return rows
}

function parseTokenValue(kind: DesignTokenKind, value: unknown): string | undefined {
  if (typeof value !== 'string' || !value) return undefined
  if (kind === 'color') return parseColor(value)
  if (kind === 'space' || kind === 'radius') return /^\d+(\.\d+)?$/.test(value) ? value : undefined
  return /^\d+(\.\d+)?\/(regular|medium|bold)$/.test(value) ? value : undefined
}

function parseToken(value: unknown): DesignToken | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const row = value as Record<string, unknown>
  const name = parseTokenName(row.name)
  const kind = oneOf(row.kind, TOKEN_KINDS)
  if (!name || !kind || !row.values || typeof row.values !== 'object' || Array.isArray(row.values)) return null
  const values: Record<string, string> = {}
  for (const [mode, item] of Object.entries(row.values)) {
    if (!mode) return null
    const parsed = parseTokenValue(kind, item)
    if (!parsed) return null
    values[mode] = parsed
  }
  if (!Object.keys(values).length) return null
  return { name, kind, values }
}

function parseVariant(value: unknown): DesignVariant | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const row = value as Record<string, unknown>
  const node = parseNode(row.node)
  if (!node || node.kind !== 'frame') return null
  const props = parseProps(row.props) ?? {}
  return { props, node }
}

function parseComponent(value: unknown): DesignComponent | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const row = value as Record<string, unknown>
  if (typeof row.id !== 'string' || !row.id || typeof row.name !== 'string' || !row.name) return null
  if (!Array.isArray(row.variants) || row.variants.length === 0) return null
  const variants: DesignVariant[] = []
  for (const item of row.variants) {
    const variant = parseVariant(item)
    if (!variant) return null
    variants.push(variant)
  }
  const component: DesignComponent = { id: row.id, name: row.name, variants }
  if (row.axes !== undefined) {
    const axes = parseAxes(row.axes)
    if (!axes) return null
    if (Object.keys(axes).length) component.axes = axes
  }
  return component
}

function parseModes(value: unknown): string[] | null {
  if (value === undefined) return ['light', 'dark']
  if (!Array.isArray(value) || value.length === 0) return null
  const modes: string[] = []
  for (const item of value) {
    if (typeof item !== 'string' || !item || modes.includes(item)) return null
    modes.push(item)
  }
  return modes
}

export function parseDesign(text: string): { doc: DesignDoc; error: string | null } {
  let value: unknown
  try {
    value = JSON.parse(text)
  } catch {
    return { doc: emptyDesign(), error: 'This file is not a design' }
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return { doc: emptyDesign(), error: 'This file is not a design' }
  }
  const raw = value as Record<string, unknown>
  const version = raw.version === undefined ? 1 : raw.version
  if (version !== 1) return { doc: emptyDesign(), error: 'This file is not a design' }
  const modes = parseModes(raw.modes)
  if (!modes) return { doc: emptyDesign(), error: 'This file is not a design' }
  const mode = typeof raw.mode === 'string' && modes.includes(raw.mode) ? raw.mode : modes[0]
  const grid = num(raw.grid)
  const tokens: DesignToken[] = []
  if (raw.tokens !== undefined) {
    if (!Array.isArray(raw.tokens)) return { doc: emptyDesign(), error: 'This file is not a design' }
    for (const item of raw.tokens) {
      const token = parseToken(item)
      if (!token) return { doc: emptyDesign(), error: 'This file is not a design' }
      tokens.push(token)
    }
  }
  const components: DesignComponent[] = []
  if (raw.components !== undefined) {
    if (!Array.isArray(raw.components)) return { doc: emptyDesign(), error: 'This file is not a design' }
    for (const item of raw.components) {
      const component = parseComponent(item)
      if (!component) return { doc: emptyDesign(), error: 'This file is not a design' }
      components.push(component)
    }
  }
  const screens: DesignNode[] = []
  if (raw.screens !== undefined) {
    if (!Array.isArray(raw.screens)) return { doc: emptyDesign(), error: 'This file is not a design' }
    for (const item of raw.screens) {
      const screen = parseNode(item)
      if (!screen) return { doc: emptyDesign(), error: 'This file is not a design' }
      screens.push(screen)
    }
  }
  if (raw.version === undefined && tokens.length === 0 && components.length === 0 && screens.length === 0) {
    return { doc: emptyDesign(), error: 'This file is not a design' }
  }
  return {
    doc: {
      version: 1,
      modes,
      mode,
      snap: raw.snap === true,
      grid: grid != null && grid > 0 ? grid : DESIGN_GRID,
      tokens,
      components,
      screens,
    },
    error: null,
  }
}

function writeNode(node: DesignNode): DesignNode {
  const row: DesignNode = { id: node.id, kind: node.kind, x: node.x, y: node.y, w: node.w, h: node.h }
  if (node.name) row.name = node.name
  if (node.wMode) row.wMode = node.wMode
  if (node.hMode) row.hMode = node.hMode
  if (node.absolute) row.absolute = true
  if (node.layout) row.layout = node.layout
  if (node.gap) row.gap = node.gap
  if (node.pad) row.pad = node.pad
  if (node.padTop != null) row.padTop = node.padTop
  if (node.padRight != null) row.padRight = node.padRight
  if (node.padBottom != null) row.padBottom = node.padBottom
  if (node.padLeft != null) row.padLeft = node.padLeft
  if (node.wrap) row.wrap = true
  if (node.minW != null) row.minW = node.minW
  if (node.maxW != null) row.maxW = node.maxW
  if (node.minH != null) row.minH = node.minH
  if (node.maxH != null) row.maxH = node.maxH
  if (node.clip === false) row.clip = false
  if (node.align && node.align !== 'start') row.align = node.align
  if (node.justify && node.justify !== 'start') row.justify = node.justify
  if (node.fill) row.fill = node.fill
  if (node.stroke) row.stroke = node.stroke
  if (node.strokeWidth != null && node.strokeWidth !== 1) row.strokeWidth = node.strokeWidth
  if (typeof node.radius === 'number' ? node.radius > 0 : node.radius) row.radius = node.radius
  if (node.radiusTL != null) row.radiusTL = node.radiusTL
  if (node.radiusTR != null) row.radiusTR = node.radiusTR
  if (node.radiusBR != null) row.radiusBR = node.radiusBR
  if (node.radiusBL != null) row.radiusBL = node.radiusBL
  if (node.opacity != null && node.opacity < 1) row.opacity = node.opacity
  if (node.text) row.text = node.text
  if (node.fontSize != null && node.fontSize > 0 && node.fontSize !== 13) row.fontSize = node.fontSize
  if (node.fontFamily) row.fontFamily = node.fontFamily
  if (node.weight && node.weight !== 'regular') row.weight = node.weight
  if (node.textAlign && node.textAlign !== 'left') row.textAlign = node.textAlign
  if (node.textVertical && node.textVertical !== 'center') row.textVertical = node.textVertical
  if (node.textHug) row.textHug = node.textHug
  if (node.lineHeight != null && node.lineHeight > 0) row.lineHeight = node.lineHeight
  if (node.letterSpacing) row.letterSpacing = node.letterSpacing
  if (node.color && node.color !== 'none') row.color = node.color
  if (node.rotation) row.rotation = node.rotation
  if (node.flipX) row.flipX = true
  if (node.flipY) row.flipY = true
  if (node.visible === false) row.visible = false
  if (node.locked) row.locked = true
  if (node.kind === 'vector' && node.vector) row.vector = cloneVector(node.vector)
  if (node.component) row.component = node.component
  if (node.variant && Object.keys(node.variant).length) row.variant = { ...node.variant }
  if (node.overrides?.length) {
    row.overrides = node.overrides.map((item) => {
      const copy: DesignOverride = { id: item.id }
      if (item.text != null) copy.text = item.text
      if (item.fill) copy.fill = item.fill
      if (item.visible != null) copy.visible = item.visible
      return copy
    })
  }
  if (isDesignContainer(node.kind) && node.children?.length) row.children = node.children.map(writeNode)
  return row
}

function parsePoint(value: unknown): DesignVectorPoint | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const row = value as Record<string, unknown>
  const x = num(row.x)
  const y = num(row.y)
  if (x == null || y == null) return null
  return { x, y }
}

function parseVector(value: unknown): DesignVector | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const row = value as Record<string, unknown>
  if (!Array.isArray(row.vertices) || !Array.isArray(row.segments)) return null
  const vertices: DesignVectorPoint[] = []
  for (const item of row.vertices) {
    const point = parsePoint(item)
    if (!point) return null
    vertices.push(point)
  }
  const segments: DesignVectorSegment[] = []
  for (const item of row.segments) {
    if (!item || typeof item !== 'object' || Array.isArray(item)) return null
    const segment = item as Record<string, unknown>
    const start = num(segment.start)
    const end = num(segment.end)
    if (start == null || end == null || !Number.isInteger(start) || !Number.isInteger(end)) return null
    if (start < 0 || end < 0 || start >= vertices.length || end >= vertices.length) return null
    segments.push({
      start,
      end,
      tangentStart: parsePoint(segment.tangentStart) ?? { x: 0, y: 0 },
      tangentEnd: parsePoint(segment.tangentEnd) ?? { x: 0, y: 0 },
    })
  }
  const regions: DesignVectorRegion[] = []
  if (row.regions !== undefined) {
    if (!Array.isArray(row.regions)) return null
    for (const item of row.regions) {
      if (!item || typeof item !== 'object' || Array.isArray(item)) return null
      const region = item as Record<string, unknown>
      const winding = region.winding === 'evenodd' ? 'evenodd' : region.winding === 'nonzero' ? 'nonzero' : null
      if (!winding || !Array.isArray(region.loops)) return null
      const loops: number[][] = []
      for (const loop of region.loops) {
        if (!Array.isArray(loop) || !loop.length) return null
        const indexes: number[] = []
        for (const index of loop) {
          if (typeof index !== 'number' || !Number.isInteger(index) || index < 0 || index >= segments.length) return null
          indexes.push(index)
        }
        loops.push(indexes)
      }
      regions.push({ winding, loops })
    }
  }
  return { vertices, segments, regions }
}

export function serializeDesign(doc: DesignDoc): string {
  const row: Record<string, unknown> = { version: 1 }
  if (doc.modes.length !== 2 || doc.modes[0] !== 'light' || doc.modes[1] !== 'dark') row.modes = doc.modes
  if (doc.mode !== doc.modes[0]) row.mode = doc.mode
  if (doc.snap) row.snap = true
  if (doc.grid !== DESIGN_GRID) row.grid = doc.grid
  if (doc.tokens.length) {
    row.tokens = doc.tokens.map((token) => ({ name: token.name, kind: token.kind, values: { ...token.values } }))
  }
  if (doc.components.length) {
    row.components = doc.components.map((component) => {
      const item: Record<string, unknown> = {
        id: component.id,
        name: component.name,
        variants: component.variants.map((variant) => ({
          props: variant.props,
          node: writeNode(variant.node),
        })),
      }
      if (component.axes && Object.keys(component.axes).length) item.axes = component.axes
      return item
    })
  }
  if (doc.screens.length) row.screens = doc.screens.map(writeNode)
  return JSON.stringify(row)
}

export type DesignQuery =
  | { tokens: true }
  | { component: string; variant?: Record<string, string> }
  | { screen: string }

export type DesignTokenSlice = {
  name: string
  kind: DesignTokenKind
  values: Record<string, string>
}

export type DesignQuerySlice = {
  mode: string
  modes: string[]
  tokens: DesignTokenSlice[]
  component?: {
    id: string
    name: string
    axes: Record<string, string[]>
    variant: Record<string, string>
    tree: DesignNode
  }
  screen?: {
    id: string
    name: string
    tree: DesignNode
  }
  components?: { id: string; name: string; axes: Record<string, string[]>; variant: Record<string, string> }[]
}

function tokenRefs(node: DesignNode, into: Set<string>) {
  for (const field of [node.fill, node.stroke, node.color]) {
    if (field && field !== 'none' && !field.startsWith('#')) into.add(field)
  }
  if (typeof node.radius === 'string') into.add(node.radius)
  for (const corner of [node.radiusTL, node.radiusTR, node.radiusBR, node.radiusBL]) {
    if (typeof corner === 'string') into.add(corner)
  }
  for (const row of node.overrides ?? []) {
    if (row.fill && row.fill !== 'none' && !row.fill.startsWith('#')) into.add(row.fill)
  }
  for (const child of node.children ?? []) tokenRefs(child, into)
}

function tokenSlices(doc: DesignDoc, names: Set<string>): DesignTokenSlice[] {
  return doc.tokens
    .filter((token) => names.has(token.name))
    .map((token) => ({ name: token.name, kind: token.kind, values: { ...token.values } }))
}

function sliceBase(doc: DesignDoc, names: Set<string>): DesignQuerySlice {
  return { mode: doc.mode, modes: [...doc.modes], tokens: tokenSlices(doc, names) }
}

/**
 * One compact slice for the model. A token query has no screens.
 * A component query is that variant's tree. A screen query keeps instances
 * as instances and names the components they use. It never returns the file.
 */
export function queryDesign(doc: DesignDoc, query: DesignQuery): DesignQuerySlice | null {
  if ('tokens' in query) return sliceBase(doc, new Set(doc.tokens.map((token) => token.name)))
  if ('component' in query) {
    const component = doc.components.find((item) => item.id === query.component)
    if (!component) return null
    const variant = pickVariant(component, query.variant)
    if (!variant) return null
    const names = new Set<string>()
    tokenRefs(variant.node, names)
    return {
      ...sliceBase(doc, names),
      component: {
        id: component.id,
        name: component.name,
        axes: component.axes ? { ...component.axes } : {},
        variant: { ...variant.props },
        tree: writeNode(variant.node),
      },
    }
  }
  const screen = doc.screens.find((item) => item.id === query.screen || item.name === query.screen)
  if (!screen) return null
  const names = new Set<string>()
  tokenRefs(screen, names)
  const used: DesignQuerySlice['components'] = []
  const seen = new Set<string>()
  const visit = (node: DesignNode) => {
    if (node.kind === 'instance' && node.component && !seen.has(`${node.component}:${variantKey(node.variant)}`)) {
      seen.add(`${node.component}:${variantKey(node.variant)}`)
      const component = doc.components.find((item) => item.id === node.component)
      if (component) {
        const variant = pickVariant(component, node.variant)
        if (variant) tokenRefs(variant.node, names)
        used.push({
          id: component.id,
          name: component.name,
          axes: component.axes ? { ...component.axes } : {},
          variant: { ...(variant?.props ?? node.variant ?? {}) },
        })
      }
    }
    for (const child of node.children ?? []) visit(child)
  }
  visit(screen)
  return {
    ...sliceBase(doc, names),
    screen: { id: screen.id, name: screen.name ?? 'Screen', tree: writeNode(screen) },
    components: used,
  }
}

/** One fenced slice for chat. The fence is the model payload. */
export function designChatText(doc: DesignDoc, query: DesignQuery): string | null {
  const slice = queryDesign(doc, query)
  if (!slice) return null
  return '```kdsgn\n' + JSON.stringify(slice) + '\n```'
}

/** The screen or component tree a chat query names. */
export function designQueryNode(doc: DesignDoc, query: DesignQuery): DesignNode | null {
  if ('tokens' in query) return null
  if ('component' in query) {
    const component = doc.components.find((item) => item.id === query.component)
    if (!component) return null
    return pickVariant(component, query.variant)?.node ?? null
  }
  return doc.screens.find((item) => item.id === query.screen || item.name === query.screen) ?? null
}

/** Parent-relative box for each node, so a picture can be read against positions. */
export function designCoordinateText(doc: DesignDoc, node: DesignNode): string {
  const lines: string[] = []
  const walk = (item: DesignNode, depth: number) => {
    if (lines.length >= 80) return
    const detail = [
      item.kind === 'text' && item.text ? JSON.stringify(item.text) : '',
      item.kind === 'instance' ? doc.components.find((component) => component.id === item.component)?.name ?? item.component ?? '' : '',
      item.fill && item.fill !== 'none' ? item.fill : '',
      item.flipX ? 'flipX' : '',
      item.flipY ? 'flipY' : '',
    ].filter(Boolean).join(' ')
    lines.push(`${'  '.repeat(depth)}${designLayerName(item)} ${Math.round(item.w)}×${Math.round(item.h)} at (${Math.round(item.x)}, ${Math.round(item.y)})${detail ? ` ${detail}` : ''}`)
    for (const child of item.children ?? []) walk(child, depth + 1)
  }
  walk(node, 0)
  return lines.join('\n')
}

/** Coordinate list plus the fenced slice. Prefer `designChatText` for chat chips. */
export function designChatNote(doc: DesignDoc, query: DesignQuery): string | null {
  const fence = designChatText(doc, query)
  const node = designQueryNode(doc, query)
  if (!fence || !node) return null
  return `Attached image is the render. Coordinates are parent-relative pixels.\n${designCoordinateText(doc, node)}\n\n${fence}`
}

export function designChatTitle(doc: DesignDoc, query: DesignQuery): string {
  if ('component' in query) return doc.components.find((item) => item.id === query.component)?.name ?? 'Component'
  const node = designQueryNode(doc, query)
  return node ? designLayerName(node) : 'Design'
}

const KDSGN_FENCE = /```[ \t]*kdsgn[ \t]*\r?\n[\s\S]*?```/gi

/** Title shown on a design chip. The fence body is one query slice. */
export function designFenceTitle(fence: string): string {
  const body = fence.replace(/^```[ \t]*kdsgn[ \t]*\r?\n/, '').replace(/```\s*$/, '')
  try {
    const value = JSON.parse(body) as { screen?: { name?: string }; component?: { name?: string } }
    const title = value.screen?.name || value.component?.name
    return title || 'Design'
  } catch {
    return 'Design'
  }
}

/** Drop legacy coordinate dumps; chips + PNG carry design context now. */
function stripLegacyDesignCoordinateProse(prose: string): string {
  const rest = prose.split('\n').filter((line) => {
    if (/^Attached image is the render\./.test(line.trim())) return false
    if (/\d+[×x]\d+ at \(/i.test(line)) return false
    return true
  })
  return rest.join('\n').replace(/\n{3,}/g, '\n\n').trim()
}

/** Pull fenced design slices out of a user message. The fence is what the model read. */
export function splitDesignMessage(content: string): { prose: string; designs: { text: string; title: string }[] } {
  const designs: { text: string; title: string }[] = []
  const prose = content.replace(new RegExp(KDSGN_FENCE.source, 'gi'), (fence) => {
    designs.push({ text: fence, title: designFenceTitle(fence) })
    return ''
  })
  const trimmed = prose.replace(/\n{3,}/g, '\n\n').trim()
  return { prose: stripLegacyDesignCoordinateProse(trimmed), designs }
}
