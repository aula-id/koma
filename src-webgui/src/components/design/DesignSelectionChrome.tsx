import type { PointerEvent as ReactPointerEvent } from 'react'
import {
  composeDesignAffine,
  cornerPixels,
  DESIGN_AFFINE_ID,
  designImageSize,
  designNodeMatrix,
  imageCropRect,
  isImageCropPaint,
  isPaintVisible,
  nodePaints,
  paintAlias,
  resolveInstanceTree,
  screenChromePlacement,
  translateDesignAffine,
  visibleDesignScreens,
  type DesignAffine,
  type DesignDoc,
  type DesignHandle,
  type DesignNode,
} from '../../lib/design'
import { HANDLES, SELECTION, type RadiusCorner } from './tabShared'

const HANDLE = 7
const RADIUS_DOT = 8
const ROTATE_HIT = 20

type ChromeBox = {
  id: string
  kind: DesignNode['kind']
  matrix: DesignAffine
  width: number
  height: number
  showOutline: boolean
  showHandles: boolean
  showRotate: boolean
  showCorners: boolean
  crop: boolean
  corners: { tl: number; tr: number; br: number; bl: number } | null
  zoom: number
}

function cropBox(doc: DesignDoc, node: DesignNode, boxW: number, boxH: number) {
  const painted = node.kind === 'instance' ? resolveInstanceTree(doc, node) ?? node : node
  const cropPaint = nodePaints(painted, 'fill').find((paint) => isPaintVisible(paint) && paintAlias(paint) !== 'none' && isImageCropPaint(paint))
  if (!cropPaint) return null
  return imageCropRect({ w: boxW, h: boxH }, cropPaint, cropPaint.hash ? designImageSize(doc, cropPaint.hash) : null)
}

function collectChrome(
  doc: DesignDoc,
  selectedIds: string[],
  panX: number,
  panY: number,
  zoom: number,
  dragCursor: string | null,
  enteredContainerId: string | null,
  cropEditId: string | null,
  canRotate: boolean,
): ChromeBox[] {
  const items: ChromeBox[] = []
  const visit = (node: DesignNode, toParent: DesignAffine, ids: string[], ancestorLocked: boolean) => {
    if (node.visible === false) return
    const visual = node.kind === 'instance' ? resolveInstanceTree(doc, node) : null
    const boxW = visual?.w ?? node.w
    const boxH = visual?.h ?? node.h
    const toCanvas = composeDesignAffine(toParent, designNodeMatrix(node, boxW, boxH))
    if (ids.includes(node.id)) {
      const cropping = cropEditId === node.id
      const crop = cropping ? cropBox(doc, node, boxW, boxH) : null
      const localX = crop?.x ?? 0
      const localY = crop?.y ?? 0
      const localW = crop?.w ?? boxW
      const localH = crop?.h ?? boxH
      const toBox = localX || localY ? composeDesignAffine(toCanvas, translateDesignAffine(localX, localY)) : toCanvas
      const placed = screenChromePlacement(toBox, localW, localH, panX, panY, zoom)
      const interactive = !ancestorLocked && !node.locked && !dragCursor
      const showCorners = interactive && !cropping && (node.kind === 'rect' || node.kind === 'frame')
      items.push({
        id: node.id,
        kind: node.kind,
        matrix: placed.matrix,
        width: placed.width,
        height: placed.height,
        showOutline: !cropping || !!crop,
        showHandles: interactive,
        showRotate: interactive && !cropping && canRotate,
        showCorners,
        crop: cropping,
        corners: showCorners ? cornerPixels(doc, visual ?? node) : null,
        zoom,
      })
    }
    const opaque = node.kind === 'group' || node.kind === 'instance'
    const lockChildren = ancestorLocked || !!node.locked || (opaque && enteredContainerId !== node.id)
    if (lockChildren) return
    const children = visual?.children ?? node.children
    for (const child of children ?? []) visit(child, toCanvas, ids, false)
  }
  for (const screen of visibleDesignScreens(doc)) visit(screen, DESIGN_AFFINE_ID, selectedIds, false)
  return items
}

function matrixCss(matrix: DesignAffine): string {
  return `matrix(${matrix.a}, ${matrix.b}, ${matrix.c}, ${matrix.d}, ${matrix.e}, ${matrix.f})`
}

function cornerInset(radius: number, span: number, zoom: number): number {
  return Math.min(span / 2, Math.max(8, radius > 0 ? radius * zoom : 14))
}

