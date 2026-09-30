import type { PointerEvent as ReactPointerEvent } from 'react'
import {
  cornerPixels,
  designLayerName,
  nodeChrome,
  resolveInstanceTree,
  resolveRef,
  textStyle,
  vectorSvgPath,
  type DesignDoc,
  type DesignHandle,
  type DesignNode,
  type DesignWeight,
} from '../../lib/design'
import { HANDLES, SELECTION, SHAPE_FILL, paintCss, weightCss, type RadiusCorner } from './tabShared'

export function DesignNodeView({
  doc,
  node,
  zoom,
  selectedIds,
  editing,
  dragCursor,
  locked = false,
  enteredContainerId = null,
  overrideTargetId = null,
  onSelect,
  onResize,
  onCorner,
  onEdit,
  onText,
  onTextBlur,
  onMenu,
  onEnterContainer,
}: {
  doc: DesignDoc
  node: DesignNode
  zoom: number
  selectedIds: string[]
  editing: string | null
  dragCursor: string | null
  locked?: boolean
  enteredContainerId?: string | null
  overrideTargetId?: string | null
  onSelect: (id: string, event: ReactPointerEvent<HTMLDivElement>) => void
  onEnterContainer?: (id: string, event: ReactPointerEvent<HTMLDivElement>) => void
  onResize: (id: string, handle: DesignHandle, event: ReactPointerEvent<HTMLButtonElement>) => void
  onCorner: (id: string, corner: RadiusCorner, event: ReactPointerEvent<HTMLButtonElement>) => void
  onEdit: (id: string) => void
  onText: (id: string, text: string) => void
  onTextBlur: () => void
  onMenu: (id: string, clientX: number, clientY: number) => void
}) {
  if (node.visible === false) return null
  const visual = node.kind === 'instance' ? resolveInstanceTree(doc, node) : null
  const chrome = nodeChrome(visual ?? node)
  const style = textStyle(visual && node.kind !== 'instance' ? visual : node)
  const selected = selectedIds.includes(node.id)
  const overrideMark = overrideTargetId === node.id
  const container = node.kind === 'frame' || node.kind === 'group'
  const bareFill = node.fill === 'none' || chrome.fill === 'none'
  const bareStroke = (node.kind === 'frame' || node.kind === 'group') && (!node.stroke || node.stroke === 'none')
  const strokeOff = chrome.stroke === 'none' || ((node.kind === 'rect' || node.kind === 'ellipse') && node.stroke == null)
  const fillFallback = node.kind === 'frame' ? '#ffffff' : node.kind === 'rect' || node.kind === 'ellipse' || node.kind === 'vector' ? SHAPE_FILL : 'var(--color-koma-panel)'
  const fill = bareFill ? 'transparent' : paintCss(doc, chrome.fill, fillFallback)
  const stroke = paintCss(doc, chrome.stroke, node.kind === 'line' || node.kind === 'vector' ? '#1c1c1c' : 'var(--color-koma-border)')
  const corners = cornerPixels(doc, node)
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
  const drillParent = node.kind === 'group' || node.kind === 'frame' || node.kind === 'instance'
  const enteredHere = enteredContainerId === node.id
  const lockChildren = locked || node.locked || (drillParent && !enteredHere)
  const childIds = lockChildren ? [] : selectedIds
  const hitHere = !locked
  return (
    <div
      className={`absolute select-none ${hitHere ? 'pointer-events-auto' : 'pointer-events-none'}`}
      style={{
        left: node.x,
        top: node.y,
        width: node.w,
        height: node.h,
        opacity: chrome.opacity,
        transform,
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
        if ((node.kind === 'group' || node.kind === 'frame' || node.kind === 'instance') && onEnterContainer) {
          event.stopPropagation()
          onEnterContainer(node.id, event)
        }
      }}
    >
      {container ? (
        <span
          className={`absolute left-0 truncate text-[11px] ${selected ? 'text-koma-fg' : 'text-koma-dim'} ${hitHere && !locked && !node.locked ? 'pointer-events-auto cursor-grab' : 'pointer-events-none'}`}
          style={{ top: -16, maxWidth: Math.max(node.w, 80) }}
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
          background: node.kind === 'line' || node.kind === 'vector' ? 'transparent' : fill,
          border: bareStroke || strokeOff || node.kind === 'line' || node.kind === 'vector' ? undefined : `${chrome.strokeWidth}px solid ${stroke}`,
          borderRadius: radius,
          overflow: clips ? 'hidden' : undefined,
        }}
      >
        {node.kind === 'line' ? (
          <svg className="absolute inset-0 overflow-visible" width={node.w} height={node.h}>
            <line x1={0} y1={node.h / 2} x2={node.w} y2={node.h / 2} stroke={stroke} strokeWidth={chrome.strokeWidth} />
          </svg>
        ) : null}
        {node.kind === 'vector' && node.vector ? (
          <svg className="absolute inset-0 overflow-visible" width={node.w} height={node.h}>
            <path d={vectorSvgPath(node.vector)} fill={chrome.fill === 'none' ? 'none' : fill} stroke={chrome.stroke === 'none' ? 'none' : stroke} strokeWidth={chrome.strokeWidth} />
          </svg>
        ) : null}
        {node.kind === 'text' && editing === node.id ? (
          <input
            autoFocus
            value={node.text ?? ''}
            onChange={(event) => onText(node.id, event.target.value)}
            onBlur={onTextBlur}
            onPointerDown={(event) => event.stopPropagation()}
            className="z-10 h-full w-full border-0 bg-transparent px-1 text-koma-fg shadow-none outline-none"
            style={{ fontSize: style.fontSize, fontFamily: style.fontFamily || undefined, fontWeight: weightCss(style.weight), textAlign: style.align, lineHeight: style.lineHeight ? `${style.lineHeight}px` : undefined, letterSpacing: style.letterSpacing ? `${style.letterSpacing}px` : undefined, color: paintCss(doc, style.color, 'var(--color-koma-fg)') }}
          />
        ) : node.kind === 'text' ? (
          <div
            className="flex h-full w-full px-1"
            style={{ fontSize: style.fontSize, fontFamily: style.fontFamily || undefined, fontWeight: weightCss(style.weight), justifyContent: style.align === 'center' ? 'center' : style.align === 'right' ? 'flex-end' : 'flex-start', alignItems: style.vertical === 'top' ? 'flex-start' : style.vertical === 'bottom' ? 'flex-end' : 'center', lineHeight: style.lineHeight ? `${style.lineHeight}px` : undefined, letterSpacing: style.letterSpacing ? `${style.letterSpacing}px` : undefined, color: paintCss(doc, style.color, 'var(--color-koma-fg)') }}
          >
            <span className="truncate">{node.text || 'Text'}</span>
          </div>
        ) : node.kind === 'instance' && !visual ? (
          <span className="pointer-events-none absolute left-2 top-1 truncate text-[11px] text-koma-dim">Missing component</span>
        ) : null}
        {children?.map((child) => (
          <DesignNodeView
            key={child.id}
            doc={doc}
            node={child}
            zoom={zoom}
            selectedIds={childIds}
            editing={editing}
            dragCursor={dragCursor}
            locked={lockChildren}
            enteredContainerId={enteredContainerId}
            overrideTargetId={overrideTargetId}
            onEnterContainer={onEnterContainer}
            onSelect={onSelect}
            onResize={onResize}
            onCorner={onCorner}
            onEdit={onEdit}
            onText={onText}
            onTextBlur={onTextBlur}
            onMenu={onMenu}
          />
        ))}
      </div>
      {selected && !locked && !node.locked && !dragCursor ? (
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
      {selected && !locked && !node.locked && !dragCursor && (node.kind === 'rect' || node.kind === 'frame') ? (
        ([
          { id: 'tl' as const, x: insetAt(corners.tl, node.w), y: insetAt(corners.tl, node.h), cursor: 'nwse-resize' },
          { id: 'tr' as const, x: node.w - insetAt(corners.tr, node.w), y: insetAt(corners.tr, node.h), cursor: 'nesw-resize' },
          { id: 'bl' as const, x: insetAt(corners.bl, node.w), y: node.h - insetAt(corners.bl, node.h), cursor: 'nesw-resize' },
          { id: 'br' as const, x: node.w - insetAt(corners.br, node.w), y: node.h - insetAt(corners.br, node.h), cursor: 'nwse-resize' },
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
