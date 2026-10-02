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
  type DesignPaint,
  type DesignImageAsset,
  type DesignEffect,
  type DesignTextRun,
  type DesignGuide,
  type DesignLayoutGrid,
  type DesignPageView,
} from './types'
import { emptyDesign, isDesignContainer } from './model'
import { clonePaint } from './paint'

const KINDS = ['frame', 'group', 'rect', 'ellipse', 'line', 'vector', 'text', 'instance'] as const
const LAYOUTS = ['row', 'column', 'grid'] as const
const BLENDS = ['normal', 'multiply', 'screen', 'overlay', 'darken', 'lighten', 'color-burn', 'color-dodge', 'soft-light', 'hard-light', 'difference', 'exclusion', 'hue', 'saturation', 'color', 'luminosity', 'pass-through'] as const
const GRADIENTS = ['linear', 'radial', 'angular', 'diamond'] as const
const IMAGE_SCALES = ['fill', 'fit', 'crop', 'tile'] as const
const STROKE_ALIGNS = ['inside', 'center', 'outside'] as const
const STROKE_CAPS = ['none', 'round', 'square'] as const
const STROKE_JOINS = ['miter', 'bevel', 'round'] as const
const CONSTRAINTS = ['start', 'center', 'end', 'stretch', 'scale'] as const
const EFFECT_KINDS = ['drop-shadow', 'inner-shadow', 'layer-blur', 'background-blur'] as const
const BOOLEAN_OPS = ['union', 'subtract', 'intersect', 'exclude'] as const
const TEXT_CASES = ['original', 'upper', 'lower', 'title'] as const
const TRUNCATES = ['off', 'end'] as const
const PAINT_TYPES = ['solid', 'gradient', 'image'] as const
const ALIGNS = ['start', 'center', 'end', 'stretch'] as const
const JUSTIFIES = ['start', 'center', 'end', 'space', 'around', 'evenly'] as const
const SIZES = ['hug', 'fill', 'fixed'] as const
const WEIGHTS = ['regular', 'medium', 'bold'] as const
const TEXT_ALIGNS = ['left', 'center', 'right', 'justify'] as const
const STROKE_MARKERS = ['none', 'arrow', 'dot'] as const
const GRID_KINDS = ['square', 'column', 'row'] as const
const GRID_ALIGNS = ['stretch', 'start', 'center', 'end'] as const
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

