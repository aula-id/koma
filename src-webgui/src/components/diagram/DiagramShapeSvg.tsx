import { shapeDecor, shapePath, type ShapeKind } from '../../lib/diagramShapes'

/** A shape's outline (and inner strokes) in a w×h box at the origin. Shared by canvas, sketch and palette. */
export function DiagramShapeSvg({
  kind,
  w,
  h,
  fill,
  stroke,
  strokeWidth = 1,
}: {
  kind: ShapeKind
  w: number
  h: number
  fill: string
  stroke: string
  strokeWidth?: number
}) {
  const width = stroke === 'none' ? 0 : strokeWidth
  // Rectangles keep their small 4px rounding from before the other kinds existed.
  if (kind === 'rect' || kind === 'text') {
    return <rect width={w} height={h} rx={kind === 'rect' ? 4 : 0} fill={fill} stroke={stroke} strokeWidth={width} />
  }
  const decor = shapeDecor(kind, w, h)
  return (
    <>
      <path d={shapePath(kind, w, h)} fill={fill} stroke={stroke} strokeWidth={width} strokeLinejoin="round" />
      {decor && stroke !== 'none' ? <path d={decor} fill="none" stroke={stroke} strokeWidth={width} /> : null}
    </>
  )
}
