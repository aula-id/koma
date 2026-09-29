// Design documents stored as `<workspace>/.koma/<name>.kdsgn`.
// A file is one design system: tokens, components, and screens.
// Child x/y are parent-relative. A screen's x/y are canvas coordinates.
// Pan and zoom are view state and are not stored here.

export const DESIGN_GRID = 8
export const DESIGN_MIME = 'application/x-koma-design'
export const DESIGN_MIN_W = 8
export const DESIGN_MIN_H = 8

export type DesignKind = 'frame' | 'rect' | 'text' | 'instance'
export type DesignLayout = 'row' | 'column'
export type DesignAlign = 'start' | 'center' | 'end'
export type DesignSize = 'hug' | 'fill'
export type DesignWeight = 'regular' | 'medium' | 'bold'
export type DesignTextAlign = 'left' | 'center' | 'right'
export type DesignTokenKind = 'color' | 'space' | 'type' | 'radius'

const KINDS: readonly DesignKind[] = ['frame', 'rect', 'text', 'instance']
const LAYOUTS: readonly DesignLayout[] = ['row', 'column']
const ALIGNS: readonly DesignAlign[] = ['start', 'center', 'end']
const SIZES: readonly DesignSize[] = ['hug', 'fill']
const WEIGHTS: readonly DesignWeight[] = ['regular', 'medium', 'bold']
const TEXT_ALIGNS: readonly DesignTextAlign[] = ['left', 'center', 'right']
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
  /** Cross-axis alignment. Omitted means start. */
  align?: DesignAlign
  /** Main-axis alignment. Omitted means start. */
  justify?: DesignAlign
  fill?: DesignRef
  stroke?: DesignRef
  strokeWidth?: number
  radius?: number | string
  opacity?: number
  text?: string
  fontSize?: number
  weight?: DesignWeight
  textAlign?: DesignTextAlign
  color?: DesignRef
  /** Instance target. Required when kind is instance. */
  component?: string
  variant?: Record<string, string>
  children?: DesignNode[]
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
    snap: true,
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