function parsePaintObject(value: unknown): DesignPaint | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const row = value as Record<string, unknown>
  const type = oneOf(row.type, PAINT_TYPES)
  if (!type) return null
  const paint: DesignPaint = { type }
  if (row.visible === false) paint.visible = false
  const opacity = num(row.opacity)
  if (opacity != null && opacity >= 0 && opacity < 1) paint.opacity = opacity
  const blend = oneOf(row.blend, BLENDS)
  if (blend && blend !== 'normal') paint.blend = blend
  if (type === 'solid') {
    const color = parsePaint(row.color)
    if (!color || color === 'none') return null
    paint.color = color
  } else if (type === 'gradient') {
    const kind = oneOf(row.kind, GRADIENTS) ?? 'linear'
    paint.kind = kind
    const stops: { color: string; at: number }[] = []
    if (Array.isArray(row.stops)) {
      for (const item of row.stops) {
        if (!item || typeof item !== 'object' || Array.isArray(item)) continue
        const stop = item as Record<string, unknown>
        const color = parsePaint(stop.color)
        const at = num(stop.at)
        if (!color || color === 'none' || at == null) continue
        stops.push({ color, at: Math.max(0, Math.min(1, at)) })
      }
    }
    if (stops.length < 2) {
      stops.push({ color: '#000000', at: 0 }, { color: '#ffffff', at: 1 })
    }
    paint.stops = stops
    if (Array.isArray(row.transform) && row.transform.every((item) => typeof item === 'number' && Number.isFinite(item))) {
      paint.transform = row.transform.slice()
    }
  } else {
    if (typeof row.hash !== 'string' || !row.hash) return null
    paint.hash = row.hash
    const scale = oneOf(row.scale, IMAGE_SCALES)
    if (scale && scale !== 'fill') paint.scale = scale
    const imageX = num(row.imageX)
    const imageY = num(row.imageY)
    const imageW = num(row.imageW)
    const imageH = num(row.imageH)
    if (imageX != null) paint.imageX = imageX
    if (imageY != null) paint.imageY = imageY
    if (imageW != null && imageW > 0) paint.imageW = imageW
    if (imageH != null && imageH > 0) paint.imageH = imageH
  }
  const width = num(row.width)
  if (width != null && width > 0 && width !== 1) paint.width = width
  const align = oneOf(row.align, STROKE_ALIGNS)
  if (align && align !== 'center') paint.align = align
  const dash = num(row.dash)
  const gap = num(row.gap)
  if (dash != null && dash > 0) paint.dash = dash
  if (gap != null && gap >= 0) paint.gap = gap
  const capStart = oneOf(row.capStart, STROKE_CAPS)
  const capEnd = oneOf(row.capEnd, STROKE_CAPS)
  if (capStart && capStart !== 'none') paint.capStart = capStart
  if (capEnd && capEnd !== 'none') paint.capEnd = capEnd
  const join = oneOf(row.join, STROKE_JOINS)
  if (join && join !== 'miter') paint.join = join
  const markerStart = oneOf(row.markerStart, STROKE_MARKERS)
  const markerEnd = oneOf(row.markerEnd, STROKE_MARKERS)
  if (markerStart && markerStart !== 'none') paint.markerStart = markerStart
  if (markerEnd && markerEnd !== 'none') paint.markerEnd = markerEnd
  const top = num(row.top)
  const right = num(row.right)
  const bottom = num(row.bottom)
  const left = num(row.left)
  if (top != null && top >= 0) paint.top = top
  if (right != null && right >= 0) paint.right = right
  if (bottom != null && bottom >= 0) paint.bottom = bottom
  if (left != null && left >= 0) paint.left = left
  return paint
}

