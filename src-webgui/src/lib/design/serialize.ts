import {
  DESIGN_GRID,
  COMPONENT_MIME,
  DESIGN_MIME,
  type DesignComponent,
  type DesignDoc,
  type DesignKind,
  type DesignNode,
  type DesignOverride,
  type DesignToken,
  type DesignTokenKind,
  type DesignVariant,
  type DesignVector,
  type DesignVectorPoint,
  type DesignVectorRegion,
  type DesignVectorSegment,
} from './types'
import { emptyDesign, isDesignContainer } from './model'

const KINDS = ['frame', 'group', 'rect', 'ellipse', 'line', 'vector', 'text', 'instance'] as const
const LAYOUTS = ['row', 'column'] as const
const ALIGNS = ['start', 'center', 'end', 'stretch'] as const
const JUSTIFIES = ['start', 'center', 'end', 'space'] as const
const SIZES = ['hug', 'fill', 'fixed'] as const
const WEIGHTS = ['regular', 'medium', 'bold'] as const
const TEXT_ALIGNS = ['left', 'center', 'right'] as const
const TEXT_VERTICAL = ['top', 'center', 'bottom'] as const
const TEXT_HUGS = ['height', 'width'] as const
const FONT_FAMILY = /^[\w][\w\s,-]{0,80}$/
const TOKEN_KINDS = ['color', 'space', 'type', 'radius'] as const
const TOKEN_NAME = /^[a-zA-Z][a-zA-Z0-9._-]*$/

import { cloneVector } from './model'

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

export function writeNode(node: DesignNode): DesignNode {
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
