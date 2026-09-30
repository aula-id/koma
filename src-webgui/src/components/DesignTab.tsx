import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react'
import { AlignCenterHorizontal, AlignCenterVertical, AlignEndHorizontal, AlignEndVertical, AlignHorizontalSpaceBetween, AlignStartHorizontal, AlignStartVertical, AlignVerticalSpaceBetween, ArrowDown, ArrowRight, ChevronRight, Circle, Component, Eye, EyeOff, FlipHorizontal2, FlipVertical2, Frame, Group, Hand, Minus, MousePointer2, PenTool, Plus, RotateCw, Spline, Square, TextAlignCenter, TextAlignEnd, TextAlignStart, Type, PanelRightClose, PanelRightOpen, X } from 'lucide-react'
import { TokenEditor } from './panels/DesignPanel'
import { DesignMenu, type DesignMenuItem } from './DesignMenu'
import { getDesignUi, publishDesignUi, type DesignLayerOp } from '../lib/designUi'
import {
  COMPONENT_MIME,
  DESIGN_MIME,
  alignDesignNodes,
  addComponentVariant,
  autoLayoutDesign,
  canLeaveParent,
  canvasDeltaToSpace,
  canvasToContent,
  componentView,
  contentAngle,
  copyTree,
  createComponentFromFrame,
  createNode,
  designChatNote,
  designChatTitle,
  applyDesignStyle,
  designCanvasBox,
  designDrop,
  designLayerName,
  designObjectSnap,
  designSnapScene,
  designStyle,
  duplicateDesignNodes,
  flowBreakBar,
  frameDesignView,
  flowInsertIndex,
  inFlowBand,
  designPath,
  designQueryNode,
  deleteDesignNode,
  flipDesignNode,
  frameAtPoint,
  findDesignNode,
  insertDesignNode,
  layoutDesign,
  locateDesign,
  makeInstance,
  moveDesignNode,
  nextVariantProps,
  nodeChrome,
  nodeFromPen,
  nodeOrigin,
  nudgeDesignNodes,
  orderDesignNode,
  parseDesign,
  placeDesignNode,
  renameComponent,
  reorderDesignNode,
  resetInstanceOverrides,
  resizeDesignNode,
  resolveInstanceTree,
  resolveRef,
  selectAllDesign,
  selectDesignHit,
  selectDesignRect,
  sharedValue,
  serializeDesign,
  setDesignLocked,
  setDesignMode,
  setDesignVisible,
  setInstanceVariant,
  setVariantProps,
  snapDesign,
  stackDesign,
  textStyle,
  updateDesignNode,
  vectorSvgPath,
  writeComponentView,
  unwrapDesignNode,
  wrapDesignNodes,
  type DesignAlignAxis,
  type DesignAlignEdge,
  type DesignDoc,
  type DesignFlowBar,
  type DesignGuide,
  type DesignHandle,
  type DesignMeasure,
  type DesignOrder,
  type DesignRect,
  type DesignStyle,
  type DesignPenPoint,
  type DesignQuery,
  type DesignNode,
  type DesignWeight,
} from '../lib/design'
import { designPngBase64 } from '../lib/designRender'
import { fileKey } from '../store/coding'
import { isTabVisible } from '../store/editorGroups'
import { useKoma } from '../store/koma'
import type { Tab } from '../store/types/tabs'
import { showCodingHistory } from './CodingHistory'
import { EditorChrome } from './EditorChrome'

const UNDO_CAP = 50
const ZOOM_MIN = 0.25
const ZOOM_MAX = 64
const SELECTION = 'var(--koma-accent)'
const SHAPE_FILL = '#d9d9d9'
const HANDLES: { id: DesignHandle; x: string; y: string; cursor: string }[] = [
  { id: 'nw', x: '0%', y: '0%', cursor: 'nwse-resize' },
  { id: 'n', x: '50%', y: '0%', cursor: 'ns-resize' },
  { id: 'ne', x: '100%', y: '0%', cursor: 'nesw-resize' },
  { id: 'e', x: '100%', y: '50%', cursor: 'ew-resize' },
  { id: 'se', x: '100%', y: '100%', cursor: 'nwse-resize' },
  { id: 's', x: '50%', y: '100%', cursor: 'ns-resize' },
  { id: 'sw', x: '0%', y: '100%', cursor: 'nesw-resize' },
  { id: 'w', x: '0%', y: '50%', cursor: 'ew-resize' },
]

type Tool = 'select' | 'frame' | 'rect' | 'ellipse' | 'line' | 'pen' | 'text' | 'pan'
type DrawShape = 'frame' | 'rect' | 'ellipse' | 'line' | 'text'
type View = { panX: number; panY: number; zoom: number }
type Ghost =
  | { kind: 'marquee'; x: number; y: number; w: number; h: number }
  | { kind: 'shape'; shape: DrawShape; x: number; y: number; w: number; h: number; rotation: number }
type PenDraft = { id: string; parentId: string | null; points: DesignPenPoint[] }
type DesignCommands = {
  copy: () => void
  paste: (at?: { x: number; y: number }) => void
  duplicate: () => void
  copyStyle: () => void
  pasteStyle: () => void
  selectAll: () => void
  nudge: (dx: number, dy: number, duplicate: boolean) => void
  zoom: (scope: 'all' | 'selection') => void
  remove: () => void
  wrap: (kind: 'group' | 'frame') => void
  unwrap: () => void
  order: (order: DesignOrder) => void
  flip: (axis: 'x' | 'y') => void
  hide: () => void
  lock: () => void
  auto: () => void
  component: () => void
  select: (id: string) => void
}
type RadiusCorner = 'tl' | 'tr' | 'bl' | 'br'
type Drag =
  | { kind: 'move'; ids: string[]; startX: number; startY: number; origins: Record<string, { x: number; y: number }>; remembered: boolean; moved: boolean; broke: boolean; alt: boolean; scene: { moving: DesignRect; targets: DesignRect[] } | null }
  | { kind: 'resize'; id: string; handle: DesignHandle; startX: number; startY: number; node: DesignNode; remembered: boolean }
  | { kind: 'radius'; id: string; corner: RadiusCorner; startX: number; startY: number; radius: number; node: DesignNode; remembered: boolean }
  | { kind: 'pan'; lastX: number; lastY: number }
  | { kind: 'marquee'; x0: number; y0: number; x1: number; y1: number }
  | { kind: 'draw'; shape: DrawShape; cx: number; cy: number; x0: number; y0: number; x1: number; y1: number; parentId: string | null }
  | { kind: 'pen'; index: number; space: boolean }

let copiedShape: { nodes: DesignNode[]; parentId: string | null } | null = null
let copiedStyle: DesignStyle | null = null
let mintSeq = 0

function drawnBox(x0: number, y0: number, x1: number, y1: number, grid: number, snap: boolean) {
  const x = snapDesign(Math.min(x0, x1), grid, snap)
  const y = snapDesign(Math.min(y0, y1), grid, snap)
  const right = snapDesign(Math.max(x0, x1), grid, snap)
  const bottom = snapDesign(Math.max(y0, y1), grid, snap)
  return { x, y, w: Math.max(0, right - x), h: Math.max(0, bottom - y) }
}

function lineBox(x0: number, y0: number, x1: number, y1: number) {
  const dx = x1 - x0
  const dy = y1 - y0
  const length = Math.max(1, Math.hypot(dx, dy))
  const rotation = (Math.atan2(dy, dx) * 180) / Math.PI
  return { x: (x0 + x1) / 2 - length / 2, y: (y0 + y1) / 2 - 1, w: length, h: 2, rotation }
}

/** Shift keeps a square. Alt draws the opposite corner around the press point. */
function shapeCorners(cx: number, cy: number, x: number, y: number, shift: boolean, alt: boolean) {
  let dx = x - cx
  let dy = y - cy
  if (shift) {
    const side = Math.max(Math.abs(dx), Math.abs(dy))
    dx = Math.sign(dx || 1) * side
    dy = Math.sign(dy || 1) * side
  }
  if (alt) return { x0: cx - dx, y0: cy - dy, x1: cx + dx, y1: cy + dy }
  return { x0: cx, y0: cy, x1: cx + dx, y1: cy + dy }
}

/** Shift snaps a line to 45 degrees. Alt extends it through the press point. */
function lineEnds(cx: number, cy: number, x: number, y: number, shift: boolean, alt: boolean) {
  let dx = x - cx
  let dy = y - cy
  if (shift) {
    const length = Math.hypot(dx, dy)
    const angle = Math.round(Math.atan2(dy, dx) / (Math.PI / 4)) * (Math.PI / 4)
    dx = Math.cos(angle) * length
    dy = Math.sin(angle) * length
  }
  if (alt) return { x0: cx - dx, y0: cy - dy, x1: cx + dx, y1: cy + dy }
  return { x0: cx, y0: cy, x1: cx + dx, y1: cy + dy }
}

function mintId(prefix: string): string {
  mintSeq += 1
  return `${prefix}${Date.now().toString(36)}${mintSeq}`
}

function instanceAxes(doc: DesignDoc, node: DesignNode): { name: string; values: string[]; current: string }[] | null {
  if (node.kind !== 'instance' || !node.component) return null
  const component = doc.components.find((item) => item.id === node.component)
  if (!component?.axes) return []
  return Object.entries(component.axes).map(([name, values]) => ({
    name,
    values,
    current: node.variant?.[name] ?? values[0] ?? '',
  }))
}

function editingDoc(stored: DesignDoc, focusId: string | null): DesignDoc {
  if (!focusId) return stored
  return componentView(stored, focusId) ?? stored
}

function projectDoc(stored: DesignDoc, focusId: string | null, view: DesignDoc): DesignDoc {
  if (!focusId) return view
  return writeComponentView(stored, focusId, view) ?? stored
}

function paintCss(doc: DesignDoc, ref: string, fallback: string): string {
  if (ref === 'none') return 'transparent'
  if (!ref) return fallback
  return resolveRef(doc, ref) || fallback
}

function weightCss(weight: DesignWeight): number {
  if (weight === 'bold') return 700
  if (weight === 'medium') return 500
  return 400
}

