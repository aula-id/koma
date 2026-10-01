// Design document types and constants.
export const DESIGN_GRID = 8
export const DESIGN_MIME = 'application/x-koma-design'
export const COMPONENT_MIME = 'application/x-koma-component'
export const DESIGN_MIN_W = 8
export const DESIGN_MIN_H = 8

export type DesignKind = 'frame' | 'group' | 'rect' | 'ellipse' | 'line' | 'vector' | 'text' | 'instance'
export type DesignLayout = 'row' | 'column' | 'grid'
export type DesignAlign = 'start' | 'center' | 'end' | 'space' | 'stretch' | 'around' | 'evenly'
export type DesignSize = 'hug' | 'fill' | 'fixed'
export type DesignWeight = 'regular' | 'medium' | 'bold'
export type DesignTextAlign = 'left' | 'center' | 'right' | 'justify'
export type DesignTextVertical = 'top' | 'center' | 'bottom'
/** `height` hugs the block height. `width` hugs both axes. */
export type DesignTextHug = 'height' | 'width'
export type DesignTokenKind = 'color' | 'space' | 'type' | 'radius'
export type DesignOrder = 'front' | 'forward' | 'backward' | 'back'
export type DesignDrawKind = 'frame' | 'group' | 'rect' | 'ellipse' | 'line' | 'vector' | 'text' | 'polygon' | 'star'
export type DesignBlend = 'normal' | 'multiply' | 'screen' | 'overlay' | 'darken' | 'lighten' | 'color-burn' | 'color-dodge' | 'soft-light' | 'hard-light' | 'difference' | 'exclusion' | 'hue' | 'saturation' | 'color' | 'luminosity' | 'pass-through'
export type DesignGradientKind = 'linear' | 'radial' | 'angular' | 'diamond'
export type DesignImageScale = 'fill' | 'fit' | 'crop' | 'tile'
export type DesignStrokeAlign = 'inside' | 'center' | 'outside'
export type DesignStrokeCap = 'none' | 'round' | 'square'
export type DesignStrokeJoin = 'miter' | 'bevel' | 'round'
export type DesignConstraint = 'start' | 'center' | 'end' | 'stretch' | 'scale'
export type DesignTextCase = 'original' | 'upper' | 'lower' | 'title'
export type DesignTextTruncate = 'off' | 'end'
export type DesignBooleanOp = 'union' | 'subtract' | 'intersect' | 'exclude'
export type DesignEffectKind = 'drop-shadow' | 'inner-shadow' | 'layer-blur' | 'background-blur'
export type DesignStrokeMarker = 'none' | 'arrow' | 'dot'
export type DesignInteractionTrigger = 'click' | 'mouse-enter' | 'mouse-leave' | 'after-delay'
export type DesignInteractionAction = 'navigate' | 'open-overlay' | 'toggle-overlay' | 'close-overlay' | 'prev-screen' | 'open-url'
export type DesignOverlayPlace = 'manual' | 'center' | 'top-left' | 'top' | 'top-right' | 'left' | 'right' | 'bottom-left' | 'bottom' | 'bottom-right'
export type DesignInteraction = {
  trigger: DesignInteractionTrigger
  action: DesignInteractionAction
  target?: string
  delay?: number
  url?: string
  overlayPlace?: DesignOverlayPlace
  overlayX?: number
  overlayY?: number
}
export type DesignLayoutGrid = { kind: 'square' | 'column' | 'row'; size?: number; color?: string; gutter?: number; count?: number; offset?: number }
export type DesignPageView = { panX: number; panY: number; zoom: number }
export type DesignBindingMap = Record<string, string>

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
const LAYOUTS: readonly DesignLayout[] = ['row', 'column', 'grid']
const ALIGNS: readonly DesignAlign[] = ['start', 'center', 'end', 'stretch']
const JUSTIFIES: readonly DesignAlign[] = ['start', 'center', 'end', 'space', 'around', 'evenly']
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

export type DesignPaintStop = { color: DesignRef; at: number }