function parsePaints(value: unknown): DesignPaint[] | undefined {
  if (!Array.isArray(value)) return undefined
  const paints: DesignPaint[] = []
  for (const item of value) {
    const paint = parsePaintObject(item)
    if (!paint) return undefined
    paints.push(paint)
  }
  return paints
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
  const fills = parsePaints(row.fills)
  const strokes = parsePaints(row.strokes)
  if (fills?.length) node.fills = fills
  if (strokes?.length) node.strokes = strokes
  const strokeAlign = oneOf(row.strokeAlign, STROKE_ALIGNS)
  if (strokeAlign && strokeAlign !== 'center') node.strokeAlign = strokeAlign
  const strokeCap = oneOf(row.strokeCap, STROKE_CAPS)
  if (strokeCap && strokeCap !== 'none') node.strokeCap = strokeCap
  const strokeJoin = oneOf(row.strokeJoin, STROKE_JOINS)
  if (strokeJoin && strokeJoin !== 'miter') node.strokeJoin = strokeJoin
  if (Array.isArray(row.strokeDash) && row.strokeDash.every((item) => typeof item === 'number' && Number.isFinite(item))) {
    node.strokeDash = row.strokeDash.slice()
  }
  const blend = oneOf(row.blend, BLENDS)
  if (blend && blend !== 'normal') node.blend = blend
  if (Array.isArray(row.effects)) {
    const effects: DesignEffect[] = []
    for (const item of row.effects) {
      if (!item || typeof item !== 'object' || Array.isArray(item)) continue
      const effect = item as Record<string, unknown>
      const kind = oneOf(effect.kind, EFFECT_KINDS)
      if (!kind) continue
      const next: DesignEffect = { kind }
      if (effect.visible === false) next.visible = false
      const x = num(effect.x)
      const y = num(effect.y)
      const blur = num(effect.blur)
      const spread = num(effect.spread)
      if (x) next.x = x
      if (y) next.y = y
      if (blur != null && blur > 0) next.blur = blur
      if (spread) next.spread = spread
      const color = parsePaint(effect.color)
      if (color && color !== 'none') next.color = color
      effects.push(next)
    }
    if (effects.length) node.effects = effects
  }
  if (row.mask === true) node.mask = true
  if (row.maskType === 'alpha' || row.maskType === 'vector' || row.maskType === 'luminance') node.maskType = row.maskType
  const constraintH = oneOf(row.constraintH, CONSTRAINTS)
  const constraintV = oneOf(row.constraintV, CONSTRAINTS)
  if (constraintH) node.constraintH = constraintH
  if (constraintV) node.constraintV = constraintV
  const pointCount = num(row.pointCount)
  if (pointCount != null && pointCount >= 3) node.pointCount = Math.round(pointCount)
  const innerRadius = num(row.innerRadius)
  if (innerRadius != null && innerRadius > 0) node.innerRadius = innerRadius
  const booleanOp = oneOf(row.booleanOp, BOOLEAN_OPS)
  if (booleanOp) node.booleanOp = booleanOp
  if (row.section === true) node.section = true
  const textCase = oneOf(row.textCase, TEXT_CASES)
  if (textCase && textCase !== 'original') node.textCase = textCase
  const truncate = oneOf(row.truncate, TRUNCATES)
  if (truncate && truncate !== 'off') node.truncate = truncate
  const maxLines = num(row.maxLines)
  if (maxLines != null && maxLines > 0) node.maxLines = Math.round(maxLines)
  if (row.italic === true) node.italic = true
  if (row.underline === true) node.underline = true
  if (row.strike === true) node.strike = true
  if (Array.isArray(row.runs)) {
    const runs: DesignTextRun[] = []
    for (const item of row.runs) {
      if (!item || typeof item !== 'object' || Array.isArray(item)) continue
      const run = item as Record<string, unknown>
      const start = num(run.start)
      const end = num(run.end)
      if (start == null || end == null || end <= start) continue
      const next: DesignTextRun = { start, end }
      const weight = oneOf(run.weight, WEIGHTS)
      if (weight) next.weight = weight
      if (run.italic === true) next.italic = true
      if (run.underline === true) next.underline = true
      if (run.strike === true) next.strike = true
      const fontSize = num(run.fontSize)
      if (fontSize != null && fontSize > 0) next.fontSize = fontSize
      const color = parsePaint(run.color)
      if (color && color !== 'none') next.color = color
      if (typeof run.fontFamily === 'string' && FONT_FAMILY.test(run.fontFamily.trim())) next.fontFamily = run.fontFamily.trim()
      runs.push(next)
    }
    if (runs.length) node.runs = runs
  }
  const colStart = num(row.colStart)
  const colSpan = num(row.colSpan)
  const rowStart = num(row.rowStart)
  const rowSpan = num(row.rowSpan)
  if (colStart != null && colStart > 0) node.colStart = Math.round(colStart)
  if (colSpan != null && colSpan > 1) node.colSpan = Math.round(colSpan)
  if (rowStart != null && rowStart > 0) node.rowStart = Math.round(rowStart)
  if (rowSpan != null && rowSpan > 1) node.rowSpan = Math.round(rowSpan)
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
  const alignSelf = oneOf(row.alignSelf, ALIGNS)
  if (alignSelf && alignSelf !== 'start') node.alignSelf = alignSelf
  const margin = num(row.margin)
  if (margin != null && margin !== 0) node.margin = margin
  const marginTop = num(row.marginTop)
  const marginRight = num(row.marginRight)
  const marginBottom = num(row.marginBottom)
  const marginLeft = num(row.marginLeft)
  if (marginTop != null) node.marginTop = marginTop
  if (marginRight != null) node.marginRight = marginRight
  if (marginBottom != null) node.marginBottom = marginBottom
  if (marginLeft != null) node.marginLeft = marginLeft
  const strokeTop = num(row.strokeTop)
  const strokeRight = num(row.strokeRight)
  const strokeBottom = num(row.strokeBottom)
  const strokeLeft = num(row.strokeLeft)
  if (strokeTop != null && strokeTop >= 0) node.strokeTop = strokeTop
  if (strokeRight != null && strokeRight >= 0) node.strokeRight = strokeRight
  if (strokeBottom != null && strokeBottom >= 0) node.strokeBottom = strokeBottom
  if (strokeLeft != null && strokeLeft >= 0) node.strokeLeft = strokeLeft
  const strokeStart = oneOf(row.strokeStart, STROKE_CAPS)
  const strokeEnd = oneOf(row.strokeEnd, STROKE_CAPS)
  if (strokeStart && strokeStart !== 'none') node.strokeStart = strokeStart
  if (strokeEnd && strokeEnd !== 'none') node.strokeEnd = strokeEnd
  const strokeMarkerStart = oneOf(row.strokeMarkerStart, STROKE_MARKERS)
  const strokeMarkerEnd = oneOf(row.strokeMarkerEnd, STROKE_MARKERS)
  if (strokeMarkerStart && strokeMarkerStart !== 'none') node.strokeMarkerStart = strokeMarkerStart
  if (strokeMarkerEnd && strokeMarkerEnd !== 'none') node.strokeMarkerEnd = strokeMarkerEnd
  if (row.proportion === true) node.proportion = true
  if (row.svgAttrs && typeof row.svgAttrs === 'object' && !Array.isArray(row.svgAttrs)) {
    const attrs: Record<string, string> = {}
    for (const [key, value] of Object.entries(row.svgAttrs as Record<string, unknown>)) {
      if (typeof value === 'string' && key) attrs[key] = value
    }
    if (Object.keys(attrs).length) node.svgAttrs = attrs
  }
  if (row.bindings && typeof row.bindings === 'object' && !Array.isArray(row.bindings)) {
    const bindings: Record<string, string> = {}
    for (const [key, value] of Object.entries(row.bindings as Record<string, unknown>)) {
      if (typeof value === 'string' && TOKEN_NAME.test(value)) bindings[key] = value
    }
    if (Object.keys(bindings).length) node.bindings = bindings
  }
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
    if (row.reverse === true) node.reverse = true
    const gapX = num(row.gapX)
    const gapY = num(row.gapY)
    if (gapX != null && gapX > 0) node.gapX = gapX
    if (gapY != null && gapY > 0) node.gapY = gapY
    const alignContent = oneOf(row.alignContent, JUSTIFIES) ?? oneOf(row.alignContent, ALIGNS)
    if (alignContent && alignContent !== 'start') node.alignContent = alignContent
    if (Array.isArray(row.layoutGrids)) {
      const grids: DesignLayoutGrid[] = []
      for (const item of row.layoutGrids) {
        if (!item || typeof item !== 'object' || Array.isArray(item)) continue
        const grid = item as Record<string, unknown>
        const kind = oneOf(grid.kind, GRID_KINDS)
        if (!kind) continue
        const next: DesignLayoutGrid = { kind }
        const size = num(grid.size)
        const gutter = num(grid.gutter)
        const count = num(grid.count)
        const offset = num(grid.offset)
        if (size != null && size > 0) next.size = size
        if (gutter != null && gutter >= 0) next.gutter = gutter
        if (count != null && count > 0) next.count = Math.round(count)
        if (offset != null) next.offset = offset
        if (typeof grid.color === 'string') next.color = grid.color
        const align = oneOf(grid.align, GRID_ALIGNS)
        if (align && align !== 'stretch') next.align = align
        grids.push(next)
      }
      if (grids.length) node.layoutGrids = grids
    }
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
    const fills = parsePaints(row.fills)
    if (fills?.length) next.fills = fills
    const stroke = parsePaint(row.stroke)
    if (stroke) next.stroke = stroke
    const strokes = parsePaints(row.strokes)
    if (strokes?.length) next.strokes = strokes
    const radius = parseRadius(row.radius)
    if (radius != null) next.radius = radius
    if (typeof row.component === 'string' && row.component) next.component = row.component
    if (row.visible === false || row.visible === true) next.visible = row.visible
    const opacity = num(row.opacity)
    if (opacity != null && opacity >= 0 && opacity <= 1) next.opacity = opacity
    const fontSize = num(row.fontSize)
    if (fontSize != null && fontSize > 0) next.fontSize = fontSize
    const weight = oneOf(row.weight, WEIGHTS)
    if (weight) next.weight = weight
    const color = parsePaint(row.color)
    if (color && color !== 'none') next.color = color
    const strokeWidth = num(row.strokeWidth)
    if (strokeWidth != null && strokeWidth >= 0) next.strokeWidth = strokeWidth
    const rotation = num(row.rotation)
    if (rotation != null) next.rotation = rotation
    if (next.text == null && next.fill == null && next.fills == null && next.stroke == null && next.strokes == null && next.radius == null && next.visible == null && next.component == null && next.opacity == null && next.fontSize == null && next.weight == null && next.color == null && next.strokeWidth == null && next.rotation == null) continue
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
  if (version !== 1 && version !== 2) return { doc: emptyDesign(), error: 'This file is not a design' }
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
  const images: Record<string, DesignImageAsset> = {}
  if (raw.images !== undefined) {
    if (!raw.images || typeof raw.images !== 'object' || Array.isArray(raw.images)) return { doc: emptyDesign(), error: 'This file is not a design' }
    for (const [hash, item] of Object.entries(raw.images)) {
      if (!hash || !item || typeof item !== 'object' || Array.isArray(item)) return { doc: emptyDesign(), error: 'This file is not a design' }
      const asset = item as Record<string, unknown>
      if (typeof asset.mime !== 'string' || !asset.mime || typeof asset.path !== 'string' || !asset.path) {
        return { doc: emptyDesign(), error: 'This file is not a design' }
      }
      const stored: DesignImageAsset = { mime: asset.mime, path: asset.path }
      const width = num(asset.w)
      const height = num(asset.h)
      if (width != null && width > 0) stored.w = width
      if (height != null && height > 0) stored.h = height
      images[hash] = stored
    }
  }
  const guides: DesignGuide[] = []
  if (raw.guides !== undefined) {
    if (!Array.isArray(raw.guides)) return { doc: emptyDesign(), error: 'This file is not a design' }
    for (const item of raw.guides) {
      if (!item || typeof item !== 'object' || Array.isArray(item)) continue
      const row = item as Record<string, unknown>
      const axis = row.axis === 'x' || row.axis === 'y' ? row.axis : null
      const at = num(row.at)
      if (!axis || at == null) continue
      guides.push({ axis, at })
    }
  }
  const libraries: string[] = []
  if (raw.libraries !== undefined) {
    if (!Array.isArray(raw.libraries)) return { doc: emptyDesign(), error: 'This file is not a design' }
    for (const item of raw.libraries) {
      if (typeof item === 'string' && item && !libraries.includes(item)) libraries.push(item)
    }
  }
  const doc: DesignDoc = {
    version: version === 2 ? 2 : 1,
    modes,
    mode,
    snap: raw.snap === true,
    grid: grid != null && grid > 0 ? grid : DESIGN_GRID,
    tokens,
    components,
    screens,
  }
  if (Object.keys(images).length) doc.images = images
  if (guides.length) doc.guides = guides
  if (libraries.length) doc.libraries = libraries
  if (typeof raw.activePage === 'string' && raw.activePage) doc.activePage = raw.activePage
  if (raw.pageViews && typeof raw.pageViews === 'object' && !Array.isArray(raw.pageViews)) {
    const pageViews: Record<string, DesignPageView> = {}
    for (const [id, value] of Object.entries(raw.pageViews as Record<string, unknown>)) {
      if (!value || typeof value !== 'object' || Array.isArray(value)) continue
      const view = value as Record<string, unknown>
      const panX = num(view.panX)
      const panY = num(view.panY)
      const zoom = num(view.zoom)
      if (panX == null || panY == null || zoom == null) continue
      pageViews[id] = { panX, panY, zoom }
    }
    if (Object.keys(pageViews).length) doc.pageViews = pageViews
  }
  return { doc, error: null }
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
  if (node.reverse) row.reverse = true
  if (node.gapX) row.gapX = node.gapX
  if (node.gapY) row.gapY = node.gapY
  if (node.alignContent && node.alignContent !== 'start') row.alignContent = node.alignContent
  if (node.alignSelf && node.alignSelf !== 'start') row.alignSelf = node.alignSelf
  if (node.margin) row.margin = node.margin
  if (node.marginTop != null) row.marginTop = node.marginTop
  if (node.marginRight != null) row.marginRight = node.marginRight
  if (node.marginBottom != null) row.marginBottom = node.marginBottom
  if (node.marginLeft != null) row.marginLeft = node.marginLeft
  if (node.layoutGrids?.length) row.layoutGrids = node.layoutGrids.map((item) => ({ ...item }))
  if (node.strokeTop != null) row.strokeTop = node.strokeTop
  if (node.strokeRight != null) row.strokeRight = node.strokeRight
  if (node.strokeBottom != null) row.strokeBottom = node.strokeBottom
  if (node.strokeLeft != null) row.strokeLeft = node.strokeLeft
  if (node.strokeStart && node.strokeStart !== 'none') row.strokeStart = node.strokeStart
  if (node.strokeEnd && node.strokeEnd !== 'none') row.strokeEnd = node.strokeEnd
  if (node.strokeMarkerStart && node.strokeMarkerStart !== 'none') row.strokeMarkerStart = node.strokeMarkerStart
  if (node.strokeMarkerEnd && node.strokeMarkerEnd !== 'none') row.strokeMarkerEnd = node.strokeMarkerEnd
  if (node.proportion) row.proportion = true
  if (node.svgAttrs && Object.keys(node.svgAttrs).length) row.svgAttrs = { ...node.svgAttrs }
  if (node.bindings && Object.keys(node.bindings).length) row.bindings = { ...node.bindings }
  if (node.minW != null) row.minW = node.minW
  if (node.maxW != null) row.maxW = node.maxW
  if (node.minH != null) row.minH = node.minH
  if (node.maxH != null) row.maxH = node.maxH
  if (node.clip === false) row.clip = false
  if (node.align && node.align !== 'start') row.align = node.align
  if (node.justify && node.justify !== 'start') row.justify = node.justify
  if (node.fill) row.fill = node.fill
  if (node.stroke) row.stroke = node.stroke
  if (node.fills?.length) row.fills = node.fills.map(clonePaint)
  if (node.strokes?.length) row.strokes = node.strokes.map(clonePaint)
  if (node.strokeWidth != null && node.strokeWidth !== 1) row.strokeWidth = node.strokeWidth
  if (node.strokeAlign && node.strokeAlign !== 'center') row.strokeAlign = node.strokeAlign
  if (node.strokeCap && node.strokeCap !== 'none') row.strokeCap = node.strokeCap
  if (node.strokeJoin && node.strokeJoin !== 'miter') row.strokeJoin = node.strokeJoin
  if (node.strokeDash?.length) row.strokeDash = node.strokeDash.slice()
  if (node.blend && node.blend !== 'normal') row.blend = node.blend
  if (node.effects?.length) row.effects = node.effects.map((item) => ({ ...item }))
  if (node.mask) row.mask = true
  if (node.maskType) row.maskType = node.maskType
  if (node.constraintH) row.constraintH = node.constraintH
  if (node.constraintV) row.constraintV = node.constraintV
  if (node.gridColumns?.length) row.gridColumns = node.gridColumns.map((item) => ({ ...item }))
  if (node.gridRows?.length) row.gridRows = node.gridRows.map((item) => ({ ...item }))
  if (node.colStart != null) row.colStart = node.colStart
  if (node.colSpan != null) row.colSpan = node.colSpan
  if (node.rowStart != null) row.rowStart = node.rowStart
  if (node.rowSpan != null) row.rowSpan = node.rowSpan
  if (node.pointCount != null) row.pointCount = node.pointCount
  if (node.innerRadius != null) row.innerRadius = node.innerRadius
  if (node.booleanOp) row.booleanOp = node.booleanOp
  if (node.section) row.section = true
  if (node.runs?.length) row.runs = node.runs.map((item) => ({ ...item }))
  if (node.textCase && node.textCase !== 'original') row.textCase = node.textCase
  if (node.truncate && node.truncate !== 'off') row.truncate = node.truncate
  if (node.maxLines != null) row.maxLines = node.maxLines
  if (node.italic) row.italic = true
  if (node.underline) row.underline = true
  if (node.strike) row.strike = true
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
      if (item.fills?.length) copy.fills = item.fills.map(clonePaint)
      if (item.stroke) copy.stroke = item.stroke
      if (item.strokes?.length) copy.strokes = item.strokes.map(clonePaint)
      if (item.radius != null) copy.radius = item.radius
      if (item.component) copy.component = item.component
      if (item.visible != null) copy.visible = item.visible
      if (item.opacity != null) copy.opacity = item.opacity
      if (item.fontSize != null) copy.fontSize = item.fontSize
      if (item.weight) copy.weight = item.weight
      if (item.color) copy.color = item.color
      if (item.strokeWidth != null) copy.strokeWidth = item.strokeWidth
      if (item.rotation != null) copy.rotation = item.rotation
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

function designNeedsV2(doc: DesignDoc): boolean {
  if (doc.version === 2) return true
  if (doc.images && Object.keys(doc.images).length) return true
  if (doc.guides?.length || doc.libraries?.length) return true
  const walk = (node: DesignNode): boolean => {
    if (node.fills?.length || node.strokes?.length || node.effects?.length || node.runs?.length) return true
    if (node.blend || node.mask || node.constraintH || node.constraintV || node.booleanOp || node.section) return true
    if (node.reverse || node.gapX || node.gapY || node.alignSelf || node.layoutGrids?.length) return true
    if (node.layout === 'grid' || node.pointCount != null) return true
    return (node.children ?? []).some(walk)
  }
  return doc.screens.some(walk) || doc.components.some((component) => component.variants.some((variant) => walk(variant.node)))
}

export function serializeDesign(doc: DesignDoc): string {
  const row: Record<string, unknown> = { version: designNeedsV2(doc) ? 2 : 1 }
  if (doc.modes.length !== 2 || doc.modes[0] !== 'light' || doc.modes[1] !== 'dark') row.modes = doc.modes
  if (doc.mode !== doc.modes[0]) row.mode = doc.mode
  if (doc.snap) row.snap = true
  if (doc.grid !== DESIGN_GRID) row.grid = doc.grid
  if (doc.images && Object.keys(doc.images).length) row.images = { ...doc.images }
  if (doc.guides?.length) row.guides = doc.guides.map((guide) => ({ ...guide }))
  if (doc.libraries?.length) row.libraries = doc.libraries.slice()
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

export function serializeDesignSlice(
  nodes: DesignNode[],
  extras?: { components?: DesignDoc['components']; images?: DesignDoc['images'] },
): string {
  const doc = emptyDesign()
  doc.version = 2
  doc.screens = nodes
  if (extras?.components?.length) doc.components = extras.components
  if (extras?.images && Object.keys(extras.images).length) doc.images = extras.images
  return serializeDesign(doc)
}

export function parseDesignSlice(text: string): { nodes: DesignNode[]; components: DesignDoc['components']; images: DesignDoc['images'] } | null {
  const parsed = parseDesign(text)
  if (parsed.error || !parsed.doc.screens.length) return null
  return { nodes: parsed.doc.screens, components: parsed.doc.components, images: parsed.doc.images }
}
