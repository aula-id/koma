// Design document types and constants.
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
export const FONT_FAMILY = /^[\w][\w\s,-]{0,80}$/
const TOKEN_KINDS: readonly DesignTokenKind[] = ['color', 'space', 'type', 'radius']
export const TOKEN_NAME = /^[a-zA-Z][a-zA-Z0-9._-]*$/

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