export function createNode(kind: 'frame' | 'rect' | 'text', id: string, x: number, y: number): DesignNode {
  if (kind === 'frame') return { id, kind, name: 'Frame', x, y, w: 360, h: 240 }
  if (kind === 'text') return { id, kind, x, y, w: 120, h: 24, text: 'Text' }
  return { id, kind, x, y, w: 160, h: 64 }
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

/** Paint the root and write the text onto the first text node. */
export function applyOverrides(root: DesignNode, override: { text?: string; fill?: string }): DesignNode {
  let textUsed = override.text == null
  const walk = (node: DesignNode, isRoot: boolean): DesignNode => {
    let next = node
    if (isRoot && override.fill) next = { ...next, fill: override.fill }
    if (!textUsed && node.kind === 'text') {
      textUsed = true
      next = { ...next, text: override.text }
    }
    if (next.children?.length) next = { ...next, children: next.children.map((child) => walk(child, false)) }
    return next
  }
  return walk(root, true)
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
  const shaped = node.kind === 'frame' || node.kind === 'rect'
  return {
    fill: node.fill ?? (shaped ? '' : 'none'),
    stroke: node.stroke ?? (shaped ? '' : 'none'),
    radius: node.radius ?? 0,
    opacity: node.opacity ?? 1,
    strokeWidth: node.strokeWidth ?? 1,
  }
}

export function textStyle(node: DesignNode): { text: string; fontSize: number; weight: DesignWeight; align: DesignTextAlign; color: string } {
  return {
    text: node.text ?? '',
    fontSize: node.fontSize && node.fontSize > 0 ? node.fontSize : 13,
    weight: node.weight ?? 'regular',
    align: node.textAlign ?? 'left',
    color: node.color ?? '',
  }
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
  if (node.align) next.align = node.align
  if (node.justify) next.justify = node.justify
  if (node.fill) next.fill = node.fill
  if (node.stroke) next.stroke = node.stroke
  if (node.strokeWidth != null) next.strokeWidth = node.strokeWidth
  if (node.radius != null) next.radius = node.radius
  if (node.opacity != null) next.opacity = node.opacity
  if (node.text != null) next.text = node.text
  if (node.fontSize != null) next.fontSize = node.fontSize
  if (node.weight) next.weight = node.weight
  if (node.textAlign) next.textAlign = node.textAlign
  if (node.color) next.color = node.color
  if (node.component) next.component = node.component
  if (node.variant) next.variant = { ...node.variant }
  if (node.children?.length) next.children = node.children.map((child) => cloneNode(child, mint))
  return next
}

export type DesignHandle = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w'

export function snapDesign(n: number, grid: number, enabled: boolean): number {
  if (!enabled || !Number.isFinite(grid) || grid <= 0) return n
  return Math.round(n / grid) * grid
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
  if (w < DESIGN_MIN_W) {
    if (handle.includes('w')) x = right - DESIGN_MIN_W
    w = DESIGN_MIN_W
  }
  if (h < DESIGN_MIN_H) {
    if (handle.includes('n')) y = bottom - DESIGN_MIN_H
    h = DESIGN_MIN_H
  }
  return { ...node, x, y, w, h }
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

/** Top-left of a node in canvas coordinates. */
export function nodeOrigin(doc: DesignDoc, id: string): { x: number; y: number } | null {
  for (const screen of doc.screens) {
    const found = originIn(screen, id, 0, 0)
    if (found) return found
  }
  return null
}

/** Topmost node under a canvas point. */
export function hitDesign(doc: DesignDoc, x: number, y: number): DesignNode | null {
  for (let i = doc.screens.length - 1; i >= 0; i--) {
    const found = hitIn(doc.screens[i], x, y)
    if (found) return found
  }
  return null
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
  if (parentId == null) {
    if (node.kind !== 'frame') return doc
    return { ...doc, screens: [...doc.screens, node] }
  }
  let changed = false
  const screens = doc.screens.map((screen) => {
    const next = addChild(screen, parentId, node)
    if (next !== screen) changed = true
    return next
  })
  return changed ? { ...doc, screens } : doc
}

/** Move a node under a new parent. A null parent promotes a frame to a screen. */
export function placeDesignNode(doc: DesignDoc, id: string, parentId: string | null, x: number, y: number): DesignDoc {
  const located = locateDesign(doc, id)
  if (!located) return doc
  if (parentId === located.parentId && located.node.x === x && located.node.y === y) return doc
  if (parentId === id) return doc
  if (parentId && containsNode(located.node, parentId)) return doc
  if (parentId == null && located.node.kind !== 'frame') return doc
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
  if (next.kind !== 'frame' || !next.layout) return next
  return placeFlow(next, frozenId)
}

function placeFlow(frame: DesignNode, frozenId?: string): DesignNode {
  const pad = frame.pad ?? 0
  const gap = frame.gap ?? 0
  const horizontal = frame.layout === 'row'
  const children = frame.children ?? []
  const flow = children.filter((child) => !child.absolute && child.id !== frozenId)
  const mainSize = (child: DesignNode) => (horizontal ? child.w : child.h)
  const crossSize = (child: DesignNode) => (horizontal ? child.h : child.w)
  const mainMode = (child: DesignNode) => (horizontal ? child.wMode : child.hMode)
  const crossMode = (child: DesignNode) => (horizontal ? child.hMode : child.wMode)
  const hugsMain = horizontal ? frame.wMode === 'hug' : frame.hMode === 'hug'
  const hugsCross = horizontal ? frame.hMode === 'hug' : frame.wMode === 'hug'
  const gaps = Math.max(0, flow.length - 1) * gap
  const contentMain = flow.reduce((sum, child) => sum + (mainMode(child) === 'fill' && !hugsMain ? 0 : mainSize(child)), 0) + gaps
  const contentCross = flow.reduce((max, child) => Math.max(max, crossSize(child)), 0)
  let width = frame.w
  let height = frame.h
  if (hugsMain) {
    const size = Math.max(horizontal ? DESIGN_MIN_W : DESIGN_MIN_H, pad * 2 + contentMain)
    if (horizontal) width = size
    else height = size
  }
  if (hugsCross) {
    const size = Math.max(horizontal ? DESIGN_MIN_H : DESIGN_MIN_W, pad * 2 + contentCross)
    if (horizontal) height = size
    else width = size
  }
  const innerMain = Math.max(0, (horizontal ? width : height) - pad * 2)
  const innerCross = Math.max(0, (horizontal ? height : width) - pad * 2)
  const fills = hugsMain ? [] : flow.filter((child) => mainMode(child) === 'fill')
  const usedFixed = flow.reduce((sum, child) => sum + (fills.includes(child) ? 0 : mainSize(child)), 0)
  const fillMain = fills.length ? Math.max(8, (innerMain - usedFixed - gaps) / fills.length) : 0
  const justify = frame.justify ?? 'start'
  const align = frame.align ?? 'start'
  const measured = flow.map((child) => {
    const main = fills.includes(child) ? fillMain : mainSize(child)
    const cross = crossMode(child) === 'fill' && !hugsCross ? innerCross : crossSize(child)
    return { child, main: Math.max(8, main), cross: Math.max(8, cross) }
  })
  const used = measured.reduce((sum, item) => sum + item.main, 0) + gaps
  let cursor = pad
  if (justify === 'center') cursor = pad + Math.max(0, innerMain - used) / 2
  if (justify === 'end') cursor = pad + Math.max(0, innerMain - used)
  const placed = new Map<string, DesignNode>()
  for (const item of measured) {
    const crossPos = align === 'center' ? pad + (innerCross - item.cross) / 2 : align === 'end' ? pad + innerCross - item.cross : pad
    const x = horizontal ? cursor : crossPos
    const y = horizontal ? crossPos : cursor
    const w = horizontal ? item.main : item.cross
    const h = horizontal ? item.cross : item.main
    placed.set(item.child.id, item.child.x === x && item.child.y === y && item.child.w === w && item.child.h === h ? item.child : { ...item.child, x, y, w, h })
    cursor += item.main + gap
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

function originIn(node: DesignNode, id: string, ax: number, ay: number): { x: number; y: number } | null {
  if (node.id === id) return { x: ax + node.x, y: ay + node.y }
  for (const child of node.children ?? []) {
    const found = originIn(child, id, ax + node.x, ay + node.y)
    if (found) return found
  }
  return null
}

function hitIn(node: DesignNode, x: number, y: number): DesignNode | null {
  if (x < node.x || y < node.y || x > node.x + node.w || y > node.y + node.h) return null
  const localX = x - node.x
  const localY = y - node.y
  const children = node.children ?? []
  for (let i = children.length - 1; i >= 0; i--) {
    const found = hitIn(children[i], localX, localY)
    if (found) return found
  }
  return node
}

function frameIn(node: DesignNode, x: number, y: number, ignoreId: string): string | null {
  if (node.id === ignoreId) return null
  if (x < node.x || y < node.y || x > node.x + node.w || y > node.y + node.h) return null
  const localX = x - node.x
  const localY = y - node.y
  if (node.kind === 'frame') {
    const children = node.children ?? []
    for (let i = children.length - 1; i >= 0; i--) {
      const found = frameIn(children[i], localX, localY, ignoreId)
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

function addChild(node: DesignNode, parentId: string, child: DesignNode): DesignNode {
  if (node.id === parentId && node.kind === 'frame') return { ...node, children: [...(node.children ?? []), child] }
  if (!node.children?.length) return node
  let changed = false
  const children = node.children.map((item) => {
    const next = addChild(item, parentId, child)
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
  if (kind === 'frame') {
    const layout = oneOf(row.layout, LAYOUTS)
    if (layout) node.layout = layout
    const gap = num(row.gap)
    const pad = num(row.pad)
    if (gap != null && gap > 0) node.gap = gap
    if (pad != null && pad > 0) node.pad = pad
    const align = oneOf(row.align, ALIGNS)
    const justify = oneOf(row.justify, ALIGNS)
    if (align && align !== 'start') node.align = align
    if (justify && justify !== 'start') node.justify = justify
    if (Array.isArray(row.children) && row.children.length) {
      const children: DesignNode[] = []
      for (const child of row.children) {
        const parsed = parseNode(child)
        if (!parsed) return null
        children.push(parsed)
      }
      node.children = children
    }
  }
  if ((kind === 'text' || kind === 'instance') && typeof row.text === 'string' && row.text) node.text = row.text
  if (kind === 'text') {
    const fontSize = num(row.fontSize)
    if (fontSize != null && fontSize > 0 && fontSize !== 13) node.fontSize = fontSize
    const weight = oneOf(row.weight, WEIGHTS)
    if (weight && weight !== 'regular') node.weight = weight
    const textAlign = oneOf(row.textAlign, TEXT_ALIGNS)
    if (textAlign && textAlign !== 'left') node.textAlign = textAlign
    const color = parsePaint(row.color)
    if (color && color !== 'none') node.color = color
  }
  if (kind === 'instance') {
    if (typeof row.component !== 'string' || !row.component) return null
    node.component = row.component
    const variant = parseProps(row.variant)
    if (variant && Object.keys(variant).length) node.variant = variant
  }
  return node
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
      if (!screen || screen.kind !== 'frame') return { doc: emptyDesign(), error: 'This file is not a design' }
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
      snap: typeof raw.snap === 'boolean' ? raw.snap : true,
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
  if (node.align && node.align !== 'start') row.align = node.align
  if (node.justify && node.justify !== 'start') row.justify = node.justify
  if (node.fill) row.fill = node.fill
  if (node.stroke) row.stroke = node.stroke
  if (node.strokeWidth != null && node.strokeWidth !== 1) row.strokeWidth = node.strokeWidth
  if (typeof node.radius === 'number' ? node.radius > 0 : node.radius) row.radius = node.radius
  if (node.opacity != null && node.opacity < 1) row.opacity = node.opacity
  if (node.text) row.text = node.text
  if (node.fontSize != null && node.fontSize > 0 && node.fontSize !== 13) row.fontSize = node.fontSize
  if (node.weight && node.weight !== 'regular') row.weight = node.weight
  if (node.textAlign && node.textAlign !== 'left') row.textAlign = node.textAlign
  if (node.color && node.color !== 'none') row.color = node.color
  if (node.component) row.component = node.component
  if (node.variant && Object.keys(node.variant).length) row.variant = { ...node.variant }
  if (node.kind === 'frame' && node.children?.length) row.children = node.children.map(writeNode)
  return row
}

export function serializeDesign(doc: DesignDoc): string {
  const row: Record<string, unknown> = { version: 1 }
  if (doc.modes.length !== 2 || doc.modes[0] !== 'light' || doc.modes[1] !== 'dark') row.modes = doc.modes
  if (doc.mode !== doc.modes[0]) row.mode = doc.mode
  if (!doc.snap) row.snap = false
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