export function DesignTab({ tab }: { tab: Extract<Tab, { kind: 'design' }> }) {
  const key = fileKey(tab.root, tab.path)
  const file = useKoma((s) => s.design.docs[key])
  const updateDesign = useKoma((s) => s.updateDesign)
  const saveDesign = useKoma((s) => s.saveDesign)
  const active = useKoma((s) => s.ui.activeTabId === tab.id && isTabVisible(s.ui, tab.id))
  const canvasRef = useRef<HTMLDivElement>(null)
  const pastRef = useRef<DesignDoc[]>([])
  const futureRef = useRef<DesignDoc[]>([])
  const nudgeOpenRef = useRef(false)
  const nudgeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const dragRef = useRef<Drag | null>(null)
  const labelNoted = useRef(false)
  const spaceRef = useRef(false)
  const toolRef = useRef<Tool>('select')
  const viewRef = useRef<View>({ panX: 40, panY: 40, zoom: 1 })
  const updateRef = useRef(updateDesign)
  updateRef.current = updateDesign
  const [view, setView] = useState<View>(viewRef.current)
  const [tool, setTool] = useState<Tool>('select')
  const [selection, setSelection] = useState<string[]>([])
  const [editing, setEditing] = useState<string | null>(null)
  const [propsOpen, setPropsOpen] = useState(true)
  const [ghost, setGhost] = useState<Ghost | null>(null)
  const [flowBar, setFlowBar] = useState<DesignFlowBar | null>(null)
  const [snapMarks, setSnapMarks] = useState<{ guides: DesignGuide[]; measures: DesignMeasure[] } | null>(null)
  const [pen, setPen] = useState<PenDraft | null>(null)
  const [penHover, setPenHover] = useState<{ x: number; y: number } | null>(null)
  const [penHandle, setPenHandle] = useState<number | null>(null)
  const [menu, setMenu] = useState<{ x: number; y: number; canvasX: number; canvasY: number } | null>(null)
  const commandsRef = useRef<DesignCommands | null>(null)
  const stepOutRef = useRef<() => void>(() => {})
  const layerOpsRef = useRef<(action: DesignLayerOp) => void>(() => {})
  const penApplyRef = useRef<(draft: PenDraft, closed?: boolean) => void>(() => {})
  const penRef = useRef<PenDraft | null>(null)
  const penNoted = useRef(false)
  const selectionRef = useRef<string[]>([])
  selectionRef.current = selection
  penRef.current = pen
  const [spaceDown, setSpaceDown] = useState(false)
  const [dragCursor, setDragCursor] = useState<string | null>(null)
  const [rev, setRev] = useState(0)
  const [focusId, setFocusId] = useState<string | null>(null)
  const focusRef = useRef<string | null>(null)
  toolRef.current = tool
  focusRef.current = focusId

  useEffect(() => {
    pastRef.current = []
    futureRef.current = []
    setSelection([])
    setEditing(null)
    setFocusId(null)
    setPen(null)
    setPenHover(null)
    setPenHandle(null)
    setMenu(null)
    const next = { panX: 40, panY: 40, zoom: 1 }
    viewRef.current = next
    setView(next)
    setRev((value) => value + 1)
  }, [key])

  useEffect(() => {
    const onFocus = (event: Event) => {
      const detail = (event as CustomEvent<{ root: string; path: string; componentId: string | null }>).detail
      if (!detail || detail.root !== tab.root || detail.path !== tab.path) return
      setFocusId(detail.componentId)
      setSelection([])
      setEditing(null)
      setPen(null)
      setPenHover(null)
      setPenHandle(null)
    }
    window.addEventListener('koma-design-focus', onFocus)
    return () => window.removeEventListener('koma-design-focus', onFocus)
  }, [tab.path, tab.root])

  useEffect(() => {
    if (!active) {
      const current = getDesignUi()
      if (current && current.root === tab.root && current.path === tab.path) publishDesignUi(null)
      return
    }
    publishDesignUi({ root: tab.root, path: tab.path, selection, focusId })
  }, [active, focusId, selection, tab.path, tab.root])

  useEffect(() => {
    return () => {
      if (nudgeTimerRef.current) clearTimeout(nudgeTimerRef.current)
      const current = getDesignUi()
      if (current && current.root === tab.root && current.path === tab.path) publishDesignUi(null)
    }
  }, [tab.path, tab.root])

  useEffect(() => {
    const onLayer = (event: Event) => {
      const detail = (event as CustomEvent<{ root: string; path: string; action: DesignLayerOp }>).detail
      if (!detail || detail.root !== tab.root || detail.path !== tab.path) return
      layerOpsRef.current(detail.action)
    }
    window.addEventListener('koma-design-layer', onLayer)
    return () => window.removeEventListener('koma-design-layer', onLayer)
  }, [tab.path, tab.root])

  useEffect(() => {
    if (tool === 'pen') return
    penRef.current = null
    penNoted.current = false
    setPen(null)
    setPenHover(null)
    setPenHandle(null)
  }, [tool])

  useEffect(() => {
    if (!focusId || !file) return
    if (!file.doc.components.some((component) => component.id === focusId)) setFocusId(null)
  }, [file, focusId])

  const closeNudge = () => {
    if (nudgeTimerRef.current) {
      clearTimeout(nudgeTimerRef.current)
      nudgeTimerRef.current = null
    }
    nudgeOpenRef.current = false
  }

  const note = (doc: DesignDoc) => {
    closeNudge()
    pastRef.current.push(doc)
    if (pastRef.current.length > UNDO_CAP) pastRef.current.shift()
    futureRef.current = []
    setRev((value) => value + 1)
  }
  const noteRef = useRef(note)
  noteRef.current = note

  const commit = (view: DesignDoc) => {
    const stored = useKoma.getState().design.docs[key]?.doc
    if (!stored) return
    const next = projectDoc(stored, focusRef.current, layoutDesign(view))
    if (serializeDesign(stored) === serializeDesign(next)) return
    note(stored)
    updateDesign(tab.root, tab.path, next)
  }
  const commitStored = (next: DesignDoc, noteOnce = false) => {
    const stored = useKoma.getState().design.docs[key]?.doc
    if (!stored) return
    const laid = layoutDesign(next)
    if (serializeDesign(stored) === serializeDesign(laid)) return
    if (noteOnce) {
      if (!labelNoted.current) {
        note(stored)
        labelNoted.current = true
      }
    } else note(stored)
    updateDesign(tab.root, tab.path, laid)
  }
  const commitRef = useRef(commit)
  commitRef.current = commit

  const undo = () => {
    closeNudge()
    const prev = pastRef.current.pop()
    if (!prev) return
    const current = useKoma.getState().design.docs[key]?.doc
    if (current) futureRef.current.push(current)
    setRev((value) => value + 1)
    setEditing(null)
    updateDesign(tab.root, tab.path, prev)
  }
  const redo = () => {
    closeNudge()
    const next = futureRef.current.pop()
    if (!next) return
    const current = useKoma.getState().design.docs[key]?.doc
    if (current) pastRef.current.push(current)
    setRev((value) => value + 1)
    setEditing(null)
    updateDesign(tab.root, tab.path, next)
  }
  const undoRef = useRef(undo)
  const redoRef = useRef(redo)
  undoRef.current = undo
  redoRef.current = redo

  const revert = () => {
    const saved = useKoma.getState().design.docs[key]?.savedText
    if (!saved) return
    const parsed = parseDesign(saved)
    if (parsed.error) return
    pastRef.current = []
    futureRef.current = []
    setRev((value) => value + 1)
    setEditing(null)
    updateDesign(tab.root, tab.path, parsed.doc)
  }

  const applyView = (next: View) => {
    viewRef.current = next
    setView(next)
  }

  const toDoc = (clientX: number, clientY: number) => {
    const rect = canvasRef.current?.getBoundingClientRect()
    if (!rect) return null
    const current = viewRef.current
    return {
      x: (clientX - rect.left - current.panX) / current.zoom,
      y: (clientY - rect.top - current.panY) / current.zoom,
    }
  }

  const zoomTo = (ids: string[] | null) => {
    const stored = useKoma.getState().design.docs[key]?.doc
    const canvas = canvasRef.current
    if (!stored || !canvas) return
    const open = editingDoc(stored, focusRef.current)
    const targets = ids?.length ? ids : open.screens.map((screen) => screen.id)
    const boxes = targets.flatMap((id) => {
      const box = designCanvasBox(open, id)
      return box ? [box] : []
    })
    const framed = frameDesignView(boxes, canvas.clientWidth, canvas.clientHeight, ZOOM_MIN, ZOOM_MAX)
    if (framed) applyView(framed)
  }

  const zoomAt = (clientX: number, clientY: number, nextZoom: number) => {
    const rect = canvasRef.current?.getBoundingClientRect()
    if (!rect) return
    const current = viewRef.current
    const zoom = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, nextZoom))
    const cx = clientX - rect.left
    const cy = clientY - rect.top
    const docX = (cx - current.panX) / current.zoom
    const docY = (cy - current.panY) / current.zoom
    applyView({ zoom, panX: cx - docX * zoom, panY: cy - docY * zoom })
  }

  useEffect(() => {
    if (!dragCursor) return
    const previous = document.body.style.cursor
    document.body.style.cursor = dragCursor
    return () => {
      document.body.style.cursor = previous
    }
  }, [dragCursor])

  useEffect(() => {
    const onRestore = (event: Event) => {
      const detail = (event as CustomEvent<{ root: string; path: string }>).detail
      if (!detail || detail.root !== tab.root || detail.path !== tab.path) return
      pastRef.current = []
      futureRef.current = []
      setEditing(null)
      setRev((value) => value + 1)
    }
    const commitDrawn = (doc: DesignDoc, drag: Extract<Drag, { kind: 'draw' }>) => {
      const stored = useKoma.getState().design.docs[key]?.doc
      if (!stored) return
      const tiny = Math.hypot(drag.x1 - drag.x0, drag.y1 - drag.y0) < 4
      const id = mintId(drag.shape[0])
      let node = createNode(drag.shape, id, drag.x0, drag.y0)
      if (!tiny && drag.shape === 'line') {
        const line = lineBox(drag.x0, drag.y0, drag.x1, drag.y1)
        node = { ...node, x: line.x, y: line.y, w: line.w, h: line.h, rotation: contentAngle(doc, drag.parentId, line.rotation) }
      } else if (!tiny) {
        const box = drawnBox(drag.x0, drag.y0, drag.x1, drag.y1, doc.grid, doc.snap)
        node = { ...node, x: box.x, y: box.y, w: Math.max(1, box.w), h: Math.max(1, box.h) }
      } else {
        node = { ...node, x: snapDesign(drag.x0, doc.grid, doc.snap), y: snapDesign(drag.y0, doc.grid, doc.snap) }
      }
      const local = canvasToContent(doc, drag.parentId, node.x, node.y)
      if (!local) return
      node = { ...node, x: snapDesign(local.x, doc.grid, doc.snap), y: snapDesign(local.y, doc.grid, doc.snap) }
      let next = doc
      if (drag.parentId) next = insertDesignNode(doc, drag.parentId, node)
      else if (focusRef.current && node.kind !== 'frame' && node.kind !== 'group') {
        const screen = createNode('frame', mintId('f'), node.x, node.y)
        node.x = 16
        node.y = 16
        screen.children = [node]
        next = insertDesignNode(doc, null, screen)
      } else next = insertDesignNode(doc, null, node)
      const laid = projectDoc(stored, focusRef.current, layoutDesign(next))
      if (serializeDesign(laid) === serializeDesign(stored)) return
      noteRef.current(stored)
      updateRef.current(tab.root, tab.path, laid)
      setSelection([node.id])
      setPropsOpen(true)
      setTool('select')
    }
    const move = (event: PointerEvent) => {
      const drag = dragRef.current
      if (toolRef.current === 'pen' && drag?.kind !== 'pen') {
        const hover = toDoc(event.clientX, event.clientY)
        if (hover) setPenHover(hover)
      }
      if (!drag) return
      if (drag.kind === 'pan') {
        const dx = event.clientX - drag.lastX
        const dy = event.clientY - drag.lastY
        drag.lastX = event.clientX
        drag.lastY = event.clientY
        const current = viewRef.current
        applyView({ ...current, panX: current.panX + dx, panY: current.panY + dy })
        return
      }
      const point = toDoc(event.clientX, event.clientY)
      if (drag.kind === 'marquee' && point) {
        drag.x1 = point.x
        drag.y1 = point.y
        const box = drawnBox(drag.x0, drag.y0, drag.x1, drag.y1, 1, false)
        setGhost({ kind: 'marquee', ...box })
        return
      }
      if (drag.kind === 'draw' && point) {
        if (drag.shape === 'line') {
          const ends = lineEnds(drag.cx, drag.cy, point.x, point.y, event.shiftKey, event.altKey)
          drag.x0 = ends.x0
          drag.y0 = ends.y0
          drag.x1 = ends.x1
          drag.y1 = ends.y1
          const line = lineBox(drag.x0, drag.y0, drag.x1, drag.y1)
          setGhost({ kind: 'shape', shape: 'line', ...line })
        } else {
          const corners = shapeCorners(drag.cx, drag.cy, point.x, point.y, event.shiftKey, event.altKey)
          drag.x0 = corners.x0
          drag.y0 = corners.y0
          drag.x1 = corners.x1
          drag.y1 = corners.y1
          const stored = useKoma.getState().design.docs[key]?.doc
          const doc = stored ? editingDoc(stored, focusRef.current) : null
          const box = drawnBox(drag.x0, drag.y0, drag.x1, drag.y1, doc?.grid ?? 8, !!doc?.snap)
          setGhost({ kind: 'shape', shape: drag.shape, ...box, rotation: 0 })
        }
        return
      }
      if (drag.kind === 'pen' && point) {
        const draft = penRef.current
        if (!draft) return
        const points = draft.points.slice()
        const current = points[drag.index]
        if (!current) return
        if (drag.space || spaceRef.current) {
          current.x = point.x
          current.y = point.y
        } else {
          current.outgoing = { x: point.x - current.x, y: point.y - current.y }
          current.incoming = { x: current.x - point.x, y: current.y - point.y }
        }
        const next = { ...draft, points }
        penRef.current = next
        setPen(next)
        penApplyRef.current(next)
        return
      }
      if (drag.kind !== 'move' && drag.kind !== 'resize' && drag.kind !== 'radius') return
      const stored = useKoma.getState().design.docs[key]?.doc
      if (!stored) return
      const focus = focusRef.current
      const doc = editingDoc(stored, focus)
      const zoom = viewRef.current.zoom || 1
      const screenDx = (event.clientX - drag.startX) / zoom
      const screenDy = (event.clientY - drag.startY) / zoom
      if (drag.kind === 'radius') {
        const located = locateDesign(doc, drag.id)
        if (!located) return
        const delta = canvasDeltaToSpace(doc, drag.id, screenDx, screenDy)
        const inward = drag.corner === 'tl' ? delta.x + delta.y : drag.corner === 'tr' ? -delta.x + delta.y : drag.corner === 'bl' ? delta.x - delta.y : -delta.x - delta.y
        const limit = Math.min(drag.node.w, drag.node.h) / 2
        const radius = Math.round(Math.min(limit, Math.max(0, drag.radius + inward / 2)))
        const nextNode = { ...drag.node }
        if (radius <= 0) delete nextNode.radius
        else nextNode.radius = radius
        const next = projectDoc(stored, focus, layoutDesign(updateDesignNode(doc, drag.id, () => nextNode)))
        if (serializeDesign(next) === serializeDesign(stored)) return
        if (!drag.remembered) {
          noteRef.current(stored)
          drag.remembered = true
        }
        updateRef.current(tab.root, tab.path, next)
        return
      }
      if (drag.kind === 'resize') {
        const located = locateDesign(doc, drag.id)
        if (!located) return
        const delta = canvasDeltaToSpace(doc, located.parentId, screenDx, screenDy)
        const nextNode = resizeDesignNode(drag.node, drag.handle, delta.x, delta.y, doc.grid, doc.snap)
        const next = projectDoc(stored, focus, layoutDesign(updateDesignNode(doc, drag.id, () => nextNode)))
        if (serializeDesign(next) === serializeDesign(stored)) return
        if (!drag.remembered) {
          noteRef.current(stored)
          drag.remembered = true
        }
        updateRef.current(tab.root, tab.path, next)
        return
      }
      if (Math.hypot(screenDx, screenDy) < 4) return
      drag.moved = true
      let live = doc
      if (drag.alt) {
        const duplicated = duplicateDesignNodes(live, drag.ids, 0, 0, () => mintId('n'))
        if (duplicated) {
          drag.alt = false
          drag.ids = duplicated.ids
          const origins: Record<string, { x: number; y: number }> = {}
          const laid = layoutDesign(duplicated.doc)
          for (const id of duplicated.ids) {
            const row = locateDesign(laid, id)
            if (row) origins[id] = { x: row.node.x, y: row.node.y }
          }
          drag.origins = origins
          const copied = projectDoc(stored, focus, laid)
          if (!drag.remembered) {
            noteRef.current(stored)
            drag.remembered = true
          }
          updateRef.current(tab.root, tab.path, copied)
          selectionRef.current = duplicated.ids
          setSelection(duplicated.ids)
          live = laid
        }
      }
      const flowId = drag.ids.length === 1 ? drag.ids[0] : null
      const flowLocated = flowId ? locateDesign(live, flowId) : null
      const flowParent = flowLocated?.parentId ? findDesignNode(live, flowLocated.parentId) : null
      const flowing = !!(flowId && flowLocated && flowParent?.layout && !flowLocated.node.absolute)
      if (flowing && flowId && flowParent && point && !drag.broke && inFlowBand(live, flowParent.id, point.x, point.y)) {
        setSnapMarks(null)
        setFlowBar(flowBreakBar(live, flowParent.id, flowId, point.x, point.y))
        return
      }
      if (flowing && point && !drag.broke) drag.broke = true
      setFlowBar(null)
      if (!drag.scene) drag.scene = designSnapScene(live, drag.ids)
      const snap = drag.scene
        ? designObjectSnap(drag.scene.moving, drag.scene.targets, screenDx, screenDy, 5 / zoom)
        : { dx: screenDx, dy: screenDy, snappedX: false, snappedY: false, guides: [], measures: [] }
      setSnapMarks(snap.guides.length || snap.measures.length ? { guides: snap.guides, measures: snap.measures } : null)
      let view = live
      let frozen: string | undefined
      for (const id of drag.ids) {
        const located = locateDesign(view, id)
        const origin = drag.origins[id]
        if (!located || !origin) continue
        const parent = located.parentId ? findDesignNode(view, located.parentId) : null
        if (parent?.layout && !located.node.absolute) frozen = id
        const delta = canvasDeltaToSpace(view, located.parentId, snap.dx, snap.dy)
        const x = snap.snappedX ? origin.x + delta.x : snapDesign(origin.x + delta.x, doc.grid, doc.snap)
        const y = snap.snappedY ? origin.y + delta.y : snapDesign(origin.y + delta.y, doc.grid, doc.snap)
        view = updateDesignNode(view, id, (node) => ({ ...node, x, y }))
      }
      const next = projectDoc(stored, focus, layoutDesign(view, frozen))
      if (serializeDesign(next) === serializeDesign(stored)) return
      if (!drag.remembered) {
        noteRef.current(stored)
        drag.remembered = true
      }
      updateRef.current(tab.root, tab.path, next)
    }
    const up = (event: PointerEvent) => {
      const drag = dragRef.current
      dragRef.current = null
      setDragCursor(null)
      setGhost(null)
      setFlowBar(null)
      setSnapMarks(null)
      if (drag?.kind === 'pen') setPenHandle(null)
      if (!drag || drag.kind === 'pan' || drag.kind === 'pen') return
      const stored = useKoma.getState().design.docs[key]?.doc
      if (!stored) return
      const focus = focusRef.current
      const doc = editingDoc(stored, focus)
      if (drag.kind === 'marquee') {
        const box = drawnBox(drag.x0, drag.y0, drag.x1, drag.y1, 1, false)
        if (box.w >= 2 || box.h >= 2) setSelection(selectDesignRect(doc, box.x, box.y, box.w, box.h))
        return
      }
      if (drag.kind === 'draw') {
        commitDrawn(doc, drag)
        return
      }
      if (drag.kind !== 'move') {
        if (drag.kind === 'resize' && drag.remembered) return
        return
      }
      if (!drag.moved) return
      const point = toDoc(event.clientX, event.clientY)
      const dragId = drag.ids[0]
      if (!dragId || !point) return
      const located = locateDesign(doc, dragId)
      let next = doc
      if (located && drag.ids.length === 1 && !(focus && located.parentId == null)) {
        const drop = designDrop(doc, dragId, point.x, point.y)
        const parent = located.parentId ? findDesignNode(doc, located.parentId) : null
        const localPoint = parent ? canvasToContent(doc, parent.id, point.x, point.y) : null
        if (drop.kind === 'stay' && parent?.layout && !located.node.absolute && localPoint && inFlowBand(doc, parent.id, point.x, point.y)) {
          const along = parent.layout === 'row' ? localPoint.x : localPoint.y
          next = reorderDesignNode(doc, dragId, flowInsertIndex(parent, dragId, along))
        } else if (drop.kind === 'move' && !(focus && drop.parentId == null)) {
          next = placeDesignNode(doc, dragId, drop.parentId, snapDesign(drop.x, doc.grid, doc.snap), snapDesign(drop.y, doc.grid, doc.snap))
        }
      }
      const laid = projectDoc(stored, focus, layoutDesign(next))
      if (serializeDesign(laid) === serializeDesign(stored)) return
      if (!drag.remembered) noteRef.current(stored)
      updateRef.current(tab.root, tab.path, laid)
    }
    const onCommit = (event: Event) => {
      const detail = (event as CustomEvent<{ root: string; path: string; doc: DesignDoc }>).detail
      if (!detail || detail.root !== tab.root || detail.path !== tab.path) return
      event.preventDefault()
      const stored = useKoma.getState().design.docs[key]?.doc
      if (!stored || serializeDesign(stored) === serializeDesign(detail.doc)) return
      noteRef.current(stored)
      updateRef.current(tab.root, tab.path, detail.doc)
    }
    window.addEventListener('koma-design-commit', onCommit)
    window.addEventListener('koma-design-restore', onRestore)
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
    window.addEventListener('pointercancel', up)
    return () => {
      window.removeEventListener('koma-design-commit', onCommit)
      window.removeEventListener('koma-design-restore', onRestore)
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      window.removeEventListener('pointercancel', up)
    }
  }, [key, tab.path, tab.root])

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const onWheel = (event: WheelEvent) => {
      event.preventDefault()
      if (!(event.ctrlKey || event.metaKey)) {
        const current = viewRef.current
        applyView({ ...current, panX: current.panX - event.deltaX, panY: current.panY - event.deltaY })
        return
      }
      const factor = event.deltaY < 0 ? 1.1 : 1 / 1.1
      zoomAt(event.clientX, event.clientY, viewRef.current.zoom * factor)
    }
    canvas.addEventListener('wheel', onWheel, { passive: false })
    return () => canvas.removeEventListener('wheel', onWheel)
  }, [file?.loading])

  useEffect(() => {
    if (!active) return
    const onKey = (event: KeyboardEvent) => {
      const typing = !!(event.target as HTMLElement | null)?.closest('input, textarea, [contenteditable="true"]')
      if ((event.metaKey || event.ctrlKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 's') {
        event.preventDefault()
        saveDesign(tab.root, tab.path)
        return
      }
      if (event.code === 'Space') {
        if (typing) return
        event.preventDefault()
        spaceRef.current = event.type === 'keydown'
        setSpaceDown(event.type === 'keydown')
        return
      }
      if (event.type !== 'keydown') return
      if (typing) return
      const meta = event.metaKey || event.ctrlKey
      if (meta && !event.altKey && event.key.toLowerCase() === 'z') {
        event.preventDefault()
        if (event.shiftKey) redoRef.current()
        else undoRef.current()
        return
      }
      if (meta && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'y') {
        event.preventDefault()
        redoRef.current()
        return
      }
      if (meta && event.altKey && !event.shiftKey && event.key.toLowerCase() === 'c') {
        event.preventDefault()
        commandsRef.current?.copyStyle()
        return
      }
      if (meta && event.altKey && !event.shiftKey && event.key.toLowerCase() === 'v') {
        event.preventDefault()
        commandsRef.current?.pasteStyle()
        return
      }
      if (meta && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'c') {
        event.preventDefault()
        commandsRef.current?.copy()
        return
      }
      if (meta && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'v') {
        event.preventDefault()
        commandsRef.current?.paste()
        return
      }
      if (meta && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'd') {
        event.preventDefault()
        commandsRef.current?.duplicate()
        return
      }
      if (meta && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'a') {
        event.preventDefault()
        commandsRef.current?.selectAll()
        return
      }
      if (meta && !event.altKey && event.key.toLowerCase() === 'g') {
        event.preventDefault()
        if (event.shiftKey) commandsRef.current?.unwrap()
        else commandsRef.current?.wrap('group')
        return
      }
      if (meta && event.altKey && !event.shiftKey && event.key.toLowerCase() === 'g') {
        event.preventDefault()
        commandsRef.current?.wrap('frame')
        return
      }
      if (meta && event.shiftKey && event.key.toLowerCase() === 'h') {
        event.preventDefault()
        commandsRef.current?.hide()
        return
      }
      if (event.key === ']' || event.key === '[') {
        event.preventDefault()
        const order: DesignOrder = event.key === ']' ? (meta ? 'front' : 'forward') : meta ? 'back' : 'backward'
        commandsRef.current?.order(order)
        return
      }
      if (!meta && event.shiftKey && event.key.toLowerCase() === 'h') {
        event.preventDefault()
        commandsRef.current?.flip('x')
        return
      }
      if (!meta && event.shiftKey && event.key.toLowerCase() === 'v') {
        event.preventDefault()
        commandsRef.current?.flip('y')
        return
      }
      if (event.key === 'Delete' || event.key === 'Backspace') {
        event.preventDefault()
        commandsRef.current?.remove()
        return
      }
      if (!meta && !event.altKey && event.shiftKey && (event.code === 'Digit1' || event.code === 'Digit2')) {
        event.preventDefault()
        commandsRef.current?.zoom(event.code === 'Digit1' ? 'all' : 'selection')
        return
      }
      if (!meta && (event.key === 'ArrowLeft' || event.key === 'ArrowRight' || event.key === 'ArrowUp' || event.key === 'ArrowDown')) {
        event.preventDefault()
        const step = event.shiftKey ? 10 : 1
        const dx = event.key === 'ArrowLeft' ? -step : event.key === 'ArrowRight' ? step : 0
        const dy = event.key === 'ArrowUp' ? -step : event.key === 'ArrowDown' ? step : 0
        commandsRef.current?.nudge(dx, dy, event.altKey)
        return
      }
      if (!meta && !event.shiftKey && !event.altKey) {
        const tools: Partial<Record<string, Tool>> = { v: 'select', r: 'rect', o: 'ellipse', l: 'line', f: 'frame', t: 'text', p: 'pen', h: 'pan' }
        const next = tools[event.key.toLowerCase()]
        if (next) {
          event.preventDefault()
          setTool(next)
          return
        }
      }
      if (event.key === 'Escape') {
        setEditing(null)
        setPen(null)
        setPenHover(null)
        setPenHandle(null)
        penRef.current = null
        penNoted.current = false
        if (toolRef.current === 'pen') setTool('select')
        else stepOutRef.current()
      }
    }
    window.addEventListener('keydown', onKey)
    window.addEventListener('keyup', onKey)
    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('keyup', onKey)
    }
  }, [active, file?.loading, key, saveDesign, selection, tab.path, tab.root])

  const placeAt = (kind: DrawShape, point: { x: number; y: number }) => {
    const stored = useKoma.getState().design.docs[key]?.doc
    if (!stored) return
    const doc = editingDoc(stored, focusRef.current)
    const parent = kind === 'frame' || focusRef.current ? frameAtPoint(doc, point.x, point.y, '') : null
    const node = createNode(kind, mintId(kind[0]), 0, 0)
    let next = doc
    if (parent) {
      const local = canvasToContent(doc, parent, point.x, point.y)
      if (!local) return
      node.x = snapDesign(local.x, doc.grid, doc.snap)
      node.y = snapDesign(local.y, doc.grid, doc.snap)
      next = insertDesignNode(doc, parent, node)
    } else if (focusRef.current && kind !== 'frame') {
      const screen = createNode('frame', mintId('f'), snapDesign(point.x, doc.grid, doc.snap), snapDesign(point.y, doc.grid, doc.snap))
      node.x = 16
      node.y = 16
      screen.children = [node]
      next = insertDesignNode(doc, null, screen)
    } else {
      node.x = snapDesign(point.x, doc.grid, doc.snap)
      node.y = snapDesign(point.y, doc.grid, doc.snap)
      next = insertDesignNode(doc, null, node)
    }
    commit(next)
    setSelection([node.id])
    setEditing(null)
    setPropsOpen(true)
    setTool('select')
  }

  const applyPen = (draft: PenDraft, closed = false) => {
    const stored = useKoma.getState().design.docs[key]?.doc
    if (!stored) return
    const focus = focusRef.current
    const doc = editingDoc(stored, focus)
    const points = draft.points.map((point) => {
      const at = canvasToContent(doc, draft.parentId, point.x, point.y)
      return {
        x: at?.x ?? point.x,
        y: at?.y ?? point.y,
        incoming: canvasDeltaToSpace(doc, draft.parentId, point.incoming.x, point.incoming.y),
        outgoing: canvasDeltaToSpace(doc, draft.parentId, point.outgoing.x, point.outgoing.y),
      }
    })
    const node = nodeFromPen(draft.id, points, closed)
    if (!node) return
    const existing = locateDesign(doc, draft.id)
    const view = existing
      ? updateDesignNode(doc, draft.id, (current) => ({
          ...current,
          x: node.x,
          y: node.y,
          w: node.w,
          h: node.h,
          vector: node.vector,
          fill: closed ? (current.fill && current.fill !== 'none' ? current.fill : '#d9d9d9') : 'none',
          stroke: current.stroke && current.stroke !== 'none' ? current.stroke : '#1c1c1c',
          strokeWidth: current.strokeWidth ?? 2,
        }))
      : insertDesignNode(doc, draft.parentId, node)
    const next = projectDoc(stored, focus, layoutDesign(view))
    if (serializeDesign(next) === serializeDesign(stored)) return
    if (!penNoted.current) {
      note(stored)
      penNoted.current = true
    }
    updateDesign(tab.root, tab.path, next)
    setSelection([draft.id])
  }
  penApplyRef.current = applyPen

  const viewDoc = () => {
    const stored = useKoma.getState().design.docs[key]?.doc
    if (!stored) return null
    return { stored, doc: editingDoc(stored, focusRef.current) }
  }

  const mutate = (next: DesignDoc | null) => {
    if (!next) return
    commit(next)
  }

  commandsRef.current = {
    copy: () => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      const rows = ids.map((id) => locateDesign(open.doc, id)).filter((row) => row != null)
      if (!rows.length || rows.some((row) => row.parentId !== rows[0].parentId)) return
      copiedShape = { nodes: rows.map((row) => row.node), parentId: rows[0].parentId }
    },
    paste: (at) => {
      if (!copiedShape?.nodes.length) return
      const open = viewDoc()
      if (!open || file?.loading) return
      const step = open.doc.snap && open.doc.grid > 0 ? open.doc.grid : 10
      const parentId = at
        ? frameAtPoint(open.doc, at.x, at.y, '')
        : copiedShape.parentId && locateDesign(open.doc, copiedShape.parentId)
          ? copiedShape.parentId
          : null
      let next = open.doc
      const pasted: DesignNode[] = []
      for (const source of copiedShape.nodes) {
        const node = copyTree(source, () => mintId('n'), step, step)
        if (at && parentId) {
          const local = canvasToContent(open.doc, parentId, at.x, at.y)
          if (local) {
            node.x = snapDesign(local.x, open.doc.grid, open.doc.snap)
            node.y = snapDesign(local.y, open.doc.grid, open.doc.snap)
          }
        } else if (at) {
          node.x = snapDesign(at.x, open.doc.grid, open.doc.snap)
          node.y = snapDesign(at.y, open.doc.grid, open.doc.snap)
        }
        if (parentId) next = insertDesignNode(next, parentId, node)
        else if (focusRef.current && node.kind !== 'frame' && node.kind !== 'group' && next.screens[0]) next = insertDesignNode(next, next.screens[0].id, node)
        else if (focusRef.current && node.kind !== 'frame' && node.kind !== 'group') {
          const screen = createNode('frame', mintId('f'), node.x, node.y)
          node.x = 16
          node.y = 16
          screen.children = [node]
          next = insertDesignNode(next, null, screen)
        } else next = insertDesignNode(next, null, node)
        pasted.push(node)
      }
      copiedShape = { nodes: pasted, parentId }
      commit(next)
      setSelection(pasted.map((node) => node.id))
      setPropsOpen(true)
    },
    duplicate: () => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length || file?.loading) return
      const step = open.doc.snap && open.doc.grid > 0 ? open.doc.grid : 10
      const duplicated = duplicateDesignNodes(open.doc, ids, step, step, () => mintId('n'))
      if (!duplicated) return
      commit(duplicated.doc)
      setSelection(duplicated.ids)
      setPropsOpen(true)
    },
    copyStyle: () => {
      const open = viewDoc()
      const id = selectionRef.current[selectionRef.current.length - 1]
      if (!open || !id) return
      const located = locateDesign(open.doc, id)
      if (!located) return
      copiedStyle = designStyle(located.node)
    },
    pasteStyle: () => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length || !copiedStyle) return
      commit(applyDesignStyle(open.doc, ids, copiedStyle))
    },
    selectAll: () => {
      const open = viewDoc()
      if (!open) return
      setSelection(selectAllDesign(open.doc, selectionRef.current))
      setPropsOpen(true)
    },
    nudge: (dx, dy, duplicate) => {
      const open = viewDoc()
      if (!open || file?.loading) return
      let view = open.doc
      let ids = selectionRef.current
      if (!ids.length) return
      if (duplicate) {
        const copied = duplicateDesignNodes(view, ids, 0, 0, () => mintId('n'))
        if (!copied) return
        view = copied.doc
        ids = copied.ids
        selectionRef.current = ids
        setSelection(ids)
      }
      view = nudgeDesignNodes(view, ids, dx, dy)
      const stored = useKoma.getState().design.docs[key]?.doc
      if (!stored) return
      const next = projectDoc(stored, focusRef.current, layoutDesign(view))
      if (serializeDesign(stored) === serializeDesign(next)) return
      if (!nudgeOpenRef.current) {
        note(stored)
        nudgeOpenRef.current = true
      }
      if (nudgeTimerRef.current) clearTimeout(nudgeTimerRef.current)
      nudgeTimerRef.current = setTimeout(() => {
        nudgeTimerRef.current = null
        nudgeOpenRef.current = false
      }, 300)
      updateDesign(tab.root, tab.path, next)
    },
    zoom: (scope) => {
      zoomTo(scope === 'all' ? null : selectionRef.current)
    },
    remove: () => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      const next = ids.reduce((doc, id) => deleteDesignNode(doc, id), open.doc)
      commit(next)
      setSelection([])
      setEditing(null)
    },
    wrap: (kind) => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      if (focusRef.current && ids.some((id) => locateDesign(open.doc, id)?.parentId == null)) return
      const id = mintId(kind[0])
      const next = wrapDesignNodes(open.doc, ids, kind, id)
      if (!next) return
      commit(next)
      setSelection([id])
    },
    unwrap: () => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      let next = open.doc
      const released: string[] = []
      for (const id of ids) {
        const children = findDesignNode(next, id)?.children?.map((child) => child.id) ?? []
        const unwrapped = unwrapDesignNode(next, id)
        if (!unwrapped) continue
        next = unwrapped
        released.push(...children)
      }
      if (!released.length || serializeDesign(next) === serializeDesign(open.doc)) return
      commit(next)
      setSelection(released)
    },
    order: (order) => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      const ranked = ids.flatMap((id) => {
        const row = locateDesign(open.doc, id)
        if (!row) return []
        const siblings = row.parentId == null ? open.doc.screens : findDesignNode(open.doc, row.parentId)?.children ?? []
        const index = siblings.findIndex((item) => item.id === id)
        return index < 0 ? [] : [{ id, index }]
      }).sort((a, b) => a.index - b.index)
      const sequence = order === 'back' || order === 'backward' ? ranked.reverse() : ranked
      mutate(sequence.reduce((current, row) => orderDesignNode(current, row.id, order), open.doc))
    },
    flip: (axis) => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      mutate(ids.reduce((doc, id) => flipDesignNode(doc, id, axis), open.doc))
    },
    hide: () => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      const show = ids.some((id) => locateDesign(open.doc, id)?.node.visible === false)
      mutate(ids.reduce((doc, id) => setDesignVisible(doc, id, show), open.doc))
    },
    lock: () => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      const unlock = ids.every((id) => locateDesign(open.doc, id)?.node.locked)
      mutate(ids.reduce((doc, id) => setDesignLocked(doc, id, !unlock), open.doc))
    },
    auto: () => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      const next = autoLayoutDesign(open.doc, ids, mintId('f'))
      if (next) commit(next)
    },
    component: () => {
      const stored = useKoma.getState().design.docs[key]?.doc
      const id = selectionRef.current.length === 1 ? selectionRef.current[0] : null
      if (!stored || !id || focusRef.current) return
      const componentId = mintId('c')
      const next = createComponentFromFrame(stored, id, componentId, () => mintId('n'))
      if (!next) return
      commitStored(next)
      setFocusId(componentId)
      setSelection([])
    },
    select: (id) => {
      setSelection([id])
      setPropsOpen(true)
    },
  }

  layerOpsRef.current = (action) => {
    const open = viewDoc()
    if (!open) return
    if (action.op === 'select') {
      const ids = selectionRef.current
      setSelection(action.shift ? (ids.includes(action.id) ? ids.filter((item) => item !== action.id) : [...ids, action.id]) : [action.id])
      setPropsOpen(true)
      setEditing(null)
      return
    }
    if (action.op === 'rename') {
      const trimmed = action.name.trim()
      commit(updateDesignNode(open.doc, action.id, (node) => {
        const next = { ...node }
        if (trimmed) next.name = trimmed
        else delete next.name
        return next
      }))
      return
    }
    if (action.op === 'visible') {
      commit(setDesignVisible(open.doc, action.id, action.visible))
      return
    }
    if (action.op === 'locked') {
      commit(setDesignLocked(open.doc, action.id, action.locked))
      return
    }
    if (action.op === 'move') {
      if (focusRef.current && action.parentId == null && locateDesign(open.doc, action.id)?.parentId != null) return
      if (!canLeaveParent(open.doc, action.id, action.parentId)) return
      commit(moveDesignNode(open.doc, action.id, action.parentId, action.index))
      return
    }
    if (!selectionRef.current.includes(action.id)) setSelection([action.id])
    const origin = nodeOrigin(open.doc, action.id)
    setMenu({ x: action.x, y: action.y, canvasX: origin?.x ?? 0, canvasY: origin?.y ?? 0 })
  }

  const beginPen = (point: { x: number; y: number }) => {
    const open = viewDoc()
    if (!open) return
    const draft = penRef.current
    const zero = { x: 0, y: 0 }
    if (draft && draft.points.length >= 3) {
      const first = draft.points[0]
      if (Math.hypot(first.x - point.x, first.y - point.y) <= 8) {
        applyPen(draft, true)
        penRef.current = null
        setPen(null)
        setPenHover(null)
        setPenHandle(null)
        penNoted.current = false
        setTool('select')
        return
      }
    }
    if (!draft) {
      penNoted.current = false
      const parentId = focusRef.current ? frameAtPoint(open.doc, point.x, point.y, '') ?? open.doc.screens.find((screen) => screen.kind === 'frame' || screen.kind === 'group')?.id ?? null : null
      const next = { id: mintId('v'), parentId, points: [{ x: point.x, y: point.y, incoming: zero, outgoing: zero }] }
      penRef.current = next
      setPen(next)
      setPenHandle(0)
      applyPen(next)
      dragRef.current = { kind: 'pen', index: 0, space: spaceRef.current }
      return
    }
    const points = [...draft.points, { x: point.x, y: point.y, incoming: zero, outgoing: zero }]
    const next = { ...draft, points }
    penRef.current = next
    setPen(next)
    setPenHandle(points.length - 1)
    applyPen(next)
    dragRef.current = { kind: 'pen', index: points.length - 1, space: spaceRef.current }
  }

  const startPan = (event: ReactPointerEvent | PointerEvent) => {
    dragRef.current = { kind: 'pan', lastX: event.clientX, lastY: event.clientY }
    setDragCursor('grabbing')
  }

  const onCanvasPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button === 1 || spaceRef.current || tool === 'pan') {
      event.preventDefault()
      startPan(event)
      return
    }
    if (event.button !== 0) return
    const point = toDoc(event.clientX, event.clientY)
    if (!point || !file) return
    if (tool === 'pen') {
      event.preventDefault()
      beginPen(point)
      return
    }
    if (tool === 'frame' || tool === 'rect' || tool === 'ellipse' || tool === 'line' || tool === 'text') {
      event.preventDefault()
      const stored = useKoma.getState().design.docs[key]?.doc
      const doc = stored ? editingDoc(stored, focusRef.current) : null
      dragRef.current = {
        kind: 'draw',
        shape: tool,
        cx: point.x,
        cy: point.y,
        x0: point.x,
        y0: point.y,
        x1: point.x,
        y1: point.y,
        parentId: doc && (tool === 'frame' || focusRef.current) ? frameAtPoint(doc, point.x, point.y, '') : null,
      }
      return
    }
    setSelection([])
    dragRef.current = { kind: 'marquee', x0: point.x, y0: point.y, x1: point.x, y1: point.y }
  }

  const placeInstance = (componentId: string, point: { x: number; y: number }) => {
    focusRef.current = null
    setFocusId(null)
    const stored = useKoma.getState().design.docs[key]?.doc
    if (!stored) return
    const node = makeInstance(stored, componentId, mintId('i'), 0, 0)
    if (!node) return
    const parent = frameAtPoint(stored, point.x, point.y, '')
    let next = stored
    if (parent) {
      const local = canvasToContent(stored, parent, point.x, point.y)
      if (!local) return
      node.x = snapDesign(local.x, stored.grid, stored.snap)
      node.y = snapDesign(local.y, stored.grid, stored.snap)
      next = insertDesignNode(stored, parent, node)
    } else if (focusRef.current) {
      const screen = createNode('frame', mintId('f'), snapDesign(point.x, stored.grid, stored.snap), snapDesign(point.y, stored.grid, stored.snap))
      node.x = 16
      node.y = 16
      screen.children = [node]
      next = insertDesignNode(stored, null, screen)
    } else {
      node.x = snapDesign(point.x, stored.grid, stored.snap)
      node.y = snapDesign(point.y, stored.grid, stored.snap)
      next = insertDesignNode(stored, null, node)
    }
    commitStored(next)
    setSelection([node.id])
    setEditing(null)
    setPropsOpen(true)
    setTool('select')
  }

  const onDrop = (event: React.DragEvent<HTMLDivElement>) => {
    event.preventDefault()
    const componentId = event.dataTransfer.getData(COMPONENT_MIME)
    const point = toDoc(event.clientX, event.clientY)
    if (!point) return
    if (componentId) {
      placeInstance(componentId, point)
      return
    }
    const kind = event.dataTransfer.getData(DESIGN_MIME)
    if (kind !== 'frame' && kind !== 'rect' && kind !== 'ellipse' && kind !== 'line' && kind !== 'text') return
    placeAt(kind, point)
  }

  if (!file) {
    return <div className="flex h-full items-center justify-center text-[12px] text-koma-dim">Loading…</div>
  }

  const storedDoc = file.doc
  const doc = editingDoc(storedDoc, focusId)
  const selectedNodes = selection.flatMap((id) => {
    const hit = locateDesign(doc, id)
    return hit ? [hit.node] : []
  })
  const selectedId = selectedNodes[selectedNodes.length - 1]?.id ?? null
  const located = selectedId ? locateDesign(doc, selectedId) : null
  const selected = located?.node ?? null
  const multi = selectedNodes.length > 1
  const parentNode = located?.parentId ? findDesignNode(doc, located.parentId) : null
  const sizeModes = !multi && (parentNode?.layout === 'row' || parentNode?.layout === 'column')
  const everyParent = selectedNodes.length > 0 && selectedNodes.every((node) => locateDesign(doc, node.id)?.parentId != null)
  const focusedComponent = focusId ? storedDoc.components.find((component) => component.id === focusId) ?? null : null
  const selectedVariant = focusedComponent && located?.parentId == null
    ? focusedComponent.variants.find((variant) => variant.node.id === selected?.id) ?? null
    : null
  const chatQuery: DesignQuery | null = focusedComponent
    ? { component: focusedComponent.id, variant: selectedVariant?.props }
    : selected?.kind === 'instance' && selected.component
      ? { component: selected.component, variant: selected.variant }
      : selected && located?.parentId == null
        ? { screen: selected.id }
        : null
  const chain = selectedId ? designPath(doc, selectedId) ?? [] : []
  const stepOut = () => {
    if (selection.length) {
      const id = selection[selection.length - 1]
      const row = id ? locateDesign(doc, id) : null
      if (row?.parentId) {
        setSelection([row.parentId])
        return
      }
      setSelection([])
      return
    }
    if (focusId) {
      setFocusId(null)
      setSelection([])
    }
  }
  stepOutRef.current = stepOut
  const sendChat = () => {
    if (!chatQuery) return
    const text = designChatNote(storedDoc, chatQuery)
    const node = designQueryNode(storedDoc, chatQuery)
    if (!text || !node) return
    const png = designPngBase64(storedDoc, node)
    const title = designChatTitle(storedDoc, chatQuery)
    if (png) {
      const stem = title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 40) || 'design'
      useKoma.getState().req({ r: 'AttachFile', name: `${stem}.png`, bytesB64: png, mime: 'image/png' })
    }
    useKoma.getState().addDesignToChat({ title, text })
  }
  const status = file.saving
    ? 'Saving…'
    : file.loading && file.savedText == null
      ? 'Loading…'
      : file.error && !file.dirty
        ? file.error
        : file.dirty
          ? 'Modified'
          : 'Saved'
  const canSave = file.dirty && !file.saving && !file.loading
  const canRevert = file.dirty && !!file.savedText && !file.saving && !file.loading
  const grid = doc.grid > 0 ? doc.grid : 8
  void rev

  const patchSelected = (fn: (node: DesignNode) => DesignNode, noteOnce = false) => {
    if (!selection.length) return
    const stored = useKoma.getState().design.docs[key]?.doc
    if (!stored) return
    let view = editingDoc(stored, focusRef.current)
    for (const id of selection) view = updateDesignNode(view, id, fn)
    const next = projectDoc(stored, focusRef.current, layoutDesign(view))
    if (serializeDesign(next) === serializeDesign(stored)) return
    if (noteOnce) {
      if (!labelNoted.current) {
        note(stored)
        labelNoted.current = true
      }
      updateDesign(tab.root, tab.path, next)
      return
    }
    commit(view)
  }

  return (
    <div className="flex h-full min-h-0 min-w-0 bg-koma-bg text-koma-fg" onContextMenu={(event) => event.preventDefault()}>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <EditorChrome
          path={tab.path}
          icon={<Frame size={13} className="flex-none text-koma-dim" />}
          status={status}
          canSave={canSave}
          canRevert={canRevert}
          saving={file.saving}
          canUndo={pastRef.current.length > 0}
          canRedo={futureRef.current.length > 0}
          onHistory={() => showCodingHistory(tab.root, tab.path)}
          onUndo={undo}
          onRedo={redo}
          onSave={() => saveDesign(tab.root, tab.path)}
          onRevert={revert}
          trailing={
            <button
              type="button"
              title={propsOpen ? 'Hide properties' : 'Properties'}
              aria-label={propsOpen ? 'Hide properties' : 'Properties'}
              aria-pressed={propsOpen}
              onClick={() => setPropsOpen((open) => !open)}
              className={`flex h-6 w-6 flex-none items-center justify-center rounded hover:bg-koma-hover hover:text-koma-fg ${propsOpen ? 'text-koma-fg' : 'text-koma-dim'}`}
            >
              {propsOpen ? <PanelRightClose size={13} /> : <PanelRightOpen size={13} />}
            </button>
          }
        />
        {file.error ? <div className="flex-none border-b border-koma-border px-3 py-1 text-[12px] text-koma-error">{file.error}</div> : null}
        <div className="flex min-h-0 flex-1">
        <div
          ref={canvasRef}
          className={`relative min-h-0 min-w-0 flex-1 overflow-hidden ${spaceDown || tool === 'pan' ? 'cursor-grab' : ''}`}
          onPointerDown={onCanvasPointerDown}
          onContextMenu={(event) => {
            event.preventDefault()
            const point = toDoc(event.clientX, event.clientY)
            if (!point) return
            setMenu({ x: event.clientX, y: event.clientY, canvasX: point.x, canvasY: point.y })
          }}
          onDragOver={(event) => event.preventDefault()}
          onDrop={onDrop}
          style={{ backgroundColor: '#e6e8ed', ...canvasBackdrop(doc.snap, grid, view) }}
        >
          <DesignRulers panX={view.panX} panY={view.panY} zoom={view.zoom} />
          <div className="pointer-events-none absolute left-0 top-0" style={{ transform: `translate(${view.panX}px, ${view.panY}px) scale(${view.zoom})`, transformOrigin: '0 0' }}>
            {flowBar ? (
              <div className="pointer-events-none absolute z-10" style={{ left: flowBar.x, top: flowBar.y, width: flowBar.w, height: flowBar.h, background: SELECTION }} />
            ) : null}
            {snapMarks?.guides.map((guide, index) => (
              <div
                key={`guide-${guide.axis}-${index}`}
                className="pointer-events-none absolute z-10"
                style={guide.axis === 'x'
                  ? { left: guide.at, top: guide.from, width: 1 / Math.max(view.zoom, 0.25), height: Math.max(guide.to - guide.from, 1), background: SELECTION }
                  : { left: guide.from, top: guide.at, width: Math.max(guide.to - guide.from, 1), height: 1 / Math.max(view.zoom, 0.25), background: SELECTION }}
              />
            ))}
            {snapMarks?.measures.map((measure, index) => (
              <div
                key={`measure-${measure.axis}-${index}`}
                className="pointer-events-none absolute z-10"
                style={{
                  left: measure.x,
                  top: measure.y,
                  width: measure.axis === 'x' ? measure.length : 1 / Math.max(view.zoom, 0.25),
                  height: measure.axis === 'y' ? measure.length : 1 / Math.max(view.zoom, 0.25),
                  background: SELECTION,
                }}
              >
                <span
                  className="absolute whitespace-nowrap rounded px-1"
                  style={{
                    left: measure.axis === 'x' ? measure.length / 2 : 0,
                    top: measure.axis === 'y' ? measure.length / 2 : 0,
                    transform: 'translate(-50%, -50%)',
                    fontSize: 11 / Math.max(view.zoom, 0.25),
                    lineHeight: 1.2,
                    color: SELECTION,
                    background: '#e6e8ed',
                  }}
                >
                  {measure.label}
                </span>
              </div>
            ))}
            {doc.screens.map((screen) => (
              <DesignNodeView
                key={screen.id}
                doc={doc}
                node={screen}
                zoom={view.zoom}
                selectedIds={selection}
                editing={editing}
                dragCursor={dragCursor}
                onCorner={(id, corner, event) => {
                  event.preventDefault()
                  event.stopPropagation()
                  setSelection([id])
                  const storedNow = useKoma.getState().design.docs[key]?.doc
                  const locatedNow = locateDesign(storedNow ? editingDoc(storedNow, focusRef.current) : doc, id)
                  if (!locatedNow) return
                  const raw = locatedNow.node.radius
                  const radius = typeof raw === 'number' ? raw : Number(resolveRef(doc, typeof raw === 'string' ? raw : '')) || 0
                  dragRef.current = {
                    kind: 'radius',
                    id,
                    corner,
                    startX: event.clientX,
                    startY: event.clientY,
                    radius,
                    node: { ...locatedNow.node },
                    remembered: false,
                  }
                }}
                onMenu={(id, clientX, clientY) => {
                  const point = toDoc(clientX, clientY)
                  if (!selection.includes(id)) setSelection([id])
                  if (point) setMenu({ x: clientX, y: clientY, canvasX: point.x, canvasY: point.y })
                }}
                onSelect={(id, event) => {
                  if (event.button === 1 || spaceRef.current || toolRef.current === 'pan') {
                    event.preventDefault()
                    event.stopPropagation()
                    startPan(event)
                    return
                  }
                  if (event.button !== 0 || toolRef.current !== 'select') return
                  event.preventDefault()
                  event.stopPropagation()
                  const storedNow = useKoma.getState().design.docs[key]?.doc
                  const current = storedNow ? editingDoc(storedNow, focusRef.current) : doc
                  const point = toDoc(event.clientX, event.clientY)
                  const target = point ? selectDesignHit(current, point.x, point.y, selectionRef.current, event.detail >= 2) ?? id : id
                  const previous = selectionRef.current
                  const ids = event.shiftKey ? previous.includes(target) ? previous.filter((item) => item !== target) : [...previous, target] : previous.includes(target) ? previous : [target]
                  selectionRef.current = ids
                  setSelection(ids)
                  setPropsOpen(true)
                  if (event.shiftKey) return
                  const origins: Record<string, { x: number; y: number }> = {}
                  for (const item of ids) {
                    const row = locateDesign(current, item)
                    if (row) origins[item] = { x: row.node.x, y: row.node.y }
                  }
                  if (locateDesign(current, target)?.node.locked) return
                  dragRef.current = {
                    kind: 'move',
                    ids,
                    startX: event.clientX,
                    startY: event.clientY,
                    origins,
                    remembered: false,
                    moved: false,
                    broke: false,
                    alt: event.altKey,
                    scene: null,
                  }
                  setDragCursor('grabbing')
                }}
                onResize={(id, handle, event) => {
                  event.preventDefault()
                  event.stopPropagation()
                  setSelection([id])
                  const storedNow = useKoma.getState().design.docs[key]?.doc
                  const located = locateDesign(storedNow ? editingDoc(storedNow, focusRef.current) : doc, id)
                  if (!located || located.node.locked) return
                  const node = { ...located.node }
                  if (handle.includes('w') || handle.includes('e')) delete node.wMode
                  if (handle.includes('n') || handle.includes('s')) delete node.hMode
                  dragRef.current = {
                    kind: 'resize',
                    id,
                    handle,
                    startX: event.clientX,
                    startY: event.clientY,
                    node,
                    remembered: false,
                  }
                  setDragCursor(HANDLES.find((item) => item.id === handle)?.cursor ?? 'grabbing')
                }}
                onEdit={(id) => {
                  setSelection([id])
                  setEditing(id)
                  setPropsOpen(true)
                  labelNoted.current = false
                }}
                onText={(id, text) => {
                  const stored = useKoma.getState().design.docs[key]?.doc
                  if (!stored) return
                  const view = updateDesignNode(editingDoc(stored, focusRef.current), id, (node) => ({ ...node, text }))
                  const next = projectDoc(stored, focusRef.current, layoutDesign(view))
                  if (serializeDesign(next) === serializeDesign(stored)) return
                  if (!labelNoted.current) {
                    note(stored)
                    labelNoted.current = true
                  }
                  updateDesign(tab.root, tab.path, next)
                }}
                onTextBlur={() => {
                  labelNoted.current = false
                  setEditing(null)
                }}
              />
            ))}
          {ghost ? (
            <div
              className="pointer-events-none absolute"
              style={{
                left: ghost.x,
                top: ghost.y,
                width: Math.max(ghost.w, 1),
                height: Math.max(ghost.h, 1),
                background: ghost.kind === 'marquee' ? 'color-mix(in srgb, var(--koma-accent) 12%, transparent)' : ghost.shape === 'frame' ? '#ffffff' : ghost.shape === 'rect' || ghost.shape === 'ellipse' ? SHAPE_FILL : 'transparent',
                border: `${1 / Math.max(view.zoom, 0.25)}px solid ${SELECTION}`,
                borderRadius: ghost.kind === 'shape' && ghost.shape === 'ellipse' ? '50%' : undefined,
                transform: ghost.kind === 'shape' && ghost.rotation ? `rotate(${ghost.rotation}deg)` : undefined,
              }}
            >
              {ghost.kind === 'shape' ? (
                <span className="absolute left-0 whitespace-nowrap" style={{ top: '100%', marginTop: 6 / Math.max(view.zoom, 0.25), fontSize: 11 / Math.max(view.zoom, 0.25), color: SELECTION }}>
                  {ghost.shape === 'line' ? Math.round(ghost.w) : `${Math.round(ghost.w)} × ${Math.round(ghost.h)}`}
                </span>
              ) : null}
            </div>
          ) : null}
          {pen ? <PenOverlay draft={pen} hover={penHandle == null ? penHover : null} zoom={view.zoom} /> : null}
          </div>
          {doc.screens.length === 0 && !file.loading ? (
            <div className="pointer-events-none absolute inset-0 flex items-center justify-center text-[12px] text-koma-fg opacity-35">
              Drag a frame onto the canvas
            </div>
          ) : null}
        </div>
        </div>
        <div className="flex h-8 flex-none items-center gap-1 border-t border-koma-border bg-koma-panel px-2">
          <ToolButton label="Move (V)" selected={tool === 'select'} onClick={() => setTool('select')}>
            <MousePointer2 size={15} strokeWidth={2.25} />
          </ToolButton>
          <ToolButton label="Frame (F)" selected={tool === 'frame'} onClick={() => setTool('frame')}>
            <Frame size={15} strokeWidth={2.25} />
          </ToolButton>
          <ToolButton label="Rectangle (R)" selected={tool === 'rect'} onClick={() => setTool('rect')}>
            <Square size={15} strokeWidth={2.25} />
          </ToolButton>
          <ToolButton label="Ellipse (O)" selected={tool === 'ellipse'} onClick={() => setTool('ellipse')}>
            <Circle size={15} strokeWidth={2.25} />
          </ToolButton>
          <ToolButton label="Line (L)" selected={tool === 'line'} onClick={() => setTool('line')}>
            <Minus size={15} strokeWidth={2.25} />
          </ToolButton>
          <ToolButton label="Pen (P)" selected={tool === 'pen'} onClick={() => setTool('pen')}>
            <PenTool size={15} strokeWidth={2.25} />
          </ToolButton>
          <ToolButton label="Text (T)" selected={tool === 'text'} onClick={() => setTool('text')}>
            <Type size={15} strokeWidth={2.25} />
          </ToolButton>
          <ToolButton label="Hand (H)" selected={tool === 'pan'} onClick={() => setTool('pan')}>
            <Hand size={15} strokeWidth={2.25} />
          </ToolButton>
          {focusedComponent || chain.length ? (
            <div className="flex min-w-0 items-center gap-0.5 overflow-hidden">
              {focusedComponent ? (
                <button
                  type="button"
                  onClick={() => {
                    setFocusId(null)
                    setSelection([])
                  }}
                  className="h-6 flex-none rounded px-1.5 text-[12px] text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
                >
                  Screens
                </button>
              ) : null}
              {chain.map((node, index) => (
                <span key={node.id} className="flex min-w-0 items-center">
                  {focusedComponent || index > 0 ? <span className="px-0.5 text-[12px] text-koma-dim">/</span> : null}
                  <button
                    type="button"
                    onClick={() => setSelection([node.id])}
                    className="h-6 max-w-24 truncate rounded px-1.5 text-[12px] text-koma-fg hover:bg-koma-hover"
                  >
                    {designLayerName(node)}
                  </button>
                </span>
              ))}
              <button
                type="button"
                aria-label="Clear"
                title="Clear"
                onClick={stepOut}
                className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
              >
                <X size={13} />
              </button>
            </div>
          ) : null}
          {chatQuery ? (
            <button
              type="button"
              onClick={sendChat}
              className="h-6 rounded px-2 text-[12px] text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
            >
              Add to chat
            </button>
          ) : null}
          <div className="ml-auto flex items-center gap-1">
            <ToolButton label="Zoom out" selected={false} onClick={() => {
              const rect = canvasRef.current?.getBoundingClientRect()
              if (!rect) return
              zoomAt(rect.left + rect.width / 2, rect.top + rect.height / 2, view.zoom / 1.1)
            }}>
              <Minus size={15} strokeWidth={2.25} />
            </ToolButton>
            <span className="w-14 text-center text-[11px] text-koma-dim">{Math.round(view.zoom * 100)}%</span>
            <button type="button" title="Zoom to fit (⇧1)" className="h-6 rounded px-1.5 text-[11px] text-koma-dim hover:bg-koma-hover hover:text-koma-fg" onClick={() => zoomTo(null)}>Fit</button>
            <button type="button" title="Zoom to selection (⇧2)" className="h-6 rounded px-1.5 text-[11px] text-koma-dim hover:bg-koma-hover hover:text-koma-fg" onClick={() => zoomTo(selection)}>Selection</button>
            <ToolButton label="Zoom in" selected={false} onClick={() => {
              const rect = canvasRef.current?.getBoundingClientRect()
              if (!rect) return
              zoomAt(rect.left + rect.width / 2, rect.top + rect.height / 2, view.zoom * 1.1)
            }}>
              <Plus size={15} strokeWidth={2.25} />
            </ToolButton>
            {doc.modes.map((mode) => (
              <button
                key={mode}
                type="button"
                aria-pressed={doc.mode === mode}
                onClick={() => commit(setDesignMode(doc, mode))}
                className={`h-6 rounded px-1.5 text-[11px] capitalize ${doc.mode === mode ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover'}`}
              >
                {mode}
              </button>
            ))}
            <button
              type="button"
              aria-pressed={doc.snap}
              onClick={() => commit({ ...doc, snap: !doc.snap })}
              className={`ml-1 h-6 rounded px-2 text-[12px] ${doc.snap ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover'}`}
            >
              Snap
            </button>
          </div>
        </div>
      </div>
      {propsOpen ? (
        <aside className="flex w-[260px] flex-none flex-col overflow-y-auto border-l border-koma-border bg-koma-panel">
          <div className="flex h-8 flex-none items-center gap-1 px-3 text-[12px] text-koma-fg">
            <span className="min-w-0 flex-1 truncate">{multi ? `${selectedNodes.length} selected` : selected ? designLayerName(selected) : 'Styles'}</span>
            {selection.length || focusId ? (
              <button
                type="button"
                aria-label="Clear"
                title="Clear"
                onClick={stepOut}
                className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
              >
                <X size={13} />
              </button>
            ) : null}
          </div>
          {selectedNodes.length ? (
            <NodeSettings
              doc={doc}
              nodes={selectedNodes}
              hasParent={everyParent}
              sizeModes={sizeModes}
              componentName={!multi && selectedVariant ? focusedComponent?.name ?? null : null}
              variantProps={!multi && selectedVariant ? selectedVariant.props : null}
              axes={!multi && selected?.kind === 'instance' ? instanceAxes(storedDoc, selected) : null}
              onMakeComponent={!multi && !focusId && selected?.kind === 'frame' ? () => commandsRef.current?.component() : undefined}
              onAddVariant={!multi && focusId ? () => {
                const stored = useKoma.getState().design.docs[key]?.doc
                const component = stored?.components.find((item) => item.id === focusId)
                if (!stored || !component) return
                const next = addComponentVariant(stored, focusId, nextVariantProps(component.variants), () => mintId('n'))
                if (next) commitStored(next)
              } : undefined}
              onVariantProps={!multi && focusId && selectedVariant ? (props) => {
                const stored = useKoma.getState().design.docs[key]?.doc
                if (!stored || !selectedId) return
                const next = setVariantProps(stored, focusId, selectedId, props)
                if (next) commitStored(next, true)
              } : undefined}
              onRenameComponent={!multi && focusId ? (name) => {
                const stored = useKoma.getState().design.docs[key]?.doc
                if (!stored) return
                const next = renameComponent(stored, focusId, name)
                if (next) commitStored(next, true)
              } : undefined}
              onInstanceVariant={!multi && selected?.kind === 'instance' ? (props) => {
                const stored = useKoma.getState().design.docs[key]?.doc
                if (!stored) return
                const next = setInstanceVariant(stored, selected.id, props)
                if (next) commitStored(next)
              } : undefined}
              onAddToChat={!multi && chatQuery ? sendChat : undefined}
              onAlign={selectedNodes.length && (multi || everyParent) ? (axis, edge) => {
                const stored = useKoma.getState().design.docs[key]?.doc
                if (!stored) return
                commit(alignDesignNodes(editingDoc(stored, focusRef.current), selection, axis, edge))
              } : undefined}
              onResetInstance={!multi && selected?.kind === 'instance' && (selected.text || selected.fill) ? () => {
                const stored = useKoma.getState().design.docs[key]?.doc
                if (!stored) return
                commitStored(resetInstanceOverrides(stored, selected.id))
              } : undefined}
              onPatch={(fn) => patchSelected(fn)}
              onType={(fn) => patchSelected(fn, true)}
              onTypeFocus={() => {
                labelNoted.current = false
              }}
              onTypeBlur={() => {
                labelNoted.current = false
              }}
            />
          ) : (
            <TokenEditor root={tab.root} path={tab.path} doc={storedDoc} onCommit={(next) => commitStored(next)} />
          )}
        </aside>
      ) : null}
      {menu ? (
        <DesignMenu
          x={menu.x}
          y={menu.y}
          items={designMenuItems(doc, selection, menu.canvasX, menu.canvasY, !!copiedShape?.nodes.length, copiedStyle != null, !focusId && selection.length === 1 && findDesignNode(doc, selection[0])?.kind === 'frame')}
          onClose={() => setMenu(null)}
          onPick={(id) => {
            const here = menu
            setMenu(null)
            const commands = commandsRef.current
            if (!commands) return
            if (id === 'copy') commands.copy()
            else if (id === 'paste') commands.paste()
            else if (id === 'paste-here') commands.paste({ x: here.canvasX, y: here.canvasY })
            else if (id === 'duplicate') commands.duplicate()
            else if (id === 'copy-style') commands.copyStyle()
            else if (id === 'paste-style') commands.pasteStyle()
            else if (id === 'front' || id === 'forward' || id === 'backward' || id === 'back') commands.order(id)
            else if (id === 'group') commands.wrap('group')
            else if (id === 'ungroup') commands.unwrap()
            else if (id === 'frame-selection') commands.wrap('frame')
            else if (id === 'auto') commands.auto()
            else if (id === 'component') commands.component()
            else if (id === 'hide') commands.hide()
            else if (id === 'lock') commands.lock()
            else if (id === 'flip-x') commands.flip('x')
            else if (id === 'flip-y') commands.flip('y')
            else if (id === 'delete') commands.remove()
            else if (id.startsWith('select:')) commands.select(id.slice('select:'.length))
          }}
        />
      ) : null}
    </div>
  )
}