export type DesignPaint = {
  type: 'solid' | 'gradient' | 'image'
  visible?: boolean
  opacity?: number
  blend?: DesignBlend
  color?: DesignRef
  kind?: DesignGradientKind
  stops?: DesignPaintStop[]
  transform?: number[]
  hash?: string
  scale?: DesignImageScale
  width?: number
  align?: DesignStrokeAlign
  dash?: number
  gap?: number
  capStart?: DesignStrokeCap
  capEnd?: DesignStrokeCap
  join?: DesignStrokeJoin
  markerStart?: DesignStrokeMarker
  markerEnd?: DesignStrokeMarker
  top?: number
  right?: number
  bottom?: number
  left?: number
}

export type DesignImageAsset = {
  mime: string
  path: string
}

export type DesignEffect = {
  kind: DesignEffectKind
  visible?: boolean
  x?: number
  y?: number
  blur?: number
  spread?: number
  color?: DesignRef
}

export type DesignTextRun = {
  start: number
  end: number
  weight?: DesignWeight
  italic?: boolean
  underline?: boolean
  strike?: boolean
  fontSize?: number
  color?: DesignRef
  fontFamily?: string
}

export type DesignGridTrack = { size: number | 'fr' | 'auto'; count?: number }

export type DesignGuide = { axis: 'x' | 'y'; at: number }

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
  gapX?: number
  gapY?: number
  pad?: number
  reverse?: boolean
  /** Cross-axis alignment. `stretch` sizes non-hug, non-fixed children to the inner cross size. Omitted means start. */
  align?: DesignAlign
  /** Main-axis alignment. Omitted means start. */
  justify?: DesignAlign
  alignContent?: DesignAlign
  alignSelf?: DesignAlign
  margin?: number
  marginTop?: number
  marginRight?: number
  marginBottom?: number
  marginLeft?: number
  layoutGrids?: DesignLayoutGrid[]
  interactions?: DesignInteraction[]
  svgAttrs?: Record<string, string>
  bindings?: DesignBindingMap
  proportion?: boolean
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
  fills?: DesignPaint[]
  strokes?: DesignPaint[]
  strokeWidth?: number
  strokeAlign?: DesignStrokeAlign
  strokeCap?: DesignStrokeCap
  strokeJoin?: DesignStrokeJoin
  strokeDash?: number[]
  strokeTop?: number
  strokeRight?: number
  strokeBottom?: number
  strokeLeft?: number
  strokeStart?: DesignStrokeCap
  strokeEnd?: DesignStrokeCap
  strokeMarkerStart?: DesignStrokeMarker
  strokeMarkerEnd?: DesignStrokeMarker
  blend?: DesignBlend
  effects?: DesignEffect[]
  mask?: boolean
  maskType?: 'alpha' | 'vector' | 'luminance'
  constraintH?: DesignConstraint
  constraintV?: DesignConstraint
  gridColumns?: DesignGridTrack[]
  gridRows?: DesignGridTrack[]
  colStart?: number
  colSpan?: number
  rowStart?: number
  rowSpan?: number
  pointCount?: number
  innerRadius?: number
  booleanOp?: DesignBooleanOp
  section?: boolean
  runs?: DesignTextRun[]
  textCase?: DesignTextCase
  truncate?: DesignTextTruncate
  maxLines?: number
  italic?: boolean
  underline?: boolean
  strike?: boolean
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
  fills?: DesignPaint[]
  stroke?: DesignRef
  strokes?: DesignPaint[]
  radius?: number | string
  visible?: boolean
  component?: string
  opacity?: number
  fontSize?: number
  weight?: DesignWeight
  color?: DesignRef
  strokeWidth?: number
  rotation?: number
}

export type DesignVariant = {
  props: Record<string, string>
  node: DesignNode
}

export type DesignComponentProp = {
  name: string
  kind: 'boolean' | 'text' | 'swap'
  nodeId?: string
}

export type DesignComponent = {
  id: string
  name: string
  axes?: Record<string, string[]>
  variants: DesignVariant[]
  props?: DesignComponentProp[]
}

export type DesignToken = {
  name: string
  kind: DesignTokenKind
  values: Record<string, string>
}

export type DesignDoc = {
  version: 1 | 2
  modes: string[]
  mode: string
  snap: boolean
  grid: number
  tokens: DesignToken[]
  components: DesignComponent[]
  screens: DesignNode[]
  images?: Record<string, DesignImageAsset>
  guides?: DesignGuide[]
  libraries?: string[]
  activePage?: string
  pageViews?: Record<string, DesignPageView>
}
