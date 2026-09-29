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
