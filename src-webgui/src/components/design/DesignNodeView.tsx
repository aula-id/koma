import { memo, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from 'react'
import {
  booleanPaintNode,
  booleanResult,
  contoursSvgPath,
  cornerPixels,
  isBooleanNode,
  nodeChrome,
  designImageSize,
  firstVisiblePaint,
  imageAssetUrl,
  imageCropRect,
  imageFillPlacement,
  isImageCropPaint,
  isPaintVisible,
  nodeFillLayers,
  svgFillLayers,
  type SvgFillLayer,
  nodeHasImageFill,
  nodePaints,
  roundedRectPath,
  paintAlias,
  siblingMaskStyle,
  layoutGridBands,
  splitTextByRuns,
  nodeStrokeCss,
  resolveInstanceTree,
  resolvePaintCss,
  resolveRef,
  strokePaintDash,
  strokePaintWidth,
  textStyle,
  vectorSvgPath,
  type DesignDoc,
  type DesignNode,
  type DesignPaint,
  type DesignWeight,
} from '../../lib/design'
import {
  SHAPE_FILL,
  TEXT_COLOR_DEFAULT,
  paintCss,
  weightCss,
} from './tabShared'
import { DesignTextEditor } from './DesignTextEditor'

export type DesignNodeHandlers = {
  /** `exact` selects this id as is, skipping the hit test at the pointer. */
  onSelect: (id: string, event: ReactPointerEvent<HTMLDivElement>, exact?: boolean) => void
  onEnterContainer: (id: string, event: ReactMouseEvent<HTMLDivElement>) => void
  onEdit: (id: string) => void
  onOpenFill: (id: string) => void
  onText: (id: string, text: string) => void
  onTextRange: (id: string, start: number, end: number) => void
  onTextBlur: () => void
  onMenu: (id: string, clientX: number, clientY: number) => void
}

/** Handlers that never change identity and call whatever the ref holds now, so memoized nodes keep their props. */
export function stableNodeHandlers(ref: { current: DesignNodeHandlers | null }): DesignNodeHandlers {
  return {
    onSelect: (id, event, exact) => ref.current?.onSelect(id, event, exact),
    onEnterContainer: (id, event) => ref.current?.onEnterContainer(id, event),
    onEdit: (id) => ref.current?.onEdit(id),
    onOpenFill: (id) => ref.current?.onOpenFill(id),
    onText: (id, text) => ref.current?.onText(id, text),
    onTextRange: (id, start, end) => ref.current?.onTextRange(id, start, end),
    onTextBlur: () => ref.current?.onTextBlur(),
    onMenu: (id, clientX, clientY) => ref.current?.onMenu(id, clientX, clientY),
  }
}

/** Re-renders only when its node or paint context changes. Unchanged siblings keep their object identity. */
export const DesignNodeView = memo(function DesignNodeView({
  doc,
  node,
  editing,
  dragCursor,
  locked = false,
  enteredContainerId = null,
  overrideTargetId = null,
  cropEditId = null,
  onSelect,
  onEdit,
  onOpenFill,
  onText,
  onTextRange,
  onTextBlur,
  onMenu,
  onEnterContainer,
  clipPath,
  maskStyle,
}: {
  doc: DesignDoc
  node: DesignNode
  editing: string | null
  dragCursor: string | null
  locked?: boolean
  enteredContainerId?: string | null
  overrideTargetId?: string | null
  cropEditId?: string | null
  onSelect: (id: string, event: ReactPointerEvent<HTMLDivElement>) => void
  onEnterContainer?: (id: string, event: ReactMouseEvent<HTMLDivElement>) => void
  onEdit: (id: string) => void
  onOpenFill?: (id: string) => void
  onText: (id: string, text: string) => void
  onTextRange?: (id: string, start: number, end: number) => void
  onTextBlur: () => void
  onMenu: (id: string, clientX: number, clientY: number) => void
  clipPath?: string
  maskStyle?: { clipPath?: string; maskImage?: string; WebkitMaskImage?: string; maskSize?: string; WebkitMaskSize?: string; maskPosition?: string; WebkitMaskPosition?: string; maskRepeat?: string; WebkitMaskRepeat?: string; maskMode?: string }
}) {
  if (node.visible === false) return null
  const booleanShape = isBooleanNode(node)
  const booleanGeometry = booleanShape ? booleanResult(doc, node) : null
  const visual = node.kind === 'instance' ? resolveInstanceTree(doc, node) : null
  const boxW = visual?.w ?? node.w
  const boxH = visual?.h ?? node.h
  const painted = booleanShape ? booleanPaintNode(node) : (visual ?? node)
  const chrome = nodeChrome(painted)
  const style = textStyle(visual && node.kind !== 'instance' ? visual : node)
  const overrideMark = overrideTargetId === node.id
  const bareFill = booleanShape ? chrome.fill === 'none' : node.fill === 'none' || chrome.fill === 'none'
  const bareStroke = (node.kind === 'frame' || node.kind === 'group') && (!node.stroke || node.stroke === 'none')
  const strokeOff = chrome.stroke === 'none' || ((node.kind === 'rect' || node.kind === 'ellipse') && node.stroke == null)
  const fillFallback = node.kind === 'frame' ? '#ffffff' : node.kind === 'rect' || node.kind === 'ellipse' || node.kind === 'vector' || booleanShape ? SHAPE_FILL : 'var(--color-koma-panel)'
  const fillPaints = nodePaints(painted, 'fill').filter((paint) => isPaintVisible(paint) && paintAlias(paint) !== 'none')
  const fillPaint = firstVisiblePaint(fillPaints)
  const fillFallbackCss = paintCss(doc, chrome.fill, fillFallback)
  const stroke = nodeStrokeCss(doc, painted, paintCss(doc, chrome.stroke, node.kind === 'line' || node.kind === 'vector' ? '#1c1c1c' : 'var(--color-koma-border)'))
  // Shapes drawn as paths take SVG fills. A shape with no fill paint of its own uses the fallback color.
  const pathFills: SvgFillLayer[] =
    chrome.fill === 'none'
      ? []
      : (() => {
          const layers = svgFillLayers(doc, painted, fillFallbackCss, `${node.id}-fill`, boxW, boxH)
          return layers.length ? layers : [{ fill: fillFallbackCss }]
        })()
  const fillLayers = bareFill || node.kind === 'line' || node.kind === 'vector' ? [] : nodeFillLayers(doc, painted, fillFallbackCss)
  const fillPlacements = fillPaints.map((paint) => imageFillPlacement({ w: boxW, h: boxH }, paint, paint.hash ? designImageSize(doc, paint.hash) : null))
  const fillSizes = fillPlacements.map((item) => item.size)
  const fillPositions = fillPlacements.map((item) => item.position)
  const fillRepeats = fillPaints.map((paint) => paint.type === 'image' && paint.scale === 'tile' ? 'repeat' : 'no-repeat')
  const cropEdit = cropEditId === node.id
  const cropPaint = cropEdit ? fillPaints.find(isImageCropPaint) : null
  const cropRect = cropPaint ? imageCropRect({ w: boxW, h: boxH }, cropPaint, cropPaint.hash ? designImageSize(doc, cropPaint.hash) : null) : null
  const cropUrl = cropPaint ? imageAssetUrl(doc, cropPaint.hash) : ''
  const corners = cornerPixels(doc, visual ?? node)
  const radius = node.kind === 'ellipse' ? '50%' : `${corners.tl}px ${corners.tr}px ${corners.br}px ${corners.bl}px`
  const clipValue = node.kind === 'instance' ? node.clip ?? visual?.clip : node.clip
  const children = visual?.children ?? (node.kind === 'instance' ? undefined : node.children)
  const cropDescendant = !!cropEditId && (node.id === cropEditId || !!(children && nodeContainsId(children, cropEditId)))
  const clips = (node.kind === 'frame' || node.kind === 'instance') && clipValue !== false && !cropDescendant
  const rotation = node.rotation ?? 0
  const flipX = node.flipX ? -1 : 1
  const flipY = node.flipY ? -1 : 1
  const transform = rotation || node.flipX || node.flipY ? `rotate(${rotation}deg) scale(${flipX}, ${flipY})` : undefined
  const opaqueUntilEnter = booleanShape || node.kind === 'group' || node.kind === 'instance'
  const enteredHere = enteredContainerId === node.id
  const lockChildren = locked || !!node.locked || (opaqueUntilEnter && !enteredHere)
  const hitHere = !locked
  const textColor = paintCss(doc, style.color, TEXT_COLOR_DEFAULT)
  const textTypeStyle = {
    fontSize: style.fontSize,
    fontFamily: style.fontFamily || undefined,
    fontWeight: weightCss(style.weight),
    fontStyle: node.italic ? 'italic' : undefined,
    textDecoration: [node.underline ? 'underline' : '', node.strike ? 'line-through' : ''].filter(Boolean).join(' ') || undefined,
    textTransform: node.textCase === 'upper' ? 'uppercase' : node.textCase === 'lower' ? 'lowercase' : node.textCase === 'title' ? 'capitalize' : undefined,
    lineHeight: style.lineHeight ? `${style.lineHeight}px` : undefined,
    letterSpacing: style.letterSpacing ? `${style.letterSpacing}px` : undefined,
    color: textColor,
  } as const
  return (
    <div
      className={`absolute select-none ${hitHere ? 'pointer-events-auto' : 'pointer-events-none'}`}
      style={{
        left: node.x,
        top: node.y,
        width: boxW,
        height: boxH,
        opacity: chrome.opacity,
        transform,
        clipPath: maskStyle?.clipPath ?? clipPath,
        maskImage: maskStyle?.maskImage,
        WebkitMaskImage: maskStyle?.WebkitMaskImage,
        maskSize: maskStyle?.maskSize,
        WebkitMaskSize: maskStyle?.WebkitMaskSize,
        maskPosition: maskStyle?.maskPosition,
        WebkitMaskPosition: maskStyle?.WebkitMaskPosition,
        maskRepeat: maskStyle?.maskRepeat,
        WebkitMaskRepeat: maskStyle?.WebkitMaskRepeat,
        maskMode: maskStyle?.maskMode,
        // Shapes keep the arrow, like Penpot; the grab hand is for panning.
        cursor: cropEdit && hitHere && !node.locked && !dragCursor ? 'move' : undefined,
      }}
      onPointerDown={hitHere ? (event) => onSelect(node.id, event) : undefined}
      onContextMenu={hitHere ? (event) => {
        event.preventDefault()
        event.stopPropagation()
        onMenu(node.id, event.clientX, event.clientY)
      } : undefined}
      onDoubleClick={(event) => {
        if (!hitHere || node.locked) return
        if (node.kind === 'text') {
          event.stopPropagation()
          onEdit(node.id)
          return
        }
        if (booleanShape && onEnterContainer) {
          event.stopPropagation()
          onEnterContainer(node.id, event)
          return
        }
        if (nodeHasImageFill(painted) && onOpenFill) {
          event.stopPropagation()
          onOpenFill(node.id)
          return
        }
        if ((node.kind === 'group' || node.kind === 'frame' || node.kind === 'instance' || node.kind === 'vector') && onEnterContainer) {
          event.stopPropagation()
          onEnterContainer(node.id, event)
        }
      }}
    >
      {cropEdit && cropRect && cropUrl ? (
        <span
          aria-hidden
          className="absolute"
          style={{
            left: cropRect.x,
            top: cropRect.y,
            width: cropRect.w,
            height: cropRect.h,
            backgroundImage: `url("${cropUrl}")`,
            backgroundSize: '100% 100%',
            backgroundRepeat: 'no-repeat',
            opacity: 0.35,
            cursor: 'move',
          }}
        />
      ) : null}
      <div
        className={`absolute inset-0 ${overrideMark ? 'ring-2 ring-inset ring-[#9747ff]' : ''}`}
        style={{
          border: undefined,
          borderRadius: radius,
          overflow: clips ? 'hidden' : undefined,
          ...effectStyle(doc, painted.effects, booleanShape || node.kind === 'text' || node.kind === 'line' || node.kind === 'vector'),
          mixBlendMode: painted.blend && painted.blend !== 'normal' && painted.blend !== 'pass-through' ? painted.blend : undefined,
        }}
      >
        {!booleanShape && node.kind !== 'line' && node.kind !== 'vector' && node.kind !== 'text' ? (
          <span
            aria-hidden
            className="pointer-events-none absolute inset-0"
            style={{
              backgroundImage: fillLayers.length ? fillLayers.join(', ') : undefined,
              backgroundSize: fillLayers.length ? fillSizes.join(', ') : undefined,
              backgroundRepeat: fillLayers.length ? fillRepeats.join(', ') : undefined,
              backgroundPosition: fillLayers.length ? fillPositions.join(', ') : undefined,
              opacity: fillPaints.length === 1 ? fillPaint?.opacity ?? 1 : 1,
              borderRadius: 'inherit',
            }}
          />
        ) : null}
        {!booleanShape && node.kind !== 'text' && node.kind !== 'vector' && !bareStroke && !strokeOff ? (
          <svg className="pointer-events-none absolute overflow-visible" width={boxW} height={boxH}>
            {nodePaints(painted, 'stroke').filter((paint) => isPaintVisible(paint) && paintAlias(paint) !== 'none').map((paint, index) => (
              <StrokeShape key={`${paint.type}-${index}`} doc={doc} node={painted} paint={paint} fallback={stroke} boxW={boxW} boxH={boxH} />
            ))}
          </svg>
        ) : node.kind === 'line' ? (
          <svg className="absolute inset-0 overflow-visible" width={boxW} height={boxH}>
            {nodePaints(painted, 'stroke').filter((paint) => isPaintVisible(paint) && paintAlias(paint) !== 'none').map((paint, index) => (
              <StrokeShape key={`${paint.type}-${index}`} doc={doc} node={painted} paint={paint} fallback={stroke} boxW={boxW} boxH={boxH} />
            ))}
          </svg>
        ) : null}
        {booleanGeometry && booleanGeometry.contours.length ? (
          <svg className="pointer-events-none absolute inset-0 overflow-visible" width={boxW} height={boxH}>
            <PathFills layers={pathFills} d={contoursSvgPath(booleanGeometry.contours)} rule={booleanGeometry.rule} />
            <path d={contoursSvgPath(booleanGeometry.contours)} fillRule={booleanGeometry.rule} fill="none" stroke={chrome.stroke === 'none' ? 'none' : stroke} strokeWidth={chrome.strokeWidth} strokeLinecap={painted.strokeCap === 'round' || painted.strokeCap === 'square' ? painted.strokeCap : 'butt'} strokeLinejoin={painted.strokeJoin ?? 'miter'} strokeDasharray={painted.strokeDash?.join(' ')} />
            {nodePaints(painted, 'stroke').slice(1).filter((paint) => isPaintVisible(paint) && paintAlias(paint) !== 'none').map((paint, index) => (
              <path key={`stroke-${index}`} d={contoursSvgPath(booleanGeometry.contours)} fill="none" fillRule={booleanGeometry.rule} stroke={resolvePaintCss(doc, paint, stroke)} strokeWidth={strokePaintWidth(paint, chrome.strokeWidth)} strokeDasharray={strokePaintDash(paint, painted.strokeDash)} />
            ))}
          </svg>
        ) : null}
        {!booleanShape && node.kind === 'vector' && node.vector ? (
          <svg className="absolute inset-0 overflow-visible" width={boxW} height={boxH}>
            <PathFills layers={pathFills} d={vectorSvgPath(node.vector)} />
            <path d={vectorSvgPath(node.vector)} fill="none" stroke={chrome.stroke === 'none' ? 'none' : stroke} strokeWidth={chrome.strokeWidth} strokeLinecap={painted.strokeCap === 'round' || painted.strokeCap === 'square' ? painted.strokeCap : 'butt'} strokeLinejoin={painted.strokeJoin ?? 'miter'} strokeDasharray={painted.strokeDash?.join(' ')} {...(node.svgAttrs ?? {})} />
            {nodePaints(painted, 'stroke').slice(1).filter((paint) => isPaintVisible(paint) && paintAlias(paint) !== 'none').map((paint, index) => (
              <path key={`stroke-${index}`} d={vectorSvgPath(node.vector)} fill="none" stroke={resolvePaintCss(doc, paint, stroke)} strokeWidth={strokePaintWidth(paint, chrome.strokeWidth)} strokeDasharray={strokePaintDash(paint, painted.strokeDash)} />
            ))}
          </svg>
        ) : null}
        {node.kind === 'text' ? (
          <div
            className="flex h-full w-full"
            style={{
              ...textTypeStyle,
              justifyContent: style.align === 'center' ? 'center' : style.align === 'right' ? 'flex-end' : 'flex-start',
              textAlign: style.align,
              alignItems: style.vertical === 'top' ? 'flex-start' : style.vertical === 'bottom' ? 'flex-end' : 'center',
            }}
          >
            {editing === node.id ? (
              <DesignTextEditor
                doc={doc}
                text={node.text ?? ''}
                runs={node.runs}
                style={{ flex: style.align === 'justify' ? 1 : undefined }}
                wrap={node.textHug !== 'width'}
                onText={(text) => onText(node.id, text)}
                onTextRange={onTextRange ? (start, end) => onTextRange(node.id, start, end) : undefined}
                onDone={onTextBlur}
              />
            ) : (
              <span className={node.textHug === 'width' ? 'whitespace-pre' : 'whitespace-pre-wrap'} style={{ flex: style.align === 'justify' ? 1 : undefined }}>
                {splitTextByRuns(node.text || 'Text', node.runs).map((part, index) => (
                  <span
                    key={`${part.text}-${index}`}
                    style={{
                      fontSize: part.run?.fontSize,
                      fontWeight: part.run?.weight ? weightCss(part.run.weight) : undefined,
                      fontStyle: part.run?.italic ? 'italic' : undefined,
                      textDecoration: [part.run?.underline ? 'underline' : '', part.run?.strike ? 'line-through' : ''].filter(Boolean).join(' ') || undefined,
                      fontFamily: part.run?.fontFamily || undefined,
                      color: part.run?.color ? paintCss(doc, part.run.color, 'inherit') : undefined,
                    }}
                  >
                    {part.text}
                  </span>
                ))}
              </span>
            )}
          </div>
        ) : node.kind === 'instance' && !visual ? (
          <span className="pointer-events-none absolute left-2 top-1 truncate text-[11px] text-koma-dim">Missing component</span>
        ) : null}
        {node.kind === 'frame' && node.layoutGrids?.length ? (
          <svg className="pointer-events-none absolute inset-0 overflow-hidden" width={boxW} height={boxH}>
            {node.layoutGrids.map((grid, index) => {
              const color = grid.color || 'rgba(255,0,0,0.18)'
              if (grid.kind === 'square') {
                // One pattern tile instead of a line per cell, so a fine grid on a big frame stays cheap.
                const size = Math.max(1, grid.size ?? 8)
                const offset = grid.offset ?? 0
                const pattern = `grid-${node.id.replace(/[^\w-]/g, '_')}-${index}`
                return [
                  <defs key={`d${index}`}>
                    <pattern id={pattern} width={size} height={size} x={offset} y={offset} patternUnits="userSpaceOnUse">
                      <path d={`M ${size} 0 L 0 0 0 ${size}`} fill="none" stroke={color} strokeWidth={1} />
                    </pattern>
                  </defs>,
                  <rect key={`r${index}`} width={boxW} height={boxH} fill={`url(#${pattern})`} />,
                ]
              }
              return layoutGridBands(grid, boxW, boxH).map((band, at) => (
                <rect key={`${grid.kind}${index}-${at}`} x={band.x} y={band.y} width={band.w} height={band.h} fill={color} />
              ))
            })}
          </svg>
        ) : null}
        {(!booleanShape || enteredHere) && children?.map((child, index) => {
          const mask = [...(children ?? [])].slice(0, index).reverse().find((item) => item.mask)
          return (
          <DesignNodeView
            key={child.id}
            doc={doc}
            node={child}
            editing={editing}
            dragCursor={dragCursor}
            locked={lockChildren}
            enteredContainerId={enteredContainerId}
            overrideTargetId={overrideTargetId}
            cropEditId={cropEditId}
            maskStyle={mask && !child.mask ? siblingMaskStyle(mask, child, doc) : undefined}
            onEnterContainer={onEnterContainer}
            onSelect={onSelect}
            onEdit={onEdit}
            onOpenFill={onOpenFill}
            onText={onText}
            onTextRange={onTextRange}
            onTextBlur={onTextBlur}
            onMenu={onMenu}
          />
          )
        })}
      </div>
    </div>
  )
})

/**
 * Effects as CSS. A box shadow follows the box, so text, lines and vectors take drop-shadow
 * filters that follow their glyphs or path. Several blurs of one kind all apply.
 */
function effectStyle(doc: DesignDoc, effects: DesignNode['effects'], followsShape: boolean): { boxShadow?: string; filter?: string; backdropFilter?: string } {
  const on = (effects ?? []).filter((item) => item.visible !== false)
  const color = (item: { color?: string }) => paintCss(doc, item.color && item.color !== 'none' ? item.color : '', 'rgba(0,0,0,0.25)')
  const shadows = on.filter((item) => item.kind === 'drop-shadow' || (item.kind === 'inner-shadow' && !followsShape))
  const blurs = on.filter((item) => item.kind === 'layer-blur').map((item) => `blur(${item.blur ?? 4}px)`)
  const drops = followsShape ? shadows.map((item) => `drop-shadow(${item.x ?? 0}px ${item.y ?? 4}px ${item.blur ?? 8}px ${color(item)})`) : []
  const boxShadow = followsShape ? '' : shadows.map((item) => `${item.kind === 'inner-shadow' ? 'inset ' : ''}${item.x ?? 0}px ${item.y ?? 4}px ${item.blur ?? 8}px ${item.spread ?? 0}px ${color(item)}`).join(', ')
  const backdrop = on.filter((item) => item.kind === 'background-blur').map((item) => `blur(${item.blur ?? 8}px)`).join(' ')
  return {
    boxShadow: boxShadow || undefined,
    filter: [...drops, ...blurs].join(' ') || undefined,
    backdropFilter: backdrop || undefined,
  }
}

/** The fill layers of a path shape (and the gradients they point at), drawn under its stroke. */
function PathFills({ layers, d, rule }: { layers: SvgFillLayer[]; d: string; rule?: 'nonzero' | 'evenodd' }) {
  const gradients = layers.flatMap((layer) => (layer.gradient ? [layer.gradient] : []))
  return (
    <>
      {gradients.length ? (
        <defs>
          {gradients.map((gradient) => {
            const stops = gradient.stops.map((stop, index) => <stop key={index} offset={stop.offset} stopColor={stop.color} />)
            return gradient.kind === 'linear' ? (
              <linearGradient key={gradient.id} id={gradient.id} gradientUnits="userSpaceOnUse" x1={gradient.x1} y1={gradient.y1} x2={gradient.x2} y2={gradient.y2}>
                {stops}
              </linearGradient>
            ) : (
              <radialGradient key={gradient.id} id={gradient.id} gradientUnits="userSpaceOnUse" cx={gradient.x1} cy={gradient.y1} r={gradient.x2}>
                {stops}
              </radialGradient>
            )
          })}
        </defs>
      ) : null}
      {layers.map((layer, index) => (
        <path key={index} d={d} fill={layer.fill} fillOpacity={layer.opacity} fillRule={rule} stroke="none" />
      ))}
    </>
  )
}

function StrokeShape({
  doc,
  node,
  paint,
  fallback,
  boxW,
  boxH,
}: {
  doc: DesignDoc
  node: DesignNode
  paint: DesignPaint
  fallback: string
  boxW: number
  boxH: number
}) {
  const color = resolvePaintCss(doc, paint, fallback)
  const width = strokePaintWidth(paint, node.strokeWidth ?? (node.kind === 'line' ? 2 : 1))
  const align = paint.align ?? node.strokeAlign ?? 'center'
  const dash = strokePaintDash(paint, node.strokeDash)
  const cap = paint.capStart ?? paint.capEnd ?? node.strokeStart ?? node.strokeCap
  const linecap = cap === 'round' || cap === 'square' ? cap : 'butt'
  const join = paint.join ?? node.strokeJoin ?? 'miter'
  const inset = align === 'inside' ? width / 2 : align === 'outside' ? -width / 2 : 0
  const top = paint.top ?? node.strokeTop ?? width
  const right = paint.right ?? node.strokeRight ?? width
  const bottom = paint.bottom ?? node.strokeBottom ?? width
  const left = paint.left ?? node.strokeLeft ?? width
  const sided = paint.top != null || paint.right != null || paint.bottom != null || paint.left != null || node.strokeTop != null
  const markerStart = paint.markerStart ?? node.strokeMarkerStart
  const markerEnd = paint.markerEnd ?? node.strokeMarkerEnd
  if (node.kind === 'line') {
    return (
      <g>
        <line x1={0} y1={boxH / 2} x2={boxW} y2={boxH / 2} stroke={color} strokeWidth={width} strokeLinecap={linecap} strokeDasharray={dash} />
        {markerStart === 'arrow' ? <polygon points={`0,${boxH / 2} 8,${boxH / 2 - 4} 8,${boxH / 2 + 4}`} fill={color} /> : null}
        {markerEnd === 'arrow' ? <polygon points={`${boxW},${boxH / 2} ${boxW - 8},${boxH / 2 - 4} ${boxW - 8},${boxH / 2 + 4}`} fill={color} /> : null}
        {markerStart === 'dot' ? <circle cx={0} cy={boxH / 2} r={3} fill={color} /> : null}
        {markerEnd === 'dot' ? <circle cx={boxW} cy={boxH / 2} r={3} fill={color} /> : null}
      </g>
    )
  }
  if (node.kind === 'ellipse') {
    return <ellipse cx={boxW / 2} cy={boxH / 2} rx={Math.max(0.5, boxW / 2 - inset)} ry={Math.max(0.5, boxH / 2 - inset)} fill="none" stroke={color} strokeWidth={width} strokeLinecap={linecap} strokeDasharray={dash} />
  }
  if (sided) {
    return (
      <g>
        {top > 0 ? <line x1={0} y1={0} x2={boxW} y2={0} stroke={color} strokeWidth={top} /> : null}
        {right > 0 ? <line x1={boxW} y1={0} x2={boxW} y2={boxH} stroke={color} strokeWidth={right} /> : null}
        {bottom > 0 ? <line x1={0} y1={boxH} x2={boxW} y2={boxH} stroke={color} strokeWidth={bottom} /> : null}
        {left > 0 ? <line x1={0} y1={0} x2={0} y2={boxH} stroke={color} strokeWidth={left} /> : null}
      </g>
    )
  }
  return (
    <g>
      <path d={roundedRectPath(boxW, boxH, cornerPixels(doc, node), inset)} fill="none" stroke={color} strokeWidth={width} strokeDasharray={dash} strokeLinecap={linecap} strokeLinejoin={join} />
      {markerEnd === 'arrow' ? <polygon points={`${boxW},${boxH / 2} ${boxW - 8},${boxH / 2 - 4} ${boxW - 8},${boxH / 2 + 4}`} fill={color} /> : null}
      {markerStart === 'dot' ? <circle cx={0} cy={boxH / 2} r={3} fill={color} /> : null}
      {markerEnd === 'dot' ? <circle cx={boxW} cy={boxH / 2} r={3} fill={color} /> : null}
    </g>
  )
}

function nodeContainsId(nodes: DesignNode[], id: string): boolean {
  return nodes.some((node) => node.id === id || !!(node.children && nodeContainsId(node.children, id)))
}