function canvasBackdrop(snap: boolean, grid: number, view: View): { backgroundImage?: string; backgroundSize?: string; backgroundPosition?: string } {
  if (view.zoom >= 8) {
    const cell = view.zoom
    return {
      backgroundImage: 'linear-gradient(to right, var(--color-koma-border) 1px, transparent 1px), linear-gradient(to bottom, var(--color-koma-border) 1px, transparent 1px)',
      backgroundSize: `${cell}px ${cell}px`,
      backgroundPosition: `${view.panX}px ${view.panY}px`,
    }
  }
  if (!snap) return {}
  return {
    backgroundImage: 'radial-gradient(circle, var(--color-koma-border) 1px, transparent 1px)',
    backgroundSize: `${grid * view.zoom}px ${grid * view.zoom}px`,
    backgroundPosition: `${view.panX}px ${view.panY}px`,
  }
}

function DesignRulers({ panX, panY, zoom }: { panX: number; panY: number; zoom: number }) {
  const step = zoom >= 32 ? 10 : zoom >= 8 ? 50 : zoom >= 2 ? 100 : 200
  const ticks = (span: number, origin: number) => {
    const start = Math.floor(-origin / zoom / step) * step - step
    const items: { at: number; label: number }[] = []
    for (let value = start; items.length < 80; value += step) {
      items.push({ at: origin + value * zoom, label: value })
    }
    return items.filter((item) => item.at > -40 && item.at < span + 40)
  }
  const span = 4096
  return (
    <>
      <div className="pointer-events-none absolute inset-x-0 top-0 z-20 h-4 border-b border-koma-border bg-koma-panel/90 text-[9px] text-koma-dim">
        {ticks(span, panX).map((tick) => (
          <span key={`x${tick.label}`} className="absolute top-0 h-4 overflow-hidden pl-0.5 leading-4" style={{ left: tick.at }}>{tick.label}</span>
        ))}
      </div>
      <div className="pointer-events-none absolute inset-y-0 left-0 z-20 w-4 border-r border-koma-border bg-koma-panel/90 text-[9px] text-koma-dim">
        {ticks(span, panY).map((tick) => (
          <span key={`y${tick.label}`} className="absolute left-0 w-4 overflow-hidden pl-px leading-3" style={{ top: tick.at }}>{tick.label}</span>
        ))}
      </div>
      <div className="pointer-events-none absolute left-0 top-0 z-30 h-4 w-4 border-b border-r border-koma-border bg-koma-panel" />
    </>
  )
}

