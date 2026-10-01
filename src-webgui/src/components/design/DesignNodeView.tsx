import type { PointerEvent as ReactPointerEvent } from 'react'
import {
  cornerPixels,
  designLayerName,
  nodeChrome,
  firstVisiblePaint,
  isPaintVisible,
  nodeFillCss,
  nodeFillLayers,
  nodePaints,
  paintAlias,
  siblingMaskStyle,
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
  type DesignHandle,
  type DesignNode,
  type DesignPaint,
  type DesignWeight,
} from '../../lib/design'
import { HANDLES, SELECTION, SHAPE_FILL, paintCss, weightCss, type RadiusCorner } from './tabShared'

export function DesignNodeView({
  doc,
  node,
  zoom,
  selectedIds,
  geometryId = null,
  editing,
  dragCursor,
  locked = false,
  enteredContainerId = null,
  overrideTargetId = null,
  onSelect,
  onResize,
  onRotate,
  onCorner,
  onEdit,
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
  zoom: number
  selectedIds: string[]
  geometryId?: string | null
  editing: string | null
  dragCursor: string | null
  locked?: boolean
  enteredContainerId?: string | null
  overrideTargetId?: string | null
  onSelect: (id: string, event: ReactPointerEvent<HTMLDivElement>) => void
  onEnterContainer?: (id: string, event: ReactPointerEvent<HTMLDivElement>) => void
  onResize: (id: string, handle: DesignHandle, event: ReactPointerEvent<HTMLButtonElement>) => void
  onRotate?: (id: string, event: ReactPointerEvent<HTMLButtonElement>) => void
  onCorner: (id: string, corner: RadiusCorner, event: ReactPointerEvent<HTMLButtonElement>) => void
  onEdit: (id: string) => void
  onText: (id: string, text: string) => void
  onTextRange?: (id: string, start: number, end: number) => void
  onTextBlur: () => void
  onMenu: (id: string, clientX: number, clientY: number) => void
  clipPath?: string
  maskStyle?: { clipPath?: string; maskImage?: string; WebkitMaskImage?: string; maskSize?: string; WebkitMaskSize?: string; maskPosition?: string; WebkitMaskPosition?: string; maskRepeat?: string; WebkitMaskRepeat?: string; maskMode?: string }
}) {
  if (node.visible === false) return null
  const visual = node.kind === 'instance' ? resolveInstanceTree(doc, node) : null
  const boxW = visual?.w ?? node.w
  const boxH = visual?.h ?? node.h
  const chrome = nodeChrome(visual ?? node)
  const style = textStyle(visual && node.kind !== 'instance' ? visual : node)
  const selected = selectedIds.includes(node.id)
  const overrideMark = overrideTargetId === node.id
  const container = node.kind === 'frame' || node.kind === 'group'
  const bareFill = node.fill === 'none' || chrome.fill === 'none'
  const bareStroke = (node.kind === 'frame' || node.kind === 'group') && (!node.stroke || node.stroke === 'none')
  const strokeOff = chrome.stroke === 'none' || ((node.kind === 'rect' || node.kind === 'ellipse') && node.stroke == null)
  const fillFallback = node.kind === 'frame' ? '#ffffff' : node.kind === 'rect' || node.kind === 'ellipse' || node.kind === 'vector' ? SHAPE_FILL : 'var(--color-koma-panel)'
  const painted = visual ?? node
  const fillPaints = nodePaints(painted, 'fill').filter((paint) => isPaintVisible(paint) && paintAlias(paint) !== 'none')
  const fillPaint = firstVisiblePaint(fillPaints)
  const fillFallbackCss = paintCss(doc, chrome.fill, fillFallback)
  const fill = bareFill ? 'transparent' : nodeFillCss(doc, painted, fillFallbackCss)
  const stroke = nodeStrokeCss(doc, painted, paintCss(doc, chrome.stroke, node.kind === 'line' || node.kind === 'vector' ? '#1c1c1c' : 'var(--color-koma-border)'))
  const fillLayers = bareFill || node.kind === 'line' || node.kind === 'vector' ? [] : nodeFillLayers(doc, painted, fillFallbackCss)
  const fillSizes = fillPaints.map((paint) => paint.type === 'image' ? (paint.scale === 'tile' ? 'auto' : paint.scale === 'fit' ? 'contain' : 'cover') : '100% 100%')
  const fillRepeats = fillPaints.map((paint) => paint.type === 'image' && paint.scale === 'tile' ? 'repeat' : 'no-repeat')
  const corners = cornerPixels(doc, visual ?? node)
  const radius = node.kind === 'ellipse' ? '50%' : `${corners.tl}px ${corners.tr}px ${corners.br}px ${corners.bl}px`
  const unit = 1 / Math.max(zoom, 0.25)
  const insetAt = (value: number, span: number) => Math.min(span / 2, Math.max(8 * unit, value > 0 ? value : 14 * unit))
  const clipValue = node.kind === 'instance' ? node.clip ?? visual?.clip : node.clip
  const clips = (node.kind === 'frame' || node.kind === 'instance') && clipValue !== false
  const children = visual?.children ?? (node.kind === 'instance' ? undefined : node.children)
  const rotation = node.rotation ?? 0
  const flipX = node.flipX ? -1 : 1
  const flipY = node.flipY ? -1 : 1
  const transform = rotation || node.flipX || node.flipY ? `rotate(${rotation}deg) scale(${flipX}, ${flipY})` : undefined
  const opaqueUntilEnter = node.kind === 'group' || node.kind === 'instance'
  const enteredHere = enteredContainerId === node.id
  const lockChildren = locked || !!node.locked || (opaqueUntilEnter && !enteredHere)
  const childIds = lockChildren ? [] : selectedIds
  const hitHere = !locked
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
        outline: selected ? `${unit}px solid ${SELECTION}` : undefined,
        cursor: !hitHere || node.locked || dragCursor ? undefined : 'grab',
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
        if ((node.kind === 'group' || node.kind === 'frame' || node.kind === 'instance' || node.kind === 'vector') && onEnterContainer) {
          event.stopPropagation()
          onEnterContainer(node.id, event)
        }
      }}
    >
      {container ? (
        <span
          className={`absolute left-0 truncate text-[11px] ${selected ? 'text-koma-fg' : 'text-koma-dim'} ${hitHere && !locked && !node.locked ? 'pointer-events-auto cursor-grab' : 'pointer-events-none'}`}
          style={{ top: -16, maxWidth: Math.max(boxW, 80) }}
          onPointerDown={
            hitHere && !locked && !node.locked
              ? (event) => {
                  event.stopPropagation()
                  onSelect(node.id, event)
                }
              : undefined
          }
        >
          {designLayerName(node)}
        </span>
      ) : null}
      <div
        className={`absolute inset-0 ${overrideMark ? 'ring-2 ring-inset ring-[#9747ff]' : ''}`}
        style={{
          border: undefined,
          borderRadius: radius,
          overflow: clips ? 'hidden' : undefined,
          boxShadow: painted.effects?.filter((item) => item.visible !== false && (item.kind === 'drop-shadow' || item.kind === 'inner-shadow')).map((item) => `${item.kind === 'inner-shadow' ? 'inset ' : ''}${item.x ?? 0}px ${item.y ?? 4}px ${item.blur ?? 8}px ${item.spread ?? 0}px ${item.color && item.color !== 'none' ? item.color : 'rgba(0,0,0,0.25)'}`).join(', ') || undefined,
          filter: painted.effects?.some((item) => item.visible !== false && item.kind === 'layer-blur') ? `blur(${painted.effects.find((item) => item.kind === 'layer-blur')?.blur ?? 4}px)` : undefined,
          backdropFilter: painted.effects?.some((item) => item.visible !== false && item.kind === 'background-blur') ? `blur(${painted.effects.find((item) => item.kind === 'background-blur')?.blur ?? 8}px)` : undefined,
          mixBlendMode: painted.blend && painted.blend !== 'normal' && painted.blend !== 'pass-through' ? painted.blend : undefined,
        }}
      >
        {node.kind !== 'line' && node.kind !== 'vector' ? (
          <span
            aria-hidden
            className="pointer-events-none absolute inset-0"
            style={{
              backgroundImage: fillLayers.length ? fillLayers.join(', ') : undefined,
              backgroundSize: fillLayers.length ? fillSizes.join(', ') : undefined,
              backgroundRepeat: fillLayers.length ? fillRepeats.join(', ') : undefined,
              backgroundPosition: fillLayers.length ? fillPaints.map(() => 'center').join(', ') : undefined,
              opacity: fillPaints.length === 1 ? fillPaint?.opacity ?? 1 : 1,
              borderRadius: 'inherit',
            }}
          />
        ) : null}
        {node.kind !== 'text' && node.kind !== 'vector' && !bareStroke && !strokeOff ? (
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
        {node.kind === 'vector' && node.vector ? (
          <svg className="absolute inset-0 overflow-visible" width={boxW} height={boxH}>
            <path d={vectorSvgPath(node.vector)} fill={chrome.fill === 'none' ? 'none' : fill} stroke={chrome.stroke === 'none' ? 'none' : stroke} strokeWidth={chrome.strokeWidth} strokeLinecap={painted.strokeCap === 'round' || painted.strokeCap === 'square' ? painted.strokeCap : 'butt'} strokeLinejoin={painted.strokeJoin ?? 'miter'} strokeDasharray={painted.strokeDash?.join(' ')} {...(node.svgAttrs ?? {})} />
            {nodePaints(painted, 'stroke').slice(1).filter((paint) => isPaintVisible(paint) && paintAlias(paint) !== 'none').map((paint, index) => (
              <path key={`stroke-${index}`} d={vectorSvgPath(node.vector)} fill="none" stroke={resolvePaintCss(doc, paint, stroke)} strokeWidth={strokePaintWidth(paint, chrome.strokeWidth)} strokeDasharray={strokePaintDash(paint, painted.strokeDash)} />
            ))}
          </svg>
        ) : null}
        {node.kind === 'text' && editing === node.id ? (
          <textarea
            autoFocus
            value={node.text ?? ''}
            onChange={(event) => onText(node.id, event.target.value)}
            onSelect={(event) => onTextRange?.(node.id, event.currentTarget.selectionStart, event.currentTarget.selectionEnd)}
            onKeyUp={(event) => onTextRange?.(node.id, event.currentTarget.selectionStart, event.currentTarget.selectionEnd)}
            onMouseUp={(event) => onTextRange?.(node.id, event.currentTarget.selectionStart, event.currentTarget.selectionEnd)}
            onBlur={onTextBlur}
            onPointerDown={(event) => event.stopPropagation()}
            onKeyDown={(event) => {
              if (event.key === 'Escape') {
                event.preventDefault()
                onTextBlur()
              }
            }}
            className="z-10 h-full w-full resize-none border-0 bg-transparent px-1 text-koma-fg shadow-none outline-none"
            style={{ fontSize: style.fontSize, fontFamily: style.fontFamily || undefined, fontWeight: weightCss(style.weight), fontStyle: node.italic ? 'italic' : undefined, textDecoration: [node.underline ? 'underline' : '', node.strike ? 'line-through' : ''].filter(Boolean).join(' ') || undefined, textAlign: style.align, textTransform: node.textCase === 'upper' ? 'uppercase' : node.textCase === 'lower' ? 'lowercase' : node.textCase === 'title' ? 'capitalize' : undefined, lineHeight: style.lineHeight ? `${style.lineHeight}px` : undefined, letterSpacing: style.letterSpacing ? `${style.letterSpacing}px` : undefined, color: paintCss(doc, style.color, 'var(--color-koma-fg)') }}
          />
        ) : node.kind === 'text' ? (
          <div
            className="flex h-full w-full px-1"
            style={{ fontSize: style.fontSize, fontFamily: style.fontFamily || undefined, fontWeight: weightCss(style.weight), fontStyle: node.italic ? 'italic' : undefined, textDecoration: [node.underline ? 'underline' : '', node.strike ? 'line-through' : ''].filter(Boolean).join(' ') || undefined, textTransform: node.textCase === 'upper' ? 'uppercase' : node.textCase === 'lower' ? 'lowercase' : node.textCase === 'title' ? 'capitalize' : undefined, justifyContent: style.align === 'center' ? 'center' : style.align === 'right' ? 'flex-end' : style.align === 'justify' ? 'stretch' : 'flex-start', textAlign: style.align, alignItems: style.vertical === 'top' ? 'flex-start' : style.vertical === 'bottom' ? 'flex-end' : 'center', lineHeight: style.lineHeight ? `${style.lineHeight}px` : undefined, letterSpacing: style.letterSpacing ? `${style.letterSpacing}px` : undefined, color: paintCss(doc, style.color || chrome.fill, 'var(--color-koma-fg)') }}
          >
            <span className="whitespace-pre-wrap">
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
          </div>
        ) : node.kind === 'instance' && !visual ? (
          <span className="pointer-events-none absolute left-2 top-1 truncate text-[11px] text-koma-dim">Missing component</span>
        ) : null}
        {node.kind === 'frame' && node.layoutGrids?.length ? (
          <svg className="pointer-events-none absolute inset-0 overflow-hidden" width={boxW} height={boxH}>
            {node.layoutGrids.map((grid, index) => {
              const color = grid.color || 'rgba(255,0,0,0.18)'
              if (grid.kind === 'square') {
                const size = grid.size ?? 8
                const lines = []
                for (let x = grid.offset ?? 0; x < boxW; x += size) lines.push(<line key={`v${index}-${x}`} x1={x} y1={0} x2={x} y2={boxH} stroke={color} strokeWidth={1} />)
                for (let y = grid.offset ?? 0; y < boxH; y += size) lines.push(<line key={`h${index}-${y}`} x1={0} y1={y} x2={boxW} y2={y} stroke={color} strokeWidth={1} />)
                return lines
              }
              const count = grid.count ?? 3
              const gutter = grid.gutter ?? 16
              const offset = grid.offset ?? 0
              const span = grid.kind === 'column' ? boxW : boxH
              const cell = Math.max(1, (span - offset * 2 - gutter * Math.max(0, count - 1)) / count)
              return Array.from({ length: count }, (_, at) => {
                const start = offset + at * (cell + gutter)
                return grid.kind === 'column'
                  ? <rect key={`c${index}-${at}`} x={start} y={0} width={cell} height={boxH} fill={color} />
                  : <rect key={`r${index}-${at}`} x={0} y={start} width={boxW} height={cell} fill={color} />
              })
            })}
          </svg>
        ) : null}
        {children?.map((child, index) => {
          const mask = [...(children ?? [])].slice(0, index).reverse().find((item) => item.mask)
          return (
          <DesignNodeView
            key={child.id}
            doc={doc}
            node={child}
            zoom={zoom}
            selectedIds={childIds}
            geometryId={geometryId}
            editing={editing}
            dragCursor={dragCursor}
            locked={lockChildren}
            enteredContainerId={enteredContainerId}
            overrideTargetId={overrideTargetId}
            maskStyle={mask && !child.mask ? siblingMaskStyle(mask, child, doc) : undefined}
            onEnterContainer={onEnterContainer}
            onSelect={onSelect}
            onResize={onResize}
            onRotate={onRotate}
            onCorner={onCorner}
            onEdit={onEdit}
            onText={onText}
            onTextRange={onTextRange}
            onTextBlur={onTextBlur}
            onMenu={onMenu}
          />
          )
        })}
      </div>
      {selected && !locked && !node.locked && !dragCursor && geometryId !== node.id ? (
        HANDLES.map((handle) => (
          <button
            key={handle.id}
            type="button"
            aria-label={`Resize ${handle.id}`}
            className="absolute z-10 border-0 p-0"
            style={{ left: handle.x, top: handle.y, width: 7 * unit, height: 7 * unit, background: '#ffffff', border: `${unit}px solid ${SELECTION}`, transform: 'translate(-50%, -50%)', cursor: handle.cursor }}
            onPointerDown={(event) => onResize(node.id, handle.id, event)}
          />
        ))
      ) : null}
      {selected && !locked && !node.locked && !dragCursor && geometryId !== node.id && onRotate ? (
        (['nw', 'ne', 'sw', 'se'] as const).map((corner) => (
          <button
            key={`rot-${corner}`}
            type="button"
            aria-label={`Rotate ${corner}`}
            className="absolute z-20 border-0 p-0"
            style={{
              left: corner.includes('w') ? -18 * unit : boxW + 18 * unit,
              top: corner.includes('n') ? -18 * unit : boxH + 18 * unit,
              width: 8 * unit,
              height: 8 * unit,
              background: '#ffffff',
              border: `${unit}px solid ${SELECTION}`,
              borderRadius: '50%',
              transform: 'translate(-50%, -50%)',
              cursor: 'grab',
            }}
            onPointerDown={(event) => onRotate(node.id, event)}
          />
        ))
      ) : null}
      {selected && !locked && !node.locked && !dragCursor && geometryId !== node.id && (node.kind === 'rect' || node.kind === 'frame') ? (
        ([
          { id: 'tl' as const, x: insetAt(corners.tl, boxW), y: insetAt(corners.tl, boxH), cursor: 'nwse-resize' },
          { id: 'tr' as const, x: boxW - insetAt(corners.tr, boxW), y: insetAt(corners.tr, boxH), cursor: 'nesw-resize' },
          { id: 'bl' as const, x: insetAt(corners.bl, boxW), y: boxH - insetAt(corners.bl, boxH), cursor: 'nesw-resize' },
          { id: 'br' as const, x: boxW - insetAt(corners.br, boxW), y: boxH - insetAt(corners.br, boxH), cursor: 'nwse-resize' },
        ]).map((corner) => (
          <button
            key={corner.id}
            type="button"
            aria-label={`Corner radius ${corner.id}`}
            className="absolute z-10 rounded-full border-0 p-0"
            style={{ left: corner.x, top: corner.y, width: 8 * unit, height: 8 * unit, background: '#ffffff', border: `${unit}px solid ${SELECTION}`, transform: 'translate(-50%, -50%)', cursor: corner.cursor }}
            onPointerDown={(event) => onCorner(node.id, corner.id, event)}
          />
        ))
      ) : null}
    </div>
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
    return <ellipse cx={boxW / 2} cy={boxH / 2} rx={Math.max(0.5, boxW / 2 - inset)} ry={Math.max(0.5, boxH / 2 - inset)} fill="none" stroke={color} strokeWidth={width} strokeDasharray={dash} />
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
      <rect x={inset} y={inset} width={Math.max(1, boxW - inset * 2)} height={Math.max(1, boxH - inset * 2)} rx={cornersSafe(node)} fill="none" stroke={color} strokeWidth={width} strokeDasharray={dash} strokeLinecap={linecap} strokeLinejoin={join} />
      {markerEnd === 'arrow' ? <polygon points={`${boxW},${boxH / 2} ${boxW - 8},${boxH / 2 - 4} ${boxW - 8},${boxH / 2 + 4}`} fill={color} /> : null}
      {markerStart === 'dot' ? <circle cx={0} cy={boxH / 2} r={3} fill={color} /> : null}
      {markerEnd === 'dot' ? <circle cx={boxW} cy={boxH / 2} r={3} fill={color} /> : null}
    </g>
  )
}

function cornersSafe(node: DesignNode): number {
  return typeof node.radius === 'number' ? node.radius : 0
}