export function DesignSelectionChrome({
  doc,
  panX,
  panY,
  zoom,
  selectedIds,
  dragCursor,
  enteredContainerId,
  cropEditId,
  onResize,
  onRotate,
  onCorner,
}: {
  doc: DesignDoc
  panX: number
  panY: number
  zoom: number
  selectedIds: string[]
  dragCursor: string | null
  enteredContainerId: string | null
  cropEditId: string | null
  onResize: (id: string, handle: DesignHandle, event: ReactPointerEvent<Element>) => void
  onRotate?: (id: string, event: ReactPointerEvent<Element>) => void
  onCorner: (id: string, corner: RadiusCorner, event: ReactPointerEvent<Element>) => void
}) {
  if (!selectedIds.length) return null
  const items = collectChrome(doc, selectedIds, panX, panY, zoom, dragCursor, enteredContainerId, cropEditId, !!onRotate)
  if (!items.length) return null
  return (
    <div className="pointer-events-none absolute inset-0 z-10">
      {items.map((item) => {
        const { matrix } = item
        const outside = item.width <= 25 || item.height <= 25
        const inset = outside ? -HANDLE / 2 : HANDLE / 2
        return (
          <div
            key={item.id}
            className="pointer-events-none absolute left-0 top-0"
            style={{
              width: item.width,
              height: item.height,
              transformOrigin: '0 0',
              transform: matrixCss(matrix),
              boxShadow: item.showOutline ? `inset 0 0 0 1px ${SELECTION}` : undefined,
            }}
          >
            {item.showHandles
              ? HANDLES.map((handle) => {
                  const x = handle.id.includes('w') ? inset : handle.id.includes('e') ? item.width - inset : item.width / 2
                  const y = handle.id.includes('n') ? inset : handle.id.includes('s') ? item.height - inset : item.height / 2
                  const round = handle.id.length === 2
                  return (
                    <button
                      key={handle.id}
                      type="button"
                      aria-label={`${item.crop ? 'Crop' : 'Resize'} ${handle.id}`}
                      className="absolute p-0"
                      style={{
                        left: x - HANDLE / 2,
                        top: y - HANDLE / 2,
                        width: HANDLE,
                        height: HANDLE,
                        minWidth: 0,
                        minHeight: 0,
                        padding: 0,
                        boxSizing: 'border-box',
                        background: '#ffffff',
                        border: `1px solid ${SELECTION}`,
                        borderRadius: round ? 999 : 0,
                        pointerEvents: 'auto',
                        cursor: handle.cursor,
                      }}
                      onPointerDown={(event) => onResize(item.id, handle.id, event)}
                    />
                  )
                })
              : null}
            {item.showCorners && item.corners
              ? (
                  [
                    { id: 'tl' as const, x: cornerInset(item.corners.tl, item.width, item.zoom), y: cornerInset(item.corners.tl, item.height, item.zoom), cursor: 'nwse-resize' },
                    { id: 'tr' as const, x: item.width - cornerInset(item.corners.tr, item.width, item.zoom), y: cornerInset(item.corners.tr, item.height, item.zoom), cursor: 'nesw-resize' },
                    { id: 'bl' as const, x: cornerInset(item.corners.bl, item.width, item.zoom), y: item.height - cornerInset(item.corners.bl, item.height, item.zoom), cursor: 'nesw-resize' },
                    { id: 'br' as const, x: item.width - cornerInset(item.corners.br, item.width, item.zoom), y: item.height - cornerInset(item.corners.br, item.height, item.zoom), cursor: 'nwse-resize' },
                  ] as const
                ).map((corner) => (
                  <button
                    key={corner.id}
                    type="button"
                    aria-label={`Corner radius ${corner.id}`}
                    className="absolute p-0"
                    style={{
                      left: corner.x - RADIUS_DOT / 2,
                      top: corner.y - RADIUS_DOT / 2,
                      width: RADIUS_DOT,
                      height: RADIUS_DOT,
                      minWidth: 0,
                      minHeight: 0,
                      padding: 0,
                      boxSizing: 'border-box',
                      background: '#ffffff',
                      border: `1px solid ${SELECTION}`,
                      borderRadius: 999,
                      pointerEvents: 'auto',
                      cursor: corner.cursor,
                    }}
                    onPointerDown={(event) => onCorner(item.id, corner.id, event)}
                  />
                ))
              : null}
            {item.showRotate && onRotate
              ? (['nw', 'ne', 'sw', 'se'] as const).map((corner) => (
                  <button
                    key={`rot-${corner}`}
                    type="button"
                    aria-label={`Rotate ${corner}`}
                    className="absolute z-20 border-0 bg-transparent p-0"
                    style={{
                      left: corner.includes('w') ? -ROTATE_HIT : item.width,
                      top: corner.includes('n') ? -ROTATE_HIT : item.height,
                      width: ROTATE_HIT,
                      height: ROTATE_HIT,
                      minWidth: 0,
                      minHeight: 0,
                      pointerEvents: 'auto',
                      cursor: 'grab',
                    }}
                    onPointerDown={(event) => onRotate(item.id, event)}
                  />
                ))
              : null}
          </div>
        )
      })}
    </div>
  )
}