function designMenuItems(doc: DesignDoc, selection: string[], x: number, y: number, canPaste: boolean, canPasteStyle: boolean, canComponent: boolean): DesignMenuItem[] {
  const selected = selection.length > 0
  const rows = selection.map((id) => locateDesign(doc, id))
  const sameParent = rows.length > 0 && rows.every((row) => row && row.parentId === rows[0]?.parentId)
  const canUngroup = rows.some((row) => row?.node.kind === 'group')
  const stack = stackDesign(doc, x, y).slice(0, 12)
  return [
    { id: 'copy', label: 'Copy', shortcut: '⌘C', disabled: !sameParent },
    { id: 'paste', label: 'Paste', shortcut: '⌘V', disabled: !canPaste },
    { id: 'paste-here', label: 'Paste here', disabled: !canPaste },
    { id: 'duplicate', label: 'Duplicate', shortcut: '⌘D', disabled: !selected },
    { id: 'copy-style', label: 'Copy style', shortcut: '⌥⌘C', disabled: !selected },
    { id: 'paste-style', label: 'Paste style', shortcut: '⌥⌘V', disabled: !canPasteStyle || !selected },
    { id: 'divider-1', label: '' },
    { id: 'select-layer', label: 'Select layer', disabled: stack.length === 0, children: stack.map((node) => ({ id: `select:${node.id}`, label: designLayerName(node) })) },
    { id: 'divider-2', label: '' },
    { id: 'front', label: 'Bring to front', shortcut: '⌘]', disabled: !selected },
    { id: 'forward', label: 'Bring forward', shortcut: ']', disabled: !selected },
    { id: 'backward', label: 'Send backward', shortcut: '[', disabled: !selected },
    { id: 'back', label: 'Send to back', shortcut: '⌘[', disabled: !selected },
    { id: 'divider-3', label: '' },
    { id: 'group', label: 'Group selection', shortcut: '⌘G', disabled: !sameParent },
    { id: 'ungroup', label: 'Ungroup', shortcut: '⇧⌘G', disabled: !canUngroup },
    { id: 'frame-selection', label: 'Frame selection', shortcut: '⌥⌘G', disabled: !sameParent },
    { id: 'auto', label: 'Add auto layout', disabled: !sameParent },
    { id: 'component', label: 'Create component', disabled: !canComponent },
    { id: 'divider-4', label: '' },
    { id: 'hide', label: 'Show/Hide', shortcut: '⇧⌘H', disabled: !selected },
    { id: 'lock', label: 'Lock/Unlock', disabled: !selected },
    { id: 'flip-x', label: 'Flip horizontal', shortcut: '⇧H', disabled: !selected },
    { id: 'flip-y', label: 'Flip vertical', shortcut: '⇧V', disabled: !selected },
    { id: 'divider-5', label: '' },
    { id: 'delete', label: 'Delete', shortcut: 'Del', danger: true, disabled: !selected },
  ]
}

function ToolButton({ label, selected, onClick, children }: { label: string; selected: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-pressed={selected}
      onClick={onClick}
      className={`flex h-6 w-6 items-center justify-center rounded ${selected ? 'bg-koma-accent/20 text-koma-accent ring-1 ring-inset ring-koma-accent' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`}
    >
      {children}
    </button>
  )
}

function DesignNodeView({
  doc,
  node,
  zoom,
  selectedIds,
  editing,
  dragCursor,
  locked = false,
  onSelect,
  onResize,
  onCorner,
  onEdit,
  onText,
  onTextBlur,
  onMenu,
}: {
  doc: DesignDoc
  node: DesignNode
  zoom: number
  selectedIds: string[]
  editing: string | null
  dragCursor: string | null
  locked?: boolean
  onSelect: (id: string, event: ReactPointerEvent<HTMLDivElement>) => void
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
  const container = node.kind === 'frame' || node.kind === 'group'
  const bareFill = node.fill === 'none' || chrome.fill === 'none'
  const bareStroke = (node.kind === 'frame' || node.kind === 'group') && (!node.stroke || node.stroke === 'none')
  const strokeOff = chrome.stroke === 'none' || ((node.kind === 'rect' || node.kind === 'ellipse') && node.stroke == null)
  const fillFallback = node.kind === 'frame' ? '#ffffff' : node.kind === 'rect' || node.kind === 'ellipse' || node.kind === 'vector' ? SHAPE_FILL : 'var(--color-koma-panel)'
  const fill = bareFill ? 'transparent' : paintCss(doc, chrome.fill, fillFallback)
  const stroke = paintCss(doc, chrome.stroke, node.kind === 'line' || node.kind === 'vector' ? '#1c1c1c' : 'var(--color-koma-border)')
  const radius = node.kind === 'ellipse' ? '50%' : typeof chrome.radius === 'number' ? chrome.radius : Number(resolveRef(doc, String(chrome.radius))) || 0
  const unit = 1 / Math.max(zoom, 0.25)
  const radiusNumber = typeof radius === 'number' ? radius : 0
  const inset = radiusNumber > 0 ? radiusNumber : 14 * unit
  const insetX = Math.min(node.w / 2, Math.max(8 * unit, inset))
  const insetY = Math.min(node.h / 2, Math.max(8 * unit, inset))
  const children = visual?.children ?? (node.kind === 'instance' ? undefined : node.children)
  const rotation = node.rotation ?? 0
  const flipX = node.flipX ? -1 : 1
  const flipY = node.flipY ? -1 : 1
  const transform = rotation || node.flipX || node.flipY ? `rotate(${rotation}deg) scale(${flipX}, ${flipY})` : undefined
  const childIds = locked || node.locked || node.kind === 'instance' ? [] : selectedIds
  const hitHere = !locked
  return (
    <div
      className={`absolute ${hitHere ? 'pointer-events-auto' : 'pointer-events-none'}`}
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
        if (!hitHere || node.locked || node.kind !== 'text') return
        event.stopPropagation()
        onEdit(node.id)
      }}
    >
      {container ? (
        <span className="pointer-events-none absolute left-0 truncate text-[11px] text-koma-dim" style={{ top: -16, maxWidth: Math.max(node.w, 80) }}>
          {designLayerName(node)}
        </span>
      ) : null}
      <div
        className="absolute inset-0"
        style={{
          background: node.kind === 'line' || node.kind === 'vector' ? 'transparent' : fill,
          border: bareStroke || strokeOff || node.kind === 'line' || node.kind === 'vector' ? undefined : `${chrome.strokeWidth}px solid ${stroke}`,
          borderRadius: radius,
          overflow: node.kind === 'frame' || node.kind === 'instance' ? 'hidden' : undefined,
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
            style={{ fontSize: style.fontSize, fontWeight: weightCss(style.weight), textAlign: style.align, lineHeight: style.lineHeight ? `${style.lineHeight}px` : undefined, letterSpacing: style.letterSpacing ? `${style.letterSpacing}px` : undefined, color: paintCss(doc, style.color, 'var(--color-koma-fg)') }}
          />
        ) : node.kind === 'text' ? (
          <div
            className="flex h-full w-full items-center px-1"
            style={{ fontSize: style.fontSize, fontWeight: weightCss(style.weight), justifyContent: style.align === 'center' ? 'center' : style.align === 'right' ? 'flex-end' : 'flex-start', lineHeight: style.lineHeight ? `${style.lineHeight}px` : undefined, letterSpacing: style.letterSpacing ? `${style.letterSpacing}px` : undefined, color: paintCss(doc, style.color, 'var(--color-koma-fg)') }}
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
            locked={locked || !!node.locked || node.kind === 'instance'}
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
          { id: 'tl' as const, x: insetX, y: insetY, cursor: 'nwse-resize' },
          { id: 'tr' as const, x: node.w - insetX, y: insetY, cursor: 'nesw-resize' },
          { id: 'bl' as const, x: insetX, y: node.h - insetY, cursor: 'nesw-resize' },
          { id: 'br' as const, x: node.w - insetX, y: node.h - insetY, cursor: 'nwse-resize' },
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

function NodeSettings({
  doc,
  nodes,
  hasParent,
  sizeModes,
  componentName,
  variantProps,
  axes,
  onMakeComponent,
  onAddVariant,
  onVariantProps,
  onRenameComponent,
  onInstanceVariant,
  onResetInstance,
  onAddToChat,
  onAlign,
  onPatch,
  onType,
  onTypeFocus,
  onTypeBlur,
}: {
  doc: DesignDoc
  nodes: DesignNode[]
  hasParent: boolean
  sizeModes: boolean
  componentName?: string | null
  variantProps?: Record<string, string> | null
  axes?: { name: string; values: string[]; current: string }[] | null
  onMakeComponent?: () => void
  onAddVariant?: () => void
  onVariantProps?: (props: Record<string, string>) => void
  onRenameComponent?: (name: string) => void
  onInstanceVariant?: (props: Record<string, string>) => void
  onResetInstance?: () => void
  onAddToChat?: () => void
  onAlign?: (axis: DesignAlignAxis, edge: DesignAlignEdge) => void
  onPatch: (fn: (node: DesignNode) => DesignNode) => void
  onType: (fn: (node: DesignNode) => DesignNode) => void
  onTypeFocus: () => void
  onTypeBlur: () => void
}) {
  const [propName, setPropName] = useState('variant')
  const [propValue, setPropValue] = useState('')
  const node = nodes[0]
  const multi = nodes.length > 1
  const allFrames = nodes.every((item) => item.kind === 'frame')
  const allText = nodes.every((item) => item.kind === 'text')
  const showRadius = nodes.every((item) => item.kind === 'rect' || item.kind === 'frame')
  const chrome = nodeChrome(node)
  const style = textStyle(node)
  const colorTokens = doc.tokens.filter((token) => token.kind === 'color')
  const radiusTokens = doc.tokens.filter((token) => token.kind === 'radius')
  const numberOf = (pick: (item: DesignNode) => number) => {
    const value = sharedValue(nodes.map(pick))
    return { value: value ?? pick(node), mixed: value == null }
  }
  const textOf = <T extends string>(pick: (item: DesignNode) => T) => {
    const value = sharedValue(nodes.map(pick))
    return { value: value ?? pick(node), mixed: value == null }
  }
  const setField = (patch: Partial<DesignNode>, clear: (keyof DesignNode)[] = []) => {
    onPatch((current) => {
      const next: DesignNode = { ...current, ...patch }
      for (const key of clear) delete next[key]
      return next
    })
  }
  const paintChange = (field: 'fill' | 'stroke', next: string | null) => {
    onPatch((current) => {
      const fallback = field === 'fill' ? (current.kind === 'frame' ? '#ffffff' : SHAPE_FILL) : '#1c1c1c'
      const themedFill = field === 'fill' && (current.kind === 'rect' || current.kind === 'ellipse' || (current.kind === 'vector' && !!current.vector?.regions.length))
      const copy: DesignNode = { ...current }
      if (next === 'none') copy[field] = 'none'
      else if (next == null) {
        if (themedFill) delete copy[field]
        else copy[field] = fallback
      } else copy[field] = next
      return copy
    })
  }
  const xField = numberOf((item) => item.x)
  const yField = numberOf((item) => item.y)
  const wField = numberOf((item) => item.w)
  const hField = numberOf((item) => item.h)
  const rotationField = numberOf((item) => item.rotation ?? 0)
  const opacityField = numberOf((item) => Math.round((item.opacity ?? 1) * 100))
  const strokeWidthField = numberOf((item) => nodeChrome(item).strokeWidth)
  const radiusField = textOf((item) => (typeof item.radius === 'number' ? (item.radius > 0 ? String(item.radius) : '0') : item.radius || '0'))
  const fillField = textOf((item) => containerPaint(item, 'fill', nodeChrome(item).fill))
  const strokeField = textOf((item) => containerPaint(item, 'stroke', nodeChrome(item).stroke))
  const layoutField = textOf((item) => item.layout ?? 'free')
  const gapField = numberOf((item) => item.gap ?? 0)
  const padField = numberOf((item) => item.pad ?? 0)
  const justifyField = textOf((item) => item.justify ?? 'start')
  const alignField = textOf((item) => item.align ?? 'start')
  const wModeField = textOf((item) => item.wMode ?? 'fixed')
  const hModeField = textOf((item) => item.hMode ?? 'fixed')
  const weightField = textOf((item) => textStyle(item).weight)
  const textAlignField = textOf((item) => textStyle(item).align)
  const fontSizeField = numberOf((item) => textStyle(item).fontSize)
  const lineField = numberOf((item) => textStyle(item).lineHeight)
  const trackingField = numberOf((item) => textStyle(item).letterSpacing)
  const colorField = textOf((item) => textStyle(item).color || 'none')
  const absoluteField = textOf((item) => (item.absolute ? 'on' : 'off'))
  const flows = nodes.every((item) => item.layout === 'row' || item.layout === 'column')
  const flipXOn = nodes.every((item) => !!item.flipX)
  const flipYOn = nodes.every((item) => !!item.flipY)
  const columnLayout = !layoutField.mixed && layoutField.value === 'column'
  const setJustify = (justify: 'start' | 'center' | 'end' | 'space') => {
    setField(justify === 'start' ? {} : { justify }, justify === 'start' ? ['justify'] : [])
  }
  const setCross = (align: 'start' | 'center' | 'end') => {
    setField(align === 'start' ? {} : { align }, align === 'start' ? ['align'] : [])
  }
  return (
    <div className="flex flex-col gap-3 px-3 pb-3 text-[12px]">
      {onMakeComponent ? (
        <button type="button" onClick={onMakeComponent} className="h-7 rounded bg-koma-accent/20 text-koma-accent">
          Create component
        </button>
      ) : null}
      {onAddToChat ? (
        <button type="button" onClick={onAddToChat} className="h-7 rounded text-koma-dim hover:bg-koma-hover">
          Add to chat
        </button>
      ) : null}
      {componentName != null ? (
        <label className="flex flex-col gap-1">
          <span className="text-koma-dim">Component</span>
          <input
            value={componentName}
            onFocus={onTypeFocus}
            onBlur={onTypeBlur}
            onChange={(event) => onRenameComponent?.(event.target.value)}
            className="h-7 rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
          />
        </label>
      ) : null}
      {variantProps ? (
        <div className="flex flex-col gap-1">
          <span className="text-koma-dim">Variant</span>
          {Object.entries(variantProps).map(([key, value]) => (
            <label key={key} className="flex items-center gap-1">
              <span className="w-16 flex-none truncate text-koma-dim">{key}</span>
              <input
                value={value}
                aria-label={`${key} value`}
                onFocus={onTypeFocus}
                onBlur={onTypeBlur}
                onChange={(event) => onVariantProps?.({ ...variantProps, [key]: event.target.value })}
                className="h-7 min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
              />
            </label>
          ))}
          <form
            className="flex items-center gap-1"
            onSubmit={(event) => {
              event.preventDefault()
              const name = propName.trim()
              const value = propValue.trim()
              if (!name || !value) return
              onVariantProps?.({ ...variantProps, [name]: value })
              setPropValue('')
            }}
          >
            <input value={propName} aria-label="Property name" onChange={(event) => setPropName(event.target.value)} className="h-7 w-16 flex-none rounded border border-koma-border bg-koma-bg px-1 text-[12px] text-koma-fg outline-none" />
            <input value={propValue} aria-label="Property value" onChange={(event) => setPropValue(event.target.value)} className="h-7 min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-1 text-[12px] text-koma-fg outline-none" />
            <button type="submit" className="h-7 rounded px-1 text-koma-dim hover:bg-koma-hover">Add</button>
          </form>
          {onAddVariant ? (
            <button type="button" onClick={onAddVariant} className="h-7 rounded text-koma-dim hover:bg-koma-hover">
              Add variant
            </button>
          ) : null}
        </div>
      ) : null}
      {axes?.map((axis) => (
        <Choices
          key={axis.name}
          label={axis.name}
          value={axis.current}
          options={axis.values.map((value) => ({ value, label: value }))}
          onChange={(value) => onInstanceVariant?.({ ...(node.variant ?? {}), [axis.name]: value })}
        />
      ))}
      {onResetInstance ? (
        <button type="button" onClick={onResetInstance} className="h-7 rounded text-koma-dim hover:bg-koma-hover">
          Reset overrides
        </button>
      ) : null}
      {multi ? null : (
        <label className="flex items-center gap-1">
          <span className={`flex h-7 w-7 flex-none items-center justify-center ${node.kind === 'instance' ? 'text-[#9747ff]' : 'text-koma-dim'}`}>
            <KindMark kind={node.kind} />
          </span>
          <input
            aria-label={node.kind === 'text' || node.kind === 'instance' ? 'Text' : 'Name'}
            value={node.kind === 'text' || node.kind === 'instance' ? node.text ?? '' : node.name ?? ''}
            placeholder={node.kind === 'instance' ? 'Override' : node.kind === 'text' ? 'Text' : 'Name'}
            onFocus={onTypeFocus}
            onBlur={onTypeBlur}
            onChange={(event) => {
              const value = event.target.value
              onType((current) => {
                if (current.kind === 'text') return { ...current, text: value }
                if (current.kind === 'instance') {
                  const next = { ...current }
                  if (value) next.text = value
                  else delete next.text
                  return next
                }
                return { ...current, name: value || undefined }
              })
            }}
            className="h-7 min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
          />
        </label>
      )}
      {onAlign ? (
        <div className="flex flex-wrap items-center gap-1">
          <div className="flex gap-0.5">
            <AlignButton label="Align left" onClick={() => onAlign('horizontal', 'min')}><AlignStartVertical size={14} /></AlignButton>
            <AlignButton label="Align center" onClick={() => onAlign('horizontal', 'center')}><AlignCenterVertical size={14} /></AlignButton>
            <AlignButton label="Align right" onClick={() => onAlign('horizontal', 'max')}><AlignEndVertical size={14} /></AlignButton>
          </div>
          <div className="flex gap-0.5">
            <AlignButton label="Align top" onClick={() => onAlign('vertical', 'min')}><AlignStartHorizontal size={14} /></AlignButton>
            <AlignButton label="Align middle" onClick={() => onAlign('vertical', 'center')}><AlignCenterHorizontal size={14} /></AlignButton>
            <AlignButton label="Align bottom" onClick={() => onAlign('vertical', 'max')}><AlignEndHorizontal size={14} /></AlignButton>
          </div>
          {nodes.length >= 3 && hasParent ? (
            <div className="flex gap-0.5">
              <AlignButton label="Distribute horizontal" onClick={() => onAlign('horizontal', 'spread')}><AlignHorizontalSpaceBetween size={14} /></AlignButton>
              <AlignButton label="Distribute vertical" onClick={() => onAlign('vertical', 'spread')}><AlignVerticalSpaceBetween size={14} /></AlignButton>
            </div>
          ) : null}
        </div>
      ) : null}
      <Section title="Position">
        <div className="grid grid-cols-2 gap-1">
          <GeomField label="X" value={xField.value} mixed={xField.mixed} onChange={(x) => setField({ x })} />
          <GeomField label="Y" value={yField.value} mixed={yField.mixed} onChange={(y) => setField({ y })} />
          <div className="flex min-w-0 gap-1">
            <div className="min-w-0 flex-1">
              <GeomField label="W" value={wField.value} mixed={wField.mixed} onChange={(w) => setField({ w: Math.max(1, w) }, ['wMode'])} />
            </div>
            {sizeModes ? (
              <SizeMode label="Width sizing" value={wModeField.value} mixed={wModeField.mixed} onChange={(mode) => setField(mode === 'fixed' ? {} : { wMode: mode }, mode === 'fixed' ? ['wMode'] : [])} />
            ) : null}
          </div>
          <div className="flex min-w-0 gap-1">
            <div className="min-w-0 flex-1">
              <GeomField label="H" value={hField.value} mixed={hField.mixed} onChange={(h) => setField({ h: Math.max(1, h) }, ['hMode'])} />
            </div>
            {sizeModes ? (
              <SizeMode label="Height sizing" value={hModeField.value} mixed={hModeField.mixed} onChange={(mode) => setField(mode === 'fixed' ? {} : { hMode: mode }, mode === 'fixed' ? ['hMode'] : [])} />
            ) : null}
          </div>
        </div>
        <div className="flex items-center gap-1">
          <div className="min-w-0 flex-1">
            <GeomField label="R" suffix="°" value={rotationField.value} mixed={rotationField.mixed} onChange={(rotation) => {
              const wrapped = ((rotation % 360) + 360) % 360
              setField(wrapped ? { rotation: wrapped } : {}, wrapped ? [] : ['rotation'])
            }} />
          </div>
          <AlignButton label="Flip horizontal" pressed={flipXOn} onClick={() => onPatch((current) => {
            const next = { ...current }
            if (current.flipX) delete next.flipX
            else next.flipX = true
            return next
          })}><FlipHorizontal2 size={14} /></AlignButton>
          <AlignButton label="Flip vertical" pressed={flipYOn} onClick={() => onPatch((current) => {
            const next = { ...current }
            if (current.flipY) delete next.flipY
            else next.flipY = true
            return next
          })}><FlipVertical2 size={14} /></AlignButton>
          <AlignButton label="Rotate 90 degrees" onClick={() => onPatch((current) => {
            const wrapped = (((current.rotation ?? 0) + 90) % 360 + 360) % 360
            const next = { ...current }
            if (wrapped) next.rotation = wrapped
            else delete next.rotation
            return next
          })}><RotateCw size={14} /></AlignButton>
        </div>
      </Section>
      {allFrames || hasParent ? (
      <Section title="Layout">
      {allFrames ? (
        <div className="flex gap-0.5">
          <AlignButton label="Free" pressed={!layoutField.mixed && layoutField.value === 'free'} onClick={() => setField({}, ['layout'])}><Square size={14} /></AlignButton>
          <AlignButton label="Row" pressed={!layoutField.mixed && layoutField.value === 'row'} onClick={() => setField({ layout: 'row' })}><ArrowRight size={14} /></AlignButton>
          <AlignButton label="Column" pressed={!layoutField.mixed && layoutField.value === 'column'} onClick={() => setField({ layout: 'column' })}><ArrowDown size={14} /></AlignButton>
        </div>
      ) : null}
      {flows ? (
        <>
          <div className="grid grid-cols-2 gap-1">
            <GeomField label="Gap" value={gapField.value} mixed={gapField.mixed} onChange={(gap) => setField(gap > 0 ? { gap } : {}, gap > 0 ? [] : ['gap'])} />
            <GeomField label="Pad" value={padField.value} mixed={padField.mixed} onChange={(pad) => setField(pad > 0 ? { pad } : {}, pad > 0 ? [] : ['pad'])} />
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <div className="flex gap-0.5">
              {columnLayout ? (
                <>
                  <AlignButton label="Align top" pressed={!justifyField.mixed && justifyField.value === 'start'} onClick={() => setJustify('start')}><AlignStartHorizontal size={14} /></AlignButton>
                  <AlignButton label="Align middle" pressed={!justifyField.mixed && justifyField.value === 'center'} onClick={() => setJustify('center')}><AlignCenterHorizontal size={14} /></AlignButton>
                  <AlignButton label="Align bottom" pressed={!justifyField.mixed && justifyField.value === 'end'} onClick={() => setJustify('end')}><AlignEndHorizontal size={14} /></AlignButton>
                  <AlignButton label="Space between" pressed={!justifyField.mixed && justifyField.value === 'space'} onClick={() => setJustify('space')}><AlignVerticalSpaceBetween size={14} /></AlignButton>
                </>
              ) : (
                <>
                  <AlignButton label="Align left" pressed={!justifyField.mixed && justifyField.value === 'start'} onClick={() => setJustify('start')}><AlignStartVertical size={14} /></AlignButton>
                  <AlignButton label="Align center" pressed={!justifyField.mixed && justifyField.value === 'center'} onClick={() => setJustify('center')}><AlignCenterVertical size={14} /></AlignButton>
                  <AlignButton label="Align right" pressed={!justifyField.mixed && justifyField.value === 'end'} onClick={() => setJustify('end')}><AlignEndVertical size={14} /></AlignButton>
                  <AlignButton label="Space between" pressed={!justifyField.mixed && justifyField.value === 'space'} onClick={() => setJustify('space')}><AlignHorizontalSpaceBetween size={14} /></AlignButton>
                </>
              )}
            </div>
            <div className="flex gap-0.5">
              {columnLayout ? (
                <>
                  <AlignButton label="Align left" pressed={!alignField.mixed && alignField.value === 'start'} onClick={() => setCross('start')}><AlignStartVertical size={14} /></AlignButton>
                  <AlignButton label="Align center" pressed={!alignField.mixed && alignField.value === 'center'} onClick={() => setCross('center')}><AlignCenterVertical size={14} /></AlignButton>
                  <AlignButton label="Align right" pressed={!alignField.mixed && alignField.value === 'end'} onClick={() => setCross('end')}><AlignEndVertical size={14} /></AlignButton>
                </>
              ) : (
                <>
                  <AlignButton label="Align top" pressed={!alignField.mixed && alignField.value === 'start'} onClick={() => setCross('start')}><AlignStartHorizontal size={14} /></AlignButton>
                  <AlignButton label="Align middle" pressed={!alignField.mixed && alignField.value === 'center'} onClick={() => setCross('center')}><AlignCenterHorizontal size={14} /></AlignButton>
                  <AlignButton label="Align bottom" pressed={!alignField.mixed && alignField.value === 'end'} onClick={() => setCross('end')}><AlignEndHorizontal size={14} /></AlignButton>
                </>
              )}
            </div>
          </div>
        </>
      ) : null}
      {hasParent ? (
        <div className="flex items-center justify-between">
          <span className="text-koma-dim">Absolute{absoluteField.mixed ? ' · Mixed' : ''}</span>
          <button
            type="button"
            aria-pressed={absoluteField.value === 'on' && !absoluteField.mixed}
            onClick={() => onPatch((current) => {
              const next = { ...current }
              if (current.absolute) delete next.absolute
              else next.absolute = true
              return next
            })}
            className={`h-6 rounded px-2 ${absoluteField.value === 'on' && !absoluteField.mixed ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover'}`}
          >
            {absoluteField.mixed ? 'Mixed' : absoluteField.value === 'on' ? 'On' : 'Off'}
          </button>
        </div>
      ) : null}
      </Section>
      ) : null}
      <Section title="Appearance">
        <div className={`grid gap-1 ${showRadius ? 'grid-cols-2' : 'grid-cols-1'}`}>
          <GeomField label="Opacity" suffix="%" value={opacityField.value} mixed={opacityField.mixed} onChange={(value) => {
            const opacity = Math.min(100, Math.max(0, value)) / 100
            setField(opacity < 1 ? { opacity } : {}, opacity < 1 ? [] : ['opacity'])
          }} />
          {showRadius ? (
            <RadiusField
              value={radiusField.mixed ? undefined : node.radius}
              mixed={radiusField.mixed}
              resolved={typeof node.radius === 'number' ? String(node.radius) : resolveRef(doc, typeof node.radius === 'string' ? node.radius : '')}
              tokens={radiusTokens}
              onChange={(radius) => {
                if (radius == null) setField({}, ['radius'])
                else setField({ radius })
              }}
            />
          ) : null}
        </div>
      </Section>
      <Section title="Fill">
        <PaintRow
          label="Fill"
          mixed={fillField.mixed}
          value={fillField.value}
          fallback="#1a1d27"
          resolved={resolveRef(doc, chrome.fill)}
          tokens={colorTokens}
          onChange={(next) => paintChange('fill', next)}
        />
      </Section>
      <Section title="Stroke">
        <PaintRow
          label="Stroke"
          mixed={strokeField.mixed}
          value={strokeField.value}
          fallback="#8b93b8"
          resolved={resolveRef(doc, chrome.stroke)}
          tokens={colorTokens}
          weight={strokeWidthField}
          onWeight={(strokeWidth) => setField(strokeWidth > 0 && strokeWidth !== 1 ? { strokeWidth } : {}, strokeWidth > 0 && strokeWidth !== 1 ? [] : ['strokeWidth'])}
          onChange={(next) => paintChange('stroke', next)}
        />
      </Section>
      {allText ? (
        <Section title="Text">
          <div className="flex items-center gap-1">
            <div className="min-w-0 flex-1">
              <GeomField label="Size" value={fontSizeField.value} mixed={fontSizeField.mixed} onChange={(fontSize) => {
                if (!Number.isFinite(fontSize) || fontSize <= 0 || fontSize === 13) setField({}, ['fontSize'])
                else setField({ fontSize })
              }} />
            </div>
            <select
              aria-label="Weight"
              value={weightField.mixed ? '' : weightField.value}
              onChange={(event) => {
                const weight = event.target.value
                if (weight !== 'regular' && weight !== 'medium' && weight !== 'bold') return
                setField(weight === 'regular' ? {} : { weight }, weight === 'regular' ? ['weight'] : [])
              }}
              className="h-7 flex-none rounded border border-koma-border bg-koma-bg px-1 text-[12px] text-koma-fg outline-none"
            >
              {weightField.mixed ? <option value="">Mixed</option> : null}
              <option value="regular">Regular</option>
              <option value="medium">Medium</option>
              <option value="bold">Bold</option>
            </select>
          </div>
          <div className="flex gap-0.5">
            <AlignButton label="Align left" pressed={!textAlignField.mixed && textAlignField.value === 'left'} onClick={() => setField({}, ['textAlign'])}><TextAlignStart size={14} /></AlignButton>
            <AlignButton label="Align center" pressed={!textAlignField.mixed && textAlignField.value === 'center'} onClick={() => setField({ textAlign: 'center' })}><TextAlignCenter size={14} /></AlignButton>
            <AlignButton label="Align right" pressed={!textAlignField.mixed && textAlignField.value === 'right'} onClick={() => setField({ textAlign: 'right' })}><TextAlignEnd size={14} /></AlignButton>
          </div>
          <PaintRow
            label="Color"
            mixed={colorField.mixed}
            value={colorField.value}
            fallback="#c8d3f5"
            resolved={resolveRef(doc, style.color)}
            tokens={colorTokens}
            onChange={(next) => setField(next && next !== 'none' ? { color: next } : {}, next && next !== 'none' ? [] : ['color'])}
          />
          <div className="grid grid-cols-2 gap-1">
            <GeomField label="Line" value={lineField.value} mixed={lineField.mixed} onChange={(lineHeight) => {
              if (!Number.isFinite(lineHeight) || lineHeight <= 0) setField({}, ['lineHeight'])
              else setField({ lineHeight })
            }} />
            <GeomField label="Spacing" value={trackingField.value} mixed={trackingField.mixed} onChange={(letterSpacing) => {
              if (!Number.isFinite(letterSpacing) || letterSpacing === 0) setField({}, ['letterSpacing'])
              else setField({ letterSpacing })
            }} />
          </div>
        </Section>
      ) : null}
    </div>
  )
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  const [open, setOpen] = useState(true)
  return (
    <section className="flex flex-col border-t border-koma-border">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        className="flex h-[22px] items-center gap-1 bg-koma-head px-2 text-left text-[11px] font-semibold uppercase tracking-wide text-koma-fg opacity-75 hover:bg-koma-hover hover:opacity-100"
      >
        <ChevronRight size={14} strokeWidth={2} className={`transition-transform ${open ? 'rotate-90' : ''}`} />
        <span className="truncate">{title}</span>
      </button>
      {open ? <div className="flex flex-col gap-2 px-2 py-2">{children}</div> : null}
    </section>
  )
}

function GeomField({ label, ariaLabel, value, mixed, suffix, onChange }: { label: string; ariaLabel?: string; value: number; mixed?: boolean; suffix?: string; onChange: (value: number) => void }) {
  return (
    <label className="flex h-7 items-center gap-1 rounded border border-koma-border bg-koma-bg px-1.5">
      <span className="flex-none text-[11px] text-koma-dim">{label}</span>
      <input
        type="number"
        value={mixed ? '' : Number.isFinite(value) ? Math.round(value * 100) / 100 : 0}
        placeholder={mixed ? 'Mixed' : undefined}
        aria-label={ariaLabel ?? label}
        onChange={(event) => {
          const next = Number(event.target.value)
          if (Number.isFinite(next)) onChange(next)
        }}
        className="h-6 min-w-0 flex-1 bg-transparent text-[12px] text-koma-fg outline-none"
      />
      {suffix ? <span className="flex-none text-[11px] text-koma-dim">{suffix}</span> : null}
    </label>
  )
}

function AlignButton({ label, pressed, onClick, children }: { label: string; pressed?: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button type="button" aria-label={label} title={label} aria-pressed={pressed} onClick={onClick} className={`flex h-7 w-7 flex-none items-center justify-center rounded ${pressed ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`}>
      {children}
    </button>
  )
}

function KindMark({ kind }: { kind: DesignNode['kind'] }) {
  const props = { size: 14, strokeWidth: 2 }
  if (kind === 'frame') return <Frame {...props} />
  if (kind === 'group') return <Group {...props} />
  if (kind === 'ellipse') return <Circle {...props} />
  if (kind === 'line') return <Minus {...props} />
  if (kind === 'vector') return <Spline {...props} />
  if (kind === 'text') return <Type {...props} />
  if (kind === 'instance') return <Component {...props} />
  return <Square {...props} />
}

function SizeMode({ label, value, mixed, onChange }: { label: string; value: string; mixed?: boolean; onChange: (mode: 'fixed' | 'hug' | 'fill') => void }) {
  return (
    <select
      aria-label={label}
      value={mixed ? '' : value}
      onChange={(event) => {
        const mode = event.target.value
        if (mode === 'fixed' || mode === 'hug' || mode === 'fill') onChange(mode)
      }}
      className="h-7 w-12 flex-none rounded border border-koma-border bg-koma-bg px-0.5 text-[10px] text-koma-fg outline-none"
    >
      {mixed ? <option value="">Mix</option> : null}
      <option value="fixed">Fix</option>
      <option value="hug">Hug</option>
      <option value="fill">Fill</option>
    </select>
  )
}

function RadiusField({ value, mixed, resolved, tokens, onChange }: { value: number | string | undefined; mixed?: boolean; resolved?: string; tokens: { name: string }[]; onChange: (radius: number | string | null) => void }) {
  const [open, setOpen] = useState(false)
  const token = !mixed && typeof value === 'string' ? value : ''
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <label className="flex h-7 items-center gap-1 rounded border border-koma-border bg-koma-bg px-1.5">
        <span className="flex-none text-[11px] text-koma-dim">R</span>
        {tokens.length ? (
          <button
            type="button"
            aria-label="Corner radius token"
            aria-expanded={open}
            aria-pressed={!!token}
            onClick={() => setOpen((current) => !current)}
            className={`h-4 w-4 flex-none rounded-full border border-koma-border ${token ? 'bg-koma-accent' : 'bg-transparent'}`}
          />
        ) : null}
        <input
          type={token ? 'text' : 'number'}
          min={token ? undefined : 0}
          aria-label="Corner radius"
          value={mixed ? '' : token || (typeof value === 'number' ? value : 0)}
          placeholder={mixed ? 'Mixed' : undefined}
          onChange={(event) => {
            const raw = event.target.value.trim()
            const named = tokens.find((item) => item.name === raw)
            if (named) {
              onChange(named.name)
              return
            }
            const radius = Number(raw)
            if (!raw || !Number.isFinite(radius) || radius <= 0) onChange(null)
            else onChange(radius)
          }}
          className="h-6 min-w-0 flex-1 bg-transparent text-[12px] text-koma-fg outline-none"
        />
      </label>
      {open ? (
        <div className="flex flex-col gap-0.5 rounded border border-koma-border p-1">
          <button
            type="button"
            onClick={() => {
              const radius = Number(resolved)
              onChange(Number.isFinite(radius) && radius > 0 ? radius : null)
              setOpen(false)
            }}
            className="h-6 rounded px-1 text-left text-[12px] text-koma-dim hover:bg-koma-hover"
          >
            Number
          </button>
          {tokens.map((item) => (
            <button
              key={item.name}
              type="button"
              aria-pressed={token === item.name}
              onClick={() => {
                onChange(item.name)
                setOpen(false)
              }}
              className={`h-6 rounded px-1 text-left text-[12px] ${token === item.name ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover'}`}
            >
              {item.name}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  )
}

function containerPaint(node: DesignNode, field: 'fill' | 'stroke', chrome: string): string {
  if (node.kind === 'group' && node[field] == null) return 'none'
  if (node.kind === 'frame' && field === 'fill' && node.fill == null) return '#ffffff'
  if (node.kind === 'frame' && field === 'stroke' && node.stroke == null) return 'none'
  if ((node.kind === 'rect' || node.kind === 'ellipse') && field === 'fill' && node.fill == null) return SHAPE_FILL
  if ((node.kind === 'rect' || node.kind === 'ellipse') && field === 'stroke' && node.stroke == null) return 'none'
  if ((node.kind === 'line' || node.kind === 'vector') && field === 'stroke' && node.stroke == null) return '#1c1c1c'
  return chrome
}

function penCurve(points: DesignPenPoint[], closed: boolean): string {
  if (!points.length) return ''
  let path = `M ${points[0].x} ${points[0].y}`
  const count = closed ? points.length : points.length - 1
  for (let index = 0; index < count; index++) {
    const start = points[index]
    const end = points[(index + 1) % points.length]
    path += ` C ${start.x + start.outgoing.x} ${start.y + start.outgoing.y} ${end.x + end.incoming.x} ${end.y + end.incoming.y} ${end.x} ${end.y}`
  }
  if (closed) path += ' Z'
  return path
}

function PenOverlay({ draft, hover, zoom }: { draft: PenDraft; hover: { x: number; y: number } | null; zoom: number }) {
  const unit = 1 / Math.max(zoom, 0.25)
  const points = draft.points
  const last = points[points.length - 1]
  const rubber = hover && last ? `M ${last.x} ${last.y} C ${last.x + last.outgoing.x} ${last.y + last.outgoing.y} ${hover.x} ${hover.y} ${hover.x} ${hover.y}` : ''
  return (
    <svg className="pointer-events-none absolute overflow-visible" width={1} height={1}>
      <path d={penCurve(points, false)} fill="none" stroke="#1c1c1c" strokeWidth={2} />
      {rubber ? <path d={rubber} fill="none" stroke={SELECTION} strokeWidth={1.25 * unit} /> : null}
      {points.map((point, index) => {
        const showOut = point.outgoing.x !== 0 || point.outgoing.y !== 0
        const showIn = point.incoming.x !== 0 || point.incoming.y !== 0
        return (
          <g key={`${draft.id}-${index}`}>
            {showOut ? <line x1={point.x} y1={point.y} x2={point.x + point.outgoing.x} y2={point.y + point.outgoing.y} stroke={SELECTION} strokeWidth={unit} /> : null}
            {showIn ? <line x1={point.x} y1={point.y} x2={point.x + point.incoming.x} y2={point.y + point.incoming.y} stroke={SELECTION} strokeWidth={unit} /> : null}
            {showOut ? <circle cx={point.x + point.outgoing.x} cy={point.y + point.outgoing.y} r={3 * unit} fill="#ffffff" stroke={SELECTION} strokeWidth={unit} /> : null}
            {showIn ? <circle cx={point.x + point.incoming.x} cy={point.y + point.incoming.y} r={3 * unit} fill="#ffffff" stroke={SELECTION} strokeWidth={unit} /> : null}
            <rect x={point.x - 3.5 * unit} y={point.y - 3.5 * unit} width={7 * unit} height={7 * unit} fill="#ffffff" stroke={SELECTION} strokeWidth={unit} />
          </g>
        )
      })}
    </svg>
  )
}

function NumberField({ label, value, onChange }: { label: string; value: number; onChange: (value: number) => void }) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-koma-dim">{label}</span>
      <input
        type="number"
        min={0}
        value={value}
        onChange={(event) => {
          const next = Number(event.target.value)
          onChange(Number.isFinite(next) && next > 0 ? next : 0)
        }}
        className="h-7 rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
      />
    </label>
  )
}

function colorInputHex(value: string, fallback: string): string {
  const source = /^#[0-9a-fA-F]{3,8}$/.test(value) ? value : fallback
  const body = source.slice(1)
  if (/^[0-9a-fA-F]{6}$/.test(body) || /^[0-9a-fA-F]{8}$/.test(body)) return `#${body.slice(0, 6).toLowerCase()}`
  if (/^[0-9a-fA-F]{3}$/.test(body)) return `#${body[0]}${body[0]}${body[1]}${body[1]}${body[2]}${body[2]}`.toLowerCase()
  return '#000000'
}

function PaintRow({ label, value, mixed, fallback, resolved, tokens, weight, onWeight, onChange }: { label: string; value: string; mixed?: boolean; fallback: string; resolved?: string; tokens?: { name: string }[]; weight?: { value: number; mixed?: boolean }; onWeight?: (value: number) => void; onChange: (next: string | null) => void }) {
  const [open, setOpen] = useState(false)
  const [draft, setDraft] = useState<string | null>(null)
  const on = !mixed && value !== 'none'
  const hex = value.startsWith('#') ? value : resolved?.startsWith('#') ? resolved : fallback
  const picker = colorInputHex(hex, fallback)
  const swatch = value.startsWith('#') ? value : resolved?.startsWith('#') ? resolved : on ? 'var(--color-koma-panel)' : 'transparent'
  const shown = mixed ? '' : value === 'none' ? '' : value
  const commitText = (raw: string) => {
    const trimmed = raw.trim()
    setDraft(null)
    if (!trimmed || trimmed.toLowerCase() === 'none') {
      onChange('none')
      return
    }
    const named = tokens?.find((token) => token.name === trimmed)
    if (named) {
      onChange(named.name)
      return
    }
    const body = trimmed.startsWith('#') ? trimmed.slice(1) : trimmed
    if (/^[0-9a-fA-F]{3}$|^[0-9a-fA-F]{6}$|^[0-9a-fA-F]{8}$/.test(body)) onChange(`#${body.toLowerCase()}`)
  }
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-1">
        {tokens?.length ? (
          <button
            type="button"
            aria-label={`${label} swatch`}
            aria-expanded={open}
            onClick={() => setOpen((current) => !current)}
            className="h-7 w-7 flex-none rounded border border-koma-border"
            style={{ background: swatch }}
          />
        ) : (
          <label className="relative h-7 w-7 flex-none overflow-hidden rounded border border-koma-border">
            <span className="absolute inset-0" style={{ background: swatch }} />
            <input
              type="color"
              aria-label={`${label} color`}
              value={picker}
              onChange={(event) => onChange(event.target.value.toLowerCase())}
              className="absolute inset-0 cursor-pointer opacity-0"
            />
          </label>
        )}
        <input
          aria-label={`${label} hex`}
          value={draft ?? shown}
          placeholder={mixed ? 'Mixed' : value === 'none' ? 'None' : '#hex'}
          onChange={(event) => {
            const raw = event.target.value
            const trimmed = raw.trim()
            const named = tokens?.find((token) => token.name === trimmed)
            const body = trimmed.startsWith('#') ? trimmed.slice(1) : trimmed
            if (named) {
              setDraft(null)
              onChange(named.name)
              return
            }
            if (/^[0-9a-fA-F]{6}$|^[0-9a-fA-F]{8}$/.test(body)) {
              setDraft(null)
              onChange(`#${body.toLowerCase()}`)
              return
            }
            setDraft(raw)
          }}
          onBlur={() => {
            if (draft != null) commitText(draft)
          }}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && draft != null) commitText(draft)
          }}
          className="h-7 min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-1.5 text-[12px] text-koma-fg outline-none"
        />
        <button
          type="button"
          aria-label={`${label} visibility`}
          aria-pressed={on}
          title={mixed ? 'Mixed' : on ? 'On' : 'Off'}
          onClick={() => onChange(mixed || on ? 'none' : null)}
          className={`flex h-7 w-7 flex-none items-center justify-center rounded ${on ? 'text-koma-fg' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`}
        >
          {on ? <Eye size={14} /> : <EyeOff size={14} />}
        </button>
        {weight && onWeight ? (
          <div className="w-16 flex-none">
            <GeomField label="W" ariaLabel="Weight" value={weight.value} mixed={weight.mixed} onChange={onWeight} />
          </div>
        ) : null}
      </div>
      {open && tokens?.length ? (
        <div className="flex flex-col gap-1 rounded border border-koma-border p-1">
          <input
            type="color"
            aria-label={`${label} color`}
            value={picker}
            onChange={(event) => onChange(event.target.value.toLowerCase())}
            className="h-7 w-full cursor-pointer bg-transparent"
          />
          {tokens.map((token) => (
            <button
              key={token.name}
              type="button"
              aria-pressed={value === token.name}
              onClick={() => {
                onChange(token.name)
                setOpen(false)
              }}
              className={`h-6 rounded px-1 text-left text-[12px] ${value === token.name ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover'}`}
            >
              {token.name}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  )
}

function Choices<T extends string>({ label, value, mixed, options, onChange }: { label: string; value: T; mixed?: boolean; options: { value: T; label: string }[]; onChange: (value: T) => void }) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-koma-dim">{label}{mixed ? ' · Mixed' : ''}</span>
      <div className="flex gap-0.5">
        {options.map((option) => (
          <button
            key={option.value}
            type="button"
            aria-pressed={!mixed && option.value === value}
            onClick={() => onChange(option.value)}
            className={`h-7 flex-1 rounded text-[12px] ${!mixed && option.value === value ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover'}`}
          >
            {option.label}
          </button>
        ))}
      </div>
    </div>
  )
}
