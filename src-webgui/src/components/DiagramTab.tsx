import { canvasPoint, pagePoint } from '../lib/uiScale'
import { useEffect, useId, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from 'react'
import { Circle, Diamond, HandGrab, MousePointer2, PanelRightClose, PanelRightOpen, RotateCw, Shapes, Spline, Square, Type } from 'lucide-react'
import { useKoma, type Tab } from '../store/koma'
import { recordCodingHistory } from '../lib/coding-recovery'
import { fileKey } from '../store/coding'
import { isTerminalInputTarget } from '../lib/terminalInput'
import { isTabVisible } from '../store/editorGroups'
import { BrailleSpinner } from './BrailleSpinner'
import { showCodingHistory } from './CodingHistory'
import { EditorChrome } from './EditorChrome'
import { Toggle } from './panels/form'
import { DiagramRefMenuItems } from './DiagramVisual'
import { DiagramEdgeView, themeStroke } from './diagram/DiagramEdgeView'
import { DiagramNodeView, TABLE_KEY_WIDTH, type DiagramCell, type DiagramCellMove, type DiagramNodeEdit } from './diagram/DiagramNodeView'
import { Properties, type DiagramPanelSelection } from './diagram/DiagramSettings'
import { DiagramShapePicker } from './diagram/DiagramShapePicker'
import { paletteEntry, type DiagramPaletteEntry } from './diagram/DiagramShapePreview'
import {
  DIAGRAM_DRAG_THRESHOLD,
  DIAGRAM_HIT_TOLERANCE,
  DIAGRAM_UNDO_CAP,
  DIAGRAM_ZOOM_PER_PIXEL,
  DIAGRAM_ZOOM_STEP,
  clampZoom,
  clearDiagramHistory,
  diagramHistory,
  savedDiagramView,
  saveDiagramView,
  zoomViewAt,
  type DiagramView,
} from './diagram/diagramShared'
import {
  addDiagramDocToChat,
  addDiagramSelectionToChat,
  copyDiagramMermaid,
  copyDiagramSelection,
  diagramChatTitle,
} from '../lib/diagramChat'
import { syncDiagramShapeNotes } from '../lib/diagramNoteAttach'
import { diagramFolder, publishDiagramShapeNotes } from '../lib/diagramNotes'
import { looksLikeSql, parseSqlTables, sqlToDiagram } from '../lib/diagramSql'
import { jsonToTable, looksLikeJson } from '../lib/diagramJson'
import {
  DIAGRAM_PORT_SNAP,
  ER_LINE,
  NO_SELECTION,
  TABLE_HEADER,
  TABLE_ROW,
  TABLE_INDENT,
  addChildRow,
  indentRow,
  insertRowAfter,
  isNestType,
  moveRowWithinSiblings,
  outdentRow,
  removeRow,
  tableLineRow,
  tableLines,
  toggleCollapsed,
  updateRow,
  nextKey,
  rowAnchor,
  rowCenterY,
  tableRowAt,
  updateTableRows,
  SHAPE_MIME,
  connectInDirection,
  connectionEnd,
  copyDiagramShapes,
  diagramGuides,
  dropStraightBends,
  edgeStyle,
  insertBend,
  routeCrossings,
  routeFractionAt,
  routePointAt,
  defaultNodeSize,
  defaultCollectionRows,
  defaultTableRows,
  tableHeight,
  deleteDiagramSelection,
  dropGroupHighlight,
  groupHitZone,
  movingIds,
  reparentDragged,
  ungroupNodes,
  edgePoints,
  DIAGRAM_PORTS,
  hitDiagram,
  isDiagramKind,
  mergeSelection,
  moveDiagramNodes,
  nearestPort,
  nearestSide,
  nextRotation,
  nodeAtPoint,
  nodeBounds,
  parseDiagram,
  pointerAngle,
  portAnchor,
  resizeNode,
  rotatePoint,
  nodeCenter,
  selectInRect,
  selectionBounds,
  selectionSize,
  serializeDiagram,
  sideAnchor,
  slideSegment,
  snap,
  type DiagramDirection,
  type DiagramDoc,
  type DiagramEdge,
  type DiagramGuide,
  type DiagramHandle,
  type DiagramKind,
  type DiagramNode,
  type DiagramPoint,
  type DiagramRect,
  type DiagramRow,
  type DiagramSelection,
} from '../lib/diagram'

type Tool = 'select' | 'pan' | 'connect' | DiagramKind
type Drag =
  | {
      kind: 'move'
      ids: string[]
      grab: DiagramNode
      base: DiagramDoc
      sx: number
      sy: number
      pointerId: number
      moved: boolean
      remembered: boolean
    }
  | { kind: 'resize'; id: string; handle: DiagramHandle; sx: number; sy: number; node: DiagramNode; remembered: boolean }
  | { kind: 'rotate'; id: string; rotation: number; angle: number; node: DiagramNode; remembered: boolean }
  | { kind: 'segment'; edgeId: string; index: number; points: DiagramPoint[]; remembered: boolean }
  | { kind: 'anchor'; edgeId: string; end: 'from' | 'to'; remembered: boolean }
  /** `port` null leaves the source end floating, as a drag from a hover arrow does. */
  | { kind: 'connect'; fromId: string; port: number | null; row?: string }
  /** Dragging a row of a selected table up or down reorders it. */
  | { kind: 'row'; tableId: string; rowId: string; base: DiagramRow[]; sy: number; moved: boolean; remembered: boolean }
  | { kind: 'arrow'; sourceId: string; direction: DiagramDirection; sx: number; sy: number }
  | { kind: 'bend'; edgeId: string; index: number; remembered: boolean }
  | { kind: 'label'; edgeId: string; remembered: boolean }
  | { kind: 'pan'; sx: number; sy: number; view: DiagramView }
  | { kind: 'marquee'; x: number; y: number; sx: number; sy: number; base: DiagramSelection; additive: boolean; moved: boolean }

const TOOLS: { id: Tool; label: string; icon: typeof Square }[] = [
  { id: 'select', label: 'Select', icon: MousePointer2 },
  { id: 'pan', label: 'Move canvas', icon: HandGrab },
  { id: 'rect', label: 'Rectangle', icon: Square },
  { id: 'ellipse', label: 'Ellipse', icon: Circle },
  { id: 'diamond', label: 'Diamond', icon: Diamond },
  { id: 'text', label: 'Text', icon: Type },
  { id: 'connect', label: 'Connect', icon: Spline },
]

const HANDLES: { id: DiagramHandle; x: string; y: string; cursor: string }[] = [
  { id: 'nw', x: '0%', y: '0%', cursor: 'nwse-resize' },
  { id: 'n', x: '50%', y: '0%', cursor: 'ns-resize' },
  { id: 'ne', x: '100%', y: '0%', cursor: 'nesw-resize' },
  { id: 'e', x: '100%', y: '50%', cursor: 'ew-resize' },
  { id: 'se', x: '100%', y: '100%', cursor: 'nwse-resize' },
  { id: 's', x: '50%', y: '100%', cursor: 'ns-resize' },
  { id: 'sw', x: '0%', y: '100%', cursor: 'nesw-resize' },
  { id: 'w', x: '0%', y: '50%', cursor: 'ew-resize' },
]

const ARROW_DWELL_MS = 140
const PICKER_DWELL_MS = 500
/** Screen gap between a shape's box and its hover arrows. */
const ARROW_GAP = 14
/** Screen pixels within which a moving edge snaps to another shape's line. */
const GUIDE_TOLERANCE = 4

type Picker =
  | { mode: 'arrow'; x: number; y: number; sourceId: string; direction: DiagramDirection }
  | { mode: 'drop'; x: number; y: number; sourceId: string; port: number | null; row?: string; at: DiagramPoint }
  | { mode: 'canvas'; x: number; y: number; at: DiagramPoint }

const PANEL_DEFAULT = 260
const PANEL_KEY = 'koma.diagram.panelWidth'

function clampPanelWidth(width: number): number {
  return Math.round(Math.min(640, Math.max(220, width)))
}

/** The properties panel width this viewer last chose; storage may be unavailable. */
function readPanelWidth(): number {
  try {
    const saved = Number(localStorage.getItem(PANEL_KEY))
    return Number.isFinite(saved) && saved > 0 ? clampPanelWidth(saved) : PANEL_DEFAULT
  } catch {
    return PANEL_DEFAULT
  }
}

function savePanelWidth(width: number): void {
  try {
    localStorage.setItem(PANEL_KEY, String(width))
  } catch {
    /* storage blocked: the width just resets next time */
  }
}

function mintId(prefix: string): string {
  return `${prefix}${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`
}

/** Last copied shapes and the lines between them. Shared across diagram tabs. */
let clipboard: { nodes: DiagramNode[]; edges: DiagramEdge[] } | null = null

function placeNode(
  doc: DiagramDoc,
  kind: DiagramKind,
  x: number,
  y: number,
  preset?: { w: number; h: number },
  dialect?: 'document',
): DiagramDoc {
  const size = { ...defaultNodeSize(kind), ...preset }
  const node: DiagramNode = {
    id: mintId('n'),
    kind,
    x: snap(x - size.w / 2, doc.grid, doc.snap),
    y: snap(y - size.h / 2, doc.grid, doc.snap),
    ...size,
  }
  if (kind === 'table' && dialect === 'document') {
    node.dialect = dialect
    node.text = 'collection'
    node.w = 220
    node.rows = defaultCollectionRows()
    node.h = tableHeight(node.rows)
  } else if (kind === 'table') node.rows = defaultTableRows()
  return reparentDragged({ ...doc, nodes: [...doc.nodes, node] }, [node.id])
}

/** A palette entry from a drop. Older drags carry a bare kind. */
function droppedEntry(raw: string): Pick<DiagramPaletteEntry, 'kind' | 'size' | 'dialect'> | null {
  return paletteEntry(raw) ?? (isDiagramKind(raw) ? { kind: raw } : null)
}

function tidyNode(node: DiagramNode): DiagramNode {
  const next: DiagramNode = { ...node }
  const painted = next.kind !== 'text'
  if ((next.fill ?? painted) === painted) delete next.fill
  if ((next.stroke ?? painted) === painted) delete next.stroke
  if (!next.fillColor) delete next.fillColor
  if (!next.strokeColor) delete next.strokeColor
  if (!next.rotation) delete next.rotation
  if (!next.detail?.trim()) delete next.detail
  if (!next.dialect) delete next.dialect
  return next
}

function tidyEdge(edge: DiagramEdge): DiagramEdge {
  const next = { ...edge }
  if (next.width === 1.5) delete next.width
  if (!next.color) delete next.color
  if (next.dash === 'solid') delete next.dash
  if (next.start === 'none') delete next.start
  if (next.end === 'arrow') delete next.end
  if (next.corner === 'sharp') delete next.corner
  if (next.route === 'orthogonal') delete next.route
  if (next.jump === 'none') delete next.jump
  if (next.labelAt == null || next.labelAt === 0.5) delete next.labelAt
  if (next.stroke !== false) delete next.stroke
  if (!next.bends?.length) delete next.bends
  if (next.fromPort != null && (!Number.isInteger(next.fromPort) || next.fromPort < 0 || next.fromPort >= DIAGRAM_PORTS)) delete next.fromPort
  if (next.toPort != null && (!Number.isInteger(next.toPort) || next.toPort < 0 || next.toPort >= DIAGRAM_PORTS)) delete next.toPort
  if (next.fromPort != null) delete next.fromSide
  if (next.toPort != null) delete next.toSide
  if (!next.text?.trim()) delete next.text
  if (!next.detail?.trim()) delete next.detail
  return next
}

/** Fields a new line copies from the last line styled in the tab (draw.io currentEdgeStyle). */
const LINE_STYLE_KEYS = ['color', 'width', 'dash', 'start', 'end', 'corner', 'route', 'jump'] as const
type LineStyle = Pick<DiagramEdge, (typeof LINE_STYLE_KEYS)[number]>

function lineStyleOf(edge: DiagramEdge): LineStyle {
  const style: LineStyle = {}
  for (const key of LINE_STYLE_KEYS) if (edge[key] != null) Object.assign(style, { [key]: edge[key] })
  return style
}

/** Drops ids that no longer exist, so undo never restores a selection of ghosts. */
function liveSelection(doc: DiagramDoc, selection: DiagramSelection): DiagramSelection {
  const nodes = new Set(doc.nodes.map((node) => node.id))
  const edges = new Set(doc.edges.map((edge) => edge.id))
  return { nodes: selection.nodes.filter((id) => nodes.has(id)), edges: selection.edges.filter((id) => edges.has(id)) }
}

function sameNodeBox(a: DiagramNode, b: DiagramNode): boolean {
  return a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h && a.rotation === b.rotation
}

export function DiagramTab({ tab }: { tab: Extract<Tab, { kind: 'diagram' }> }) {
  const key = fileKey(tab.root, tab.path)
  const file = useKoma((s) => s.diagram.docs[key])
  const updateDiagram = useKoma((s) => s.updateDiagram)
  const saveDiagram = useKoma((s) => s.saveDiagram)
  const active = useKoma((s) => s.ui.activeTabId === tab.id && isTabVisible(s.ui, tab.id))
  const canvasRef = useRef<HTMLDivElement>(null)
  const dragRef = useRef<Drag | null>(null)
  const labelNoted = useRef(false)
  /** Cmd+V waits one beat so a `CREATE TABLE` paste can win over the shape clipboard. */
  const pasteTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const updateRef = useRef(updateDiagram)
  updateRef.current = updateDiagram
  const [view, setView] = useState<DiagramView>(() => savedDiagramView(key))
  const viewRef = useRef(view)
  const [tool, setTool] = useState<Tool>('select')
  const [selection, setSelectionState] = useState<DiagramSelection>(NO_SELECTION)
  const selectionRef = useRef(selection)
  const [connectFrom, setConnectFrom] = useState<string | null>(null)
  const [editing, setEditingId] = useState<string | null>(null)
  const [editMode, setEditMode] = useState<'select' | 'end'>('select')
  const [cursor, setCursor] = useState<DiagramPoint | null>(null)
  const [rev, setRev] = useState(0)
  const [marquee, setMarquee] = useState<DiagramRect | null>(null)
  const [chatMenu, setChatMenu] = useState<{ x: number; y: number } | null>(null)
  const [hover, setHover] = useState<{ type: 'node' | 'edge'; id: string } | null>(null)
  const [connectDrag, setConnectDrag] = useState<{ fromId: string; port: number | null; row?: string } | null>(null)
  const [editCell, setEditCell] = useState<DiagramCell | null>(null)
  const [hoverRow, setHoverRow] = useState<{ id: string; index: number } | null>(null)
  const [dropParent, setDropParent] = useState<string | null>(null)
  const [guides, setGuides] = useState<DiagramGuide[]>([])
  const [arrowsFor, setArrowsFor] = useState<string | null>(null)
  const arrowsRef = useRef<string | null>(null)
  arrowsRef.current = arrowsFor
  const [picker, setPicker] = useState<Picker | null>(null)
  const pickerTimer = useRef(0)
  const [propsOpen, setPropsOpen] = useState(false)
  const [panelWidth, setPanelWidth] = useState(readPanelWidth)
  const [spaceDown, setSpaceDown] = useState(false)
  const [dragCursor, setDragCursor] = useState<string | null>(null)
  const spaceRef = useRef(false)
  const lineStyleRef = useRef<LineStyle>({})
  const markerId = useId().replace(/:/g, '')

  const setEditing = (id: string | null, mode: 'select' | 'end' = 'select', cell: DiagramCell | null = null) => {
    setEditMode(mode)
    setEditCell(cell)
    setEditingId(id)
  }
  const setSelection = (next: DiagramSelection) => {
    selectionRef.current = next
    setSelectionState(next)
  }
  const applyView = (next: DiagramView) => {
    viewRef.current = next
    saveDiagramView(key, next)
    setView(next)
  }

  useEffect(() => {
    const saved = savedDiagramView(key)
    viewRef.current = saved
    setView(saved)
    selectionRef.current = NO_SELECTION
    setSelectionState(NO_SELECTION)
    setMarquee(null)
    setChatMenu(null)
    setHover(null)
    setConnectDrag(null)
    setDragCursor(null)
    setRev((value) => value + 1)
  }, [key])

  const history = diagramHistory(key)

  const note = (doc: DiagramDoc) => {
    history.past.push({ doc, selection: selectionRef.current })
    if (history.past.length > DIAGRAM_UNDO_CAP) history.past.shift()
    history.future = []
    setRev((value) => value + 1)
  }
  const noteRef = useRef(note)
  noteRef.current = note

  const currentDoc = () => useKoma.getState().diagram.docs[key]?.doc

  const commit = (next: DiagramDoc) => {
    const current = currentDoc()
    if (!current || current === next || serializeDiagram(current) === serializeDiagram(next)) return
    note(current)
    updateDiagram(tab.root, tab.path, next)
  }

  const travel = (from: 'past' | 'future') => {
    const entry = history[from].pop()
    if (!entry) return
    const current = currentDoc()
    if (current) history[from === 'past' ? 'future' : 'past'].push({ doc: current, selection: selectionRef.current })
    setRev((value) => value + 1)
    setEditing(null)
    setSelection(liveSelection(entry.doc, entry.selection))
    updateDiagram(tab.root, tab.path, entry.doc)
  }
  const undo = () => travel('past')
  const redo = () => travel('future')

  useEffect(() => {
    if (!dragCursor) return
    const previous = document.body.style.cursor
    document.body.style.cursor = dragCursor
    return () => {
      document.body.style.cursor = previous
    }
  }, [dragCursor])

  const toScreen = (clientX: number, clientY: number) => {
    const rect = canvasRef.current?.getBoundingClientRect()
    if (!rect) return null
    return { x: canvasPoint(clientX - rect.left), y: canvasPoint(clientY - rect.top) }
  }
  const toDoc = (e: { clientX: number; clientY: number }) => {
    const screen = toScreen(e.clientX, e.clientY)
    if (!screen) return null
    const v = viewRef.current
    return { x: (screen.x - v.x) / v.zoom, y: (screen.y - v.y) / v.zoom }
  }
  /** Middle of the canvas in document coordinates, where a SQL paste lands. */
  const viewCenter = () => {
    const rect = canvasRef.current?.getBoundingClientRect()
    const v = viewRef.current
    if (!rect) return { x: -v.x / v.zoom, y: -v.y / v.zoom }
    return {
      x: (canvasPoint(rect.width / 2) - v.x) / v.zoom,
      y: (canvasPoint(rect.height / 2) - v.y) / v.zoom,
    }
  }
  const toDocRef = useRef(toDoc)
  toDocRef.current = toDoc

  /** A hover arrow (or Alt+Shift+Arrow): connect to the shape there, or add one and connect it. */
  const arrowConnect = (sourceId: string, direction: DiagramDirection, entry?: DiagramPaletteEntry) => {
    const doc = currentDoc()
    if (!doc) return
    const shape = entry ? { kind: entry.kind, ...defaultNodeSize(entry.kind), ...entry.size } : undefined
    const result = connectInDirection(doc, sourceId, direction, mintId, shape, lineStyleRef.current)
    if (!result) return
    const kinds = new Set(result.doc.nodes.filter((node) => node.id === sourceId || node.id === result.nodeId).map((node) => node.kind))
    const tables = kinds.size === 1 && kinds.has('table')
    commit(
      tables
        ? { ...result.doc, edges: result.doc.edges.map((edge) => (edge.id === result.edgeId ? { ...edge, ...ER_LINE } : edge)) }
        : result.doc,
    )
    setSelection({ nodes: [result.nodeId], edges: [] })
    setPropsOpen(true)
    setArrowsFor(null)
    setPicker(null)
  }

  const pickShape = (open: Picker, entry: DiagramPaletteEntry) => {
    setPicker(null)
    if (open.mode === 'arrow') {
      arrowConnect(open.sourceId, open.direction, entry)
      return
    }
    const doc = currentDoc()
    if (!doc) return
    const placed = placeNode(doc, entry.kind, open.at.x, open.at.y, entry.size, entry.dialect)
    const node = placed.nodes[placed.nodes.length - 1]
    if (!node) return
    if (open.mode === 'canvas') {
      commit(placed)
      setSelection({ nodes: [node.id], edges: [] })
    } else {
      // The new shape is centered on the drop point, not snapped away from it.
      const centered = { ...node, x: open.at.x - node.w / 2, y: open.at.y - node.h / 2 }
      const source = doc.nodes.find((item) => item.id === open.sourceId)
      const style = source?.kind === 'table' && node.kind === 'table' ? { ...lineStyleRef.current, ...ER_LINE } : lineStyleRef.current
      const edge = tidyEdge({
        ...style,
        id: mintId('e'),
        from: open.sourceId,
        to: node.id,
        fromPort: open.row ? undefined : (open.port ?? undefined),
        fromRow: open.row,
      })
      commit(reparentDragged({ ...placed, nodes: [...doc.nodes, centered], edges: [...doc.edges, edge] }, [node.id]))
      setSelection({ nodes: [node.id], edges: [] })
    }
    setPropsOpen(true)
  }

  const closePicker = useMemo(() => () => setPicker(null), [])

  const actionsRef = useRef({ undo, redo, commit, applyView, setSelection, arrowConnect })
  actionsRef.current = { undo, redo, commit, applyView, setSelection, arrowConnect }

  useEffect(() => {
    const docPoint = (clientX: number, clientY: number) => toDocRef.current({ clientX, clientY })
    const docDelta = (client: number) => canvasPoint(client) / viewRef.current.zoom
    /** Boxes a moving shape may align with: everything else in the viewport. */
    const guideTargets = (doc: DiagramDoc, skip: ReadonlySet<string>) => {
      const canvas = canvasRef.current
      const v = viewRef.current
      if (!canvas) return []
      const view = { x: -v.x / v.zoom, y: -v.y / v.zoom, w: canvas.clientWidth / v.zoom, h: canvas.clientHeight / v.zoom }
      return doc.nodes
        .filter((node) => !skip.has(node.id))
        .map(nodeBounds)
        .filter((box) => box.x < view.x + view.w && box.x + box.w > view.x && box.y < view.y + view.h && box.y + box.h > view.y)
    }
    const apply = (drag: Drag, next: DiagramDoc, current: DiagramDoc) => {
      if (next === current) return
      if ('remembered' in drag && !drag.remembered) {
        noteRef.current(current)
        drag.remembered = true
      }
      updateRef.current(tab.root, tab.path, next)
    }
    const move = (e: PointerEvent) => {
      const drag = dragRef.current
      if (!drag) return
      if (drag.kind === 'pan') {
        actionsRef.current.applyView({
          ...drag.view,
          x: drag.view.x + canvasPoint(e.clientX - drag.sx),
          y: drag.view.y + canvasPoint(e.clientY - drag.sy),
        })
        return
      }
      if (drag.kind === 'marquee') {
        if (!drag.moved && Math.hypot(canvasPoint(e.clientX - drag.sx), canvasPoint(e.clientY - drag.sy)) < DIAGRAM_DRAG_THRESHOLD) return
        drag.moved = true
        const point = docPoint(e.clientX, e.clientY)
        const doc = useKoma.getState().diagram.docs[key]?.doc
        if (!point || !doc) return
        const rect = {
          x: Math.min(drag.x, point.x),
          y: Math.min(drag.y, point.y),
          w: Math.abs(point.x - drag.x),
          h: Math.abs(point.y - drag.y),
        }
        setMarquee(rect)
        const inside = selectInRect(doc, rect)
        actionsRef.current.setSelection(drag.additive ? mergeSelection(drag.base, inside, false) : inside)
        return
      }
      if (drag.kind === 'connect') {
        const point = docPoint(e.clientX, e.clientY)
        if (point) setCursor(point)
        return
      }
      if (drag.kind === 'arrow') {
        // Dragging a hover arrow turns into a connection with a floating source end.
        if (Math.hypot(canvasPoint(e.clientX - drag.sx), canvasPoint(e.clientY - drag.sy)) < DIAGRAM_DRAG_THRESHOLD) return
        window.clearTimeout(pickerTimer.current)
        setPicker(null)
        setArrowsFor(null)
        dragRef.current = { kind: 'connect', fromId: drag.sourceId, port: null }
        setConnectDrag({ fromId: drag.sourceId, port: null })
        setDragCursor('crosshair')
        setCursor(docPoint(e.clientX, e.clientY))
        return
      }
      const current = useKoma.getState().diagram.docs[key]
      if (!current) return
      const doc = current.doc
      if (drag.kind === 'move') {
        if (!drag.moved) {
          if (Math.hypot(canvasPoint(e.clientX - drag.sx), canvasPoint(e.clientY - drag.sy)) < DIAGRAM_DRAG_THRESHOLD) return
          drag.moved = true
          setDragCursor('move')
          try {
            canvasRef.current?.setPointerCapture(drag.pointerId)
          } catch {
            /* the pointer may already be gone */
          }
        }
        // The grabbed shape snaps; the rest of the selection follows by the same amount.
        // Alignment guides beat the grid on their axis, and Alt turns both off (draw.io).
        const rawX = docDelta(e.clientX - drag.sx)
        const rawY = docDelta(e.clientY - drag.sy)
        const free = e.altKey
        let dx = free ? rawX : snap(drag.grab.x + rawX, doc.grid, doc.snap) - drag.grab.x
        let dy = free ? rawY : snap(drag.grab.y + rawY, doc.grid, doc.snap) - drag.grab.y
        let lines: DiagramGuide[] = []
        const box = selectionBounds(drag.base, { nodes: drag.ids, edges: [] })
        if (!free && box) {
          const aligned = diagramGuides(
            guideTargets(drag.base, new Set(drag.ids)),
            { ...box, x: box.x + rawX, y: box.y + rawY },
            GUIDE_TOLERANCE / viewRef.current.zoom,
          )
          lines = aligned.guides
          if (lines.some((line) => line.axis === 'x')) dx = rawX + aligned.dx
          if (lines.some((line) => line.axis === 'y')) dy = rawY + aligned.dy
        }
        setGuides((prev) => (!prev.length && !lines.length ? prev : lines))
        const moved = reparentDragged(moveDiagramNodes(drag.base, drag.ids, dx, dy), drag.ids)
        const grabbed = doc.nodes.find((node) => node.id === drag.grab.id)
        const nextGrab = moved.nodes.find((node) => node.id === drag.grab.id)
        const highlight = dropGroupHighlight(moved, drag.ids)
        setDropParent((prev) => (prev === highlight ? prev : highlight))
        if (grabbed && nextGrab && grabbed.x === nextGrab.x && grabbed.y === nextGrab.y && grabbed.parent === nextGrab.parent) return
        apply(drag, moved, doc)
        return
      }
      if (drag.kind === 'resize') {
        // Shift keeps the ratio, Alt grows around the center and skips grid and guides.
        // Tables keep the height of their rows, so they never take the aspect lock.
        const options = { lock: e.shiftKey && drag.node.kind !== 'table', center: e.altKey }
        const base = drag.node
        let resized = resizeNode(base, drag.handle, docDelta(e.clientX - drag.sx), docDelta(e.clientY - drag.sy), doc.grid, doc.snap && !e.altKey, options)
        let lines: DiagramGuide[] = []
        if (!e.altKey && !base.rotation) {
          // Only the dragged edges align, then the resize is redone to land exactly on them.
          const xs = drag.handle.includes('w') ? [0] : drag.handle.includes('e') ? [1] : []
          const ys = drag.handle.includes('n') ? [0] : drag.handle.includes('s') ? [1] : []
          const aligned = diagramGuides(guideTargets(doc, new Set([drag.id])), resized, GUIDE_TOLERANCE / viewRef.current.zoom, xs, ys)
          lines = aligned.guides
          if (aligned.dx || aligned.dy) {
            const edgeX = drag.handle.includes('w') ? resized.x - base.x : resized.x + resized.w - (base.x + base.w)
            const edgeY = drag.handle.includes('n') ? resized.y - base.y : resized.y + resized.h - (base.y + base.h)
            resized = resizeNode(base, drag.handle, edgeX + aligned.dx, edgeY + aligned.dy, doc.grid, false, options)
          }
        }
        setGuides((prev) => (!prev.length && !lines.length ? prev : lines))
        const existing = doc.nodes.find((node) => node.id === drag.id)
        if (!existing || sameNodeBox(existing, resized)) return
        apply(drag, { ...doc, nodes: doc.nodes.map((node) => (node.id === drag.id ? resized : node)) }, doc)
        return
      }
      const point = docPoint(e.clientX, e.clientY)
      if (!point) return
      if (drag.kind === 'rotate') {
        const rotation = nextRotation(drag.rotation, drag.angle, pointerAngle(drag.node, point), doc.snap) || undefined
        const existing = doc.nodes.find((node) => node.id === drag.id)
        if (!existing || existing.rotation === rotation) return
        apply(drag, { ...doc, nodes: doc.nodes.map((node) => (node.id === drag.id ? { ...node, rotation } : node)) }, doc)
        return
      }
      if (drag.kind === 'row') {
        if (!drag.moved && Math.abs(canvasPoint(e.clientY - drag.sy)) < DIAGRAM_DRAG_THRESHOLD) return
        drag.moved = true
        setDragCursor('ns-resize')
        const table = doc.nodes.find((node) => node.id === drag.tableId)
        if (!table) return
        // A row moves among its own siblings: the target is the sibling under the pointer.
        const lines = tableLines({ ...table, rows: drag.base })
        const own = lines.find((line) => line.row.id === drag.rowId)
        const at = Math.floor((point.y - table.y - TABLE_HEADER) / TABLE_ROW)
        const siblings = lines.filter((line) => line.parentId === own?.parentId).map((line) => line.row.id)
        const under = lines[Math.max(0, Math.min(lines.length - 1, at))]
        const target = under ? siblings.indexOf(under.row.id) : -1
        if (!own || target < 0) return
        const rows = moveRowWithinSiblings(drag.base, drag.rowId, target)
        const visible = tableLines({ ...table, rows }).map((line) => line.row.id)
        if (visible.every((id, i) => tableLines(table)[i]?.row.id === id)) return
        apply(drag, updateTableRows(doc, drag.tableId, rows), doc)
        return
      }
      if (drag.kind === 'bend') {
        const existing = doc.edges.find((edge) => edge.id === drag.edgeId)
        if (!existing?.bends?.[drag.index]) return
        const at = e.altKey ? point : { x: snap(point.x, doc.grid, doc.snap), y: snap(point.y, doc.grid, doc.snap) }
        const bend = existing.bends[drag.index]
        if (bend.x === at.x && bend.y === at.y) return
        const bends = existing.bends.map((item, index) => (index === drag.index ? at : item))
        apply(drag, { ...doc, edges: doc.edges.map((edge) => (edge.id === drag.edgeId ? { ...edge, bends } : edge)) }, doc)
        return
      }
      if (drag.kind === 'label') {
        const existing = doc.edges.find((edge) => edge.id === drag.edgeId)
        const from = existing && doc.nodes.find((node) => node.id === existing.from)
        const to = existing && doc.nodes.find((node) => node.id === existing.to)
        if (!existing || !from || !to) return
        let at = Math.round(routeFractionAt(edgePoints(from, to, existing), point) * 100) / 100
        // The middle is sticky, so a label dragged back lands exactly where it started.
        if (Math.abs(at - 0.5) < 0.03) at = 0.5
        if ((existing.labelAt ?? 0.5) === at) return
        apply(drag, {
          ...doc,
          edges: doc.edges.map((edge) => (edge.id === drag.edgeId ? { ...edge, labelAt: at === 0.5 ? undefined : at } : edge)),
        }, doc)
        return
      }
      if (drag.kind === 'segment') {
        const start = drag.points[drag.index]
        const end = drag.points[drag.index + 1]
        if (!start || !end) return
        const horizontal = Math.abs(end.x - start.x) >= Math.abs(end.y - start.y)
        const slid = slideSegment(drag.points, drag.index, snap(horizontal ? point.y : point.x, doc.grid, doc.snap))
        const bends = slid.slice(1, -1)
        const existing = doc.edges.find((edge) => edge.id === drag.edgeId)
        if (!existing || JSON.stringify(existing.bends ?? []) === JSON.stringify(bends)) return
        apply(drag, { ...doc, edges: doc.edges.map((edge) => (edge.id === drag.edgeId ? tidyEdge({ ...edge, bends }) : edge)) }, doc)
        return
      }
      const edge = doc.edges.find((item) => item.id === drag.edgeId)
      if (!edge) return
      const otherId = drag.end === 'from' ? edge.to : edge.from
      const currentId = drag.end === 'from' ? edge.from : edge.to
      const node = nodeAtPoint(doc.nodes, point, otherId) ?? doc.nodes.find((item) => item.id === currentId)
      if (!node || node.id === otherId) return
      const { port } = connectionEnd(node, point, DIAGRAM_PORT_SNAP / viewRef.current.zoom)
      const same = drag.end === 'from' ? edge.from === node.id && edge.fromPort === port : edge.to === node.id && edge.toPort === port
      if (same) return
      apply(drag, {
        ...doc,
        edges: doc.edges.map((item) =>
          item.id === edge.id
            ? tidyEdge({
                ...item,
                bends: [],
                ...(drag.end === 'from'
                  ? { from: node.id, fromPort: port, fromSide: undefined }
                  : { to: node.id, toPort: port, toSide: undefined }),
              })
            : item,
        ),
      }, doc)
    }
    const finish = () => {
      setDragCursor(null)
      setGuides([])
      setDropParent(null)
      setMarquee(null)
      setConnectDrag(null)
      setCursor(null)
      dragRef.current = null
    }
    const up = (e: PointerEvent) => {
      const drag = dragRef.current
      finish()
      if (drag?.kind === 'marquee' && !drag.moved && !drag.additive) actionsRef.current.setSelection(NO_SELECTION)
      if (drag?.kind === 'move' && !drag.moved && drag.ids.length > 1 && !e.shiftKey) {
        // A click inside a group selection narrows it to the clicked shape.
        actionsRef.current.setSelection({ nodes: [drag.grab.id], edges: [] })
      }
      if (drag?.kind === 'arrow') {
        actionsRef.current.arrowConnect(drag.sourceId, drag.direction)
        return
      }
      if (drag?.kind === 'bend' && drag.remembered) {
        const doc = useKoma.getState().diagram.docs[key]?.doc
        const edge = doc?.edges.find((item) => item.id === drag.edgeId)
        const from = edge && doc?.nodes.find((node) => node.id === edge.from)
        const to = edge && doc?.nodes.find((node) => node.id === edge.to)
        if (!doc || !edge?.bends || !from || !to) return
        const route = edgePoints(from, to, edge)
        const bends = dropStraightBends(route[0], edge.bends, route[route.length - 1])
        if (bends.length !== edge.bends.length) {
          updateRef.current(tab.root, tab.path, { ...doc, edges: doc.edges.map((item) => (item.id === edge.id ? tidyEdge({ ...item, bends }) : item)) })
        }
        return
      }
      if (drag?.kind !== 'connect') return
      const point = docPoint(e.clientX, e.clientY)
      const current = useKoma.getState().diagram.docs[key]?.doc
      if (!point || !current) return
      const target = nodeAtPoint(current.nodes, point, drag.fromId)
      if (!target) {
        // Dropped on empty canvas: offer a shape to put there, already connected.
        setPicker({ mode: 'drop', x: e.clientX, y: e.clientY, sourceId: drag.fromId, port: drag.port, row: drag.row, at: point })
        return
      }
      // A table row takes the line on that row; elsewhere a near port pins it or the end floats.
      const rowIndex = tableRowAt(target, point)
      const toRow = tableLineRow(target, rowIndex)?.id
      const fromRow = drag.row
      const fromPort = fromRow ? undefined : (drag.port ?? undefined)
      const toPort = toRow ? undefined : connectionEnd(target, point, DIAGRAM_PORT_SNAP / viewRef.current.zoom).port
      const source = current.nodes.find((node) => node.id === drag.fromId)
      const style = source?.kind === 'table' && target.kind === 'table' ? { ...lineStyleRef.current, ...ER_LINE } : lineStyleRef.current
      const existing = current.edges.find(
        (edge) =>
          edge.from === drag.fromId &&
          edge.to === target.id &&
          edge.fromPort === fromPort &&
          edge.toPort === toPort &&
          edge.fromRow === fromRow &&
          edge.toRow === toRow,
      )
      if (existing) {
        actionsRef.current.setSelection({ nodes: [], edges: [existing.id] })
        setPropsOpen(true)
        return
      }
      const id = mintId('e')
      noteRef.current(current)
      updateRef.current(tab.root, tab.path, {
        ...current,
        edges: [...current.edges, tidyEdge({ ...style, id, from: drag.fromId, to: target.id, fromPort, toPort, fromRow, toRow })],
      })
      actionsRef.current.setSelection({ nodes: [], edges: [id] })
      setPropsOpen(true)
    }
    // Losing the window mid-drag (alt-tab, a dialog) must not leave a drag or Space stuck on.
    const abandon = () => {
      if (document.visibilityState === 'visible' && document.hasFocus()) return
      finish()
      spaceRef.current = false
      setSpaceDown(false)
    }
    const onRestore = (event: Event) => {
      const detail = (event as CustomEvent<{ root?: string; path?: string }>).detail
      if (detail?.root !== tab.root || detail.path !== tab.path) return
      const previous = useKoma.getState().diagram.docs[key]?.doc ?? null
      if (previous) noteRef.current(previous)
      queueMicrotask(() => {
        const restored = useKoma.getState().diagram.docs[key]?.doc
        if (restored) {
          const st = useKoma.getState()
          publishDiagramShapeNotes(st.req, tab.root, tab.path, restored, { previous })
          void syncDiagramShapeNotes(
            { hostId: st.remoteState.hostId ?? 'local', root: tab.root },
            tab.path,
            restored,
            { previous },
          )
        }
      })
      setEditing(null)
      actionsRef.current.setSelection(NO_SELECTION)
      setConnectFrom(null)
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
    window.addEventListener('pointercancel', finish)
    window.addEventListener('blur', abandon)
    document.addEventListener('visibilitychange', abandon)
    window.addEventListener('koma-diagram-restore', onRestore)
    return () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      window.removeEventListener('pointercancel', finish)
      window.removeEventListener('blur', abandon)
      document.removeEventListener('visibilitychange', abandon)
      window.removeEventListener('koma-diagram-restore', onRestore)
    }
  }, [key, tab.path, tab.root])

  const zoomAround = (factor: number, screen?: DiagramPoint) => {
    const canvas = canvasRef.current
    if (!canvas) return
    const at = screen ?? { x: canvas.clientWidth / 2, y: canvas.clientHeight / 2 }
    const current = viewRef.current
    applyView(zoomViewAt(current, at.x, at.y, current.zoom * factor))
  }
  const fitView = () => {
    const doc = currentDoc()
    const canvas = canvasRef.current
    if (!doc || !canvas) return
    const box = selectionBounds(doc, { nodes: doc.nodes.map((node) => node.id), edges: [] })
    if (!box) {
      applyView({ x: 0, y: 0, zoom: 1 })
      return
    }
    const pad = 40
    const zoom = clampZoom(Math.min(1, (canvas.clientWidth - pad * 2) / Math.max(box.w, 1), (canvas.clientHeight - pad * 2) / Math.max(box.h, 1)))
    applyView({
      zoom,
      x: canvas.clientWidth / 2 - (box.x + box.w / 2) * zoom,
      y: canvas.clientHeight / 2 - (box.y + box.h / 2) * zoom,
    })
  }
  const zoomRef = useRef({ zoomAround, fitView })
  zoomRef.current = { zoomAround, fitView }

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    // Wheel events arrive faster than frames. Deltas pile up and apply once per frame.
    const pending = { panX: 0, panY: 0, zoom: 1, x: 0, y: 0, frame: 0 }
    const flush = () => {
      pending.frame = 0
      let next = viewRef.current
      if (pending.zoom !== 1) next = zoomViewAt(next, pending.x, pending.y, next.zoom * pending.zoom)
      if (pending.panX || pending.panY) next = { ...next, x: next.x - pending.panX, y: next.y - pending.panY }
      pending.panX = 0
      pending.panY = 0
      pending.zoom = 1
      actionsRef.current.applyView(next)
    }
    const onWheel = (event: WheelEvent) => {
      event.preventDefault()
      const unit = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 16 : event.deltaMode === WheelEvent.DOM_DELTA_PAGE ? canvas.clientHeight : 1
      let dx = canvasPoint(event.deltaX * unit)
      let dy = canvasPoint(event.deltaY * unit)
      if (event.ctrlKey || event.metaKey) {
        const rect = canvas.getBoundingClientRect()
        pending.zoom *= Math.exp(-dy * DIAGRAM_ZOOM_PER_PIXEL)
        pending.x = canvasPoint(event.clientX - rect.left)
        pending.y = canvasPoint(event.clientY - rect.top)
      } else {
        if (event.shiftKey && !dx) {
          dx = dy
          dy = 0
        }
        pending.panX += dx
        pending.panY += dy
      }
      if (!pending.frame) pending.frame = requestAnimationFrame(flush)
    }
    canvas.addEventListener('wheel', onWheel, { passive: false })
    return () => {
      canvas.removeEventListener('wheel', onWheel)
      if (pending.frame) cancelAnimationFrame(pending.frame)
    }
  }, [file?.loading])

  const pasteShapes = (source: { nodes: DiagramNode[]; edges: DiagramEdge[] }, shift: boolean) => {
    const doc = currentDoc()
    if (!doc) return null
    const step = shift ? (doc.snap && doc.grid > 0 ? doc.grid : 16) : 0
    const copy = copyDiagramShapes(
      { ...doc, nodes: source.nodes, edges: source.edges },
      { nodes: source.nodes.map((node) => node.id), edges: [] },
      mintId,
      step,
      step,
    )
    if (!copy.nodes.length) return null
    commit({ ...doc, nodes: [...doc.nodes, ...copy.nodes.map(tidyNode)], edges: [...doc.edges, ...copy.edges.map(tidyEdge)] })
    setSelection({ nodes: copy.nodes.map((node) => node.id), edges: copy.edges.map((edge) => edge.id) })
    setEditing(null)
    setConnectFrom(null)
    return copy
  }
  /** `CREATE TABLE` text becomes tables at the middle of the view. Returns false when it is not SQL. */
  const importSql = (text: string) => {
    if (!looksLikeSql(text)) return false
    const doc = currentDoc()
    if (!doc) return false
    const built = sqlToDiagram(parseSqlTables(text), viewCenter(), mintId)
    if (!built.nodes.length) return false
    const next = reparentDragged(
      {
        ...doc,
        nodes: [...doc.nodes, ...built.nodes.map(tidyNode)],
        edges: [...doc.edges, ...built.edges.map(tidyEdge)],
      },
      built.nodes.map((node) => node.id),
    )
    commit(next)
    setSelection({ nodes: built.nodes.map((node) => node.id), edges: built.edges.map((edge) => edge.id) })
    setEditing(null)
    setConnectFrom(null)
    setPropsOpen(true)
    return true
  }
  /** Pasted JSON documents become a document table at the middle of the view. */
  const importJson = (text: string) => {
    if (!looksLikeJson(text)) return false
    const doc = currentDoc()
    if (!doc) return false
    const table = jsonToTable(JSON.parse(text.trim()), viewCenter(), mintId)
    if (!table) return false
    commit(reparentDragged({ ...doc, nodes: [...doc.nodes, tidyNode(table)] }, [table.id]))
    setSelection({ nodes: [table.id], edges: [] })
    setEditing(null)
    setConnectFrom(null)
    setPropsOpen(true)
    return true
  }
  const importSqlRef = useRef(importSql)
  importSqlRef.current = importSql
  const importJsonRef = useRef(importJson)
  importJsonRef.current = importJson
  const copySelection = () => {
    const doc = currentDoc()
    const picked = selectionRef.current
    if (!doc || !picked.nodes.length) return false
    const ids = new Set(picked.nodes)
    clipboard = {
      nodes: doc.nodes.filter((node) => ids.has(node.id)).map((node) => tidyNode({ ...node })),
      edges: doc.edges.filter((edge) => ids.has(edge.from) && ids.has(edge.to)).map((edge) => tidyEdge({ ...edge })),
    }
    return true
  }
  const nudge = (dx: number, dy: number, resize: boolean) => {
    const doc = currentDoc()
    const picked = selectionRef.current.nodes
    if (!doc || !picked.length) return
    if (!resize) {
      commit(moveDiagramNodes(doc, picked, dx, dy))
      return
    }
    const ids = new Set(picked)
    commit({
      ...doc,
      nodes: doc.nodes.map((node) =>
        ids.has(node.id) && !(node.kind === 'table' && dy) ? resizeNode(node, dx ? 'e' : 's', dx, dy, doc.grid, false) : node,
      ),
    })
  }
  /** Enter or F2 selects the label; typing replaces it, as in draw.io. */
  const startEditing = (id: string, seed?: string) => {
    labelNoted.current = false
    if (seed != null) editNodeText(id, seed)
    setEditing(id, seed != null ? 'end' : 'select')
    setPropsOpen(true)
  }
  const keyActionsRef = useRef({ copySelection, pasteShapes, nudge, startEditing })
  keyActionsRef.current = { copySelection, pasteShapes, nudge, startEditing }

  useEffect(() => {
    if (!active) return
    const onKey = (e: KeyboardEvent) => {
      if (isTerminalInputTarget(e.target)) return
      const target = e.target as HTMLElement | null
      const typing = !!target?.closest('input, textarea, [contenteditable="true"], [contenteditable="plaintext-only"]')
      const mod = e.metaKey || e.ctrlKey
      const lower = e.key.toLowerCase()
      if (e.code === 'Space' && !typing) {
        spaceRef.current = true
        setSpaceDown(true)
        e.preventDefault()
      }
      if (mod && lower === 's' && !e.altKey) {
        e.preventDefault()
        saveDiagram(tab.root, tab.path)
        return
      }
      if (typing) return
      const doc = currentDoc()
      const picked = selectionRef.current
      const actions = actionsRef.current
      const keys = keyActionsRef.current
      if (mod && !e.altKey && lower === 'z') {
        e.preventDefault()
        if (e.shiftKey) actions.redo()
        else actions.undo()
        return
      }
      if (mod && !e.altKey && lower === 'y') {
        e.preventDefault()
        actions.redo()
        return
      }
      if (mod && !e.altKey && (e.key === '=' || e.key === '+')) {
        e.preventDefault()
        zoomRef.current.zoomAround(DIAGRAM_ZOOM_STEP)
        return
      }
      if (mod && !e.altKey && e.key === '-') {
        e.preventDefault()
        zoomRef.current.zoomAround(1 / DIAGRAM_ZOOM_STEP)
        return
      }
      if (mod && !e.altKey && e.key === '0') {
        e.preventDefault()
        zoomRef.current.zoomAround(1 / viewRef.current.zoom)
        return
      }
      if (mod && e.shiftKey && lower === 'h') {
        e.preventDefault()
        zoomRef.current.fitView()
        return
      }
      if (!doc) return
      if (mod && !e.shiftKey && !e.altKey && lower === 'a') {
        e.preventDefault()
        actions.setSelection({ nodes: doc.nodes.map((node) => node.id), edges: doc.edges.map((edge) => edge.id) })
        return
      }
      if (mod && !e.shiftKey && !e.altKey && lower === 'c') {
        if (keys.copySelection()) e.preventDefault()
        return
      }
      if (mod && !e.shiftKey && !e.altKey && lower === 'x') {
        if (!keys.copySelection()) return
        e.preventDefault()
        actions.commit(deleteDiagramSelection(doc, picked))
        actions.setSelection(NO_SELECTION)
        return
      }
      if (mod && !e.shiftKey && !e.altKey && lower === 'v') {
        // Let the paste event see CREATE TABLE text. Shape paste runs unless that import wins.
        if (pasteTimerRef.current) clearTimeout(pasteTimerRef.current)
        pasteTimerRef.current = setTimeout(() => {
          pasteTimerRef.current = null
          if (!clipboard) return
          const pasted = keys.pasteShapes(clipboard, true)
          // The next paste lands one more step away, as in draw.io.
          if (pasted) clipboard = pasted
          setPropsOpen(true)
        }, 50)
        return
      }
      if (mod && !e.shiftKey && !e.altKey && lower === 'd') {
        if (!picked.nodes.length) return
        e.preventDefault()
        const ids = new Set(picked.nodes)
        keys.pasteShapes(
          {
            nodes: doc.nodes.filter((node) => ids.has(node.id)),
            edges: doc.edges.filter((edge) => ids.has(edge.from) && ids.has(edge.to)),
          },
          true,
        )
        return
      }
      const lone = picked.nodes.length === 1 && !picked.edges.length ? picked.nodes[0] : null
      if (lone && e.altKey && e.shiftKey && !mod && e.key.startsWith('Arrow')) {
        e.preventDefault()
        const direction = ({ ArrowUp: 'n', ArrowRight: 'e', ArrowDown: 's', ArrowLeft: 'w' } as const)[e.key as 'ArrowUp']
        if (direction) actions.arrowConnect(lone, direction)
        return
      }
      if (lone && !mod && !e.altKey && (e.key === 'Enter' || e.key === 'F2')) {
        e.preventDefault()
        keys.startEditing(lone)
        return
      }
      if (lone && !mod && !e.altKey && e.key.length === 1 && e.key !== ' ') {
        e.preventDefault()
        keys.startEditing(lone, e.key)
        return
      }
      if (e.key.startsWith('Arrow') && picked.nodes.length && !e.altKey) {
        e.preventDefault()
        const step = e.shiftKey ? (doc.grid > 0 ? doc.grid : 16) : 1
        const dx = e.key === 'ArrowLeft' ? -step : e.key === 'ArrowRight' ? step : 0
        const dy = e.key === 'ArrowUp' ? -step : e.key === 'ArrowDown' ? step : 0
        keys.nudge(dx, dy, mod)
        return
      }
      if (e.key === 'Escape') {
        setConnectFrom(null)
        setConnectDrag(null)
        dragRef.current = null
        actions.setSelection(NO_SELECTION)
        setEditing(null)
        setMarquee(null)
        setChatMenu(null)
        setCursor(null)
        return
      }
      if ((e.key === 'Delete' || e.key === 'Backspace') && selectionSize(picked)) {
        e.preventDefault()
        actions.commit(deleteDiagramSelection(doc, picked))
        actions.setSelection(NO_SELECTION)
        setConnectFrom(null)
      }
    }
    const onKeyUp = (e: KeyboardEvent) => {
      if (e.code !== 'Space') return
      spaceRef.current = false
      setSpaceDown(false)
    }
    window.addEventListener('keydown', onKey)
    window.addEventListener('keyup', onKeyUp)
    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('keyup', onKeyUp)
      spaceRef.current = false
    }
  }, [active, key, saveDiagram, tab.path, tab.root])

  useEffect(() => {
    if (!active) return
    const onPaste = (event: ClipboardEvent) => {
      const target = event.target as HTMLElement | null
      if (isTerminalInputTarget(event.target) || target?.closest?.('input, textarea, [contenteditable="true"], [contenteditable="plaintext-only"]')) return
      const text = event.clipboardData?.getData('text/plain') ?? ''
      // CREATE TABLE text builds SQL tables; JSON documents build a document table.
      if (!importSqlRef.current(text) && !importJsonRef.current(text)) return
      event.preventDefault()
      if (pasteTimerRef.current) clearTimeout(pasteTimerRef.current)
      pasteTimerRef.current = null
    }
    window.addEventListener('paste', onPaste)
    return () => {
      window.removeEventListener('paste', onPaste)
      if (pasteTimerRef.current) clearTimeout(pasteTimerRef.current)
      pasteTimerRef.current = null
    }
  }, [active])

  useEffect(() => {
    if (!chatMenu) return
    const close = () => setChatMenu(null)
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close()
    }
    window.addEventListener('mousedown', close)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', close)
      window.removeEventListener('keydown', onKey)
    }
  }, [chatMenu])

  // Hover is a geometric hit, sampled once per frame.
  const hoverFrame = useRef(0)
  const hoverPoint = useRef<DiagramPoint | null>(null)
  useEffect(() => () => cancelAnimationFrame(hoverFrame.current), [])
  const [altHeld, setAltHeld] = useState(false)
  const trackHover = (e: ReactPointerEvent) => {
    hoverPoint.current = toDoc(e)
    if (e.altKey !== altHeld) setAltHeld(e.altKey)
    if (hoverFrame.current) return
    hoverFrame.current = requestAnimationFrame(() => {
      hoverFrame.current = 0
      const point = hoverPoint.current
      const doc = currentDoc()
      if (!point || !doc || dragRef.current) return
      const zoom = viewRef.current.zoom
      let hit = hitDiagram(doc, point, DIAGRAM_HIT_TOLERANCE / zoom)
      if (!hit) {
        // Ports sit on the outline, so the hover holds a few pixels past it.
        const margin = 10 / zoom
        const near = [...doc.nodes].reverse().find((node) => {
          const box = nodeBounds(node)
          return point.x >= box.x - margin && point.x <= box.x + box.w + margin && point.y >= box.y - margin && point.y <= box.y + box.h + margin
        })
        if (near) hit = { type: 'node', id: near.id }
      }
      const arrows = arrowsRef.current ? doc.nodes.find((node) => node.id === arrowsRef.current) : undefined
      if (arrows && hit?.type !== 'node') {
        // The arrows sit outside the shape; reaching for one must not hide them.
        const box = nodeBounds(arrows)
        const reach = (ARROW_GAP + 32) / zoom
        if (point.x >= box.x - reach && point.x <= box.x + box.w + reach && point.y >= box.y - reach && point.y <= box.y + box.h + reach) {
          hit = { type: 'node', id: arrows.id }
        }
      }
      setHover((prev) => (prev?.type === hit?.type && prev?.id === hit?.id ? prev : hit))
      const table = hit?.type === 'node' ? doc.nodes.find((node) => node.id === hit.id && node.kind === 'table') : undefined
      const index = table ? tableRowAt(table, point) : null
      const row = table && index != null && index >= 0 ? { id: table.id, index } : null
      setHoverRow((prev) => (prev?.id === row?.id && prev?.index === row?.index ? prev : row))
    })
  }

  // Hover arrows appear after a short dwell, so sweeping across shapes does not flash them.
  useEffect(() => {
    const id = hover?.type === 'node' ? hover.id : null
    if (id === arrowsRef.current) return
    setArrowsFor(null)
    if (!id) return
    const timer = window.setTimeout(() => setArrowsFor(id), ARROW_DWELL_MS)
    return () => window.clearTimeout(timer)
  }, [hover])
  useEffect(() => () => window.clearTimeout(pickerTimer.current), [])

  const addAt = (kind: DiagramKind, x: number, y: number, preset?: { w: number; h: number }, dialect?: 'document') => {
    const doc = currentDoc()
    if (!doc) return
    const next = placeNode(doc, kind, x, y, preset, dialect)
    commit(next)
    const id = next.nodes[next.nodes.length - 1]?.id
    if (id) {
      setSelection({ nodes: [id], edges: [] })
      setPropsOpen(true)
    }
    setTool('select')
    setConnectFrom(null)
  }

  const startPan = (e: ReactPointerEvent) => {
    e.preventDefault()
    dragRef.current = { kind: 'pan', sx: e.clientX, sy: e.clientY, view: viewRef.current }
    setDragCursor('grabbing')
  }

  const onCanvasPointerDown = (e: ReactPointerEvent) => {
    if ((e.target as HTMLElement).closest('input, textarea, [data-diagram-ui]')) return
    if (e.button === 1 || (e.button === 0 && (spaceRef.current || tool === 'pan'))) {
      startPan(e)
      return
    }
    if (e.button !== 0 || !file || file.loading) return
    setChatMenu(null)
    const p = toDoc(e)
    if (!p) return
    if (tool !== 'select' && tool !== 'connect' && tool !== 'pan') {
      addAt(tool, p.x, p.y)
      return
    }
    const doc = file.doc
    const hit = hitDiagram(doc, p, DIAGRAM_HIT_TOLERANCE / viewRef.current.zoom)
    if (editing && hit?.id !== editing) setEditing(null)
    if (tool === 'connect') {
      const node = hit?.type === 'node' ? doc.nodes.find((item) => item.id === hit.id) : undefined
      if (!node) {
        setConnectFrom(null)
        return
      }
      if (!connectFrom) {
        setConnectFrom(node.id)
        setSelection({ nodes: [node.id], edges: [] })
        setPropsOpen(true)
        return
      }
      if (connectFrom !== node.id && !doc.edges.some((edge) => edge.from === connectFrom && edge.to === node.id)) {
        commit({ ...doc, edges: [...doc.edges, tidyEdge({ ...lineStyleRef.current, id: mintId('e'), from: connectFrom, to: node.id })] })
      }
      setConnectFrom(null)
      setSelection({ nodes: [node.id], edges: [] })
      setPropsOpen(true)
      return
    }
    const current = selectionRef.current
    if (!hit) {
      dragRef.current = { kind: 'marquee', x: p.x, y: p.y, sx: e.clientX, sy: e.clientY, base: current, additive: e.shiftKey, moved: false }
      return
    }
    setPropsOpen(true)
    setConnectFrom(null)
    if (hit.type === 'edge') {
      setSelection(e.shiftKey ? mergeSelection(current, { nodes: [], edges: [hit.id] }, true) : { nodes: [], edges: [hit.id] })
      return
    }
    if (editing === hit.id) return
    const zone = doc.nodes.find((node) => node.id === hit.id)
    if (zone?.kind === 'group' && groupHitZone(zone, p) === 'body') {
      dragRef.current = { kind: 'marquee', x: p.x, y: p.y, sx: e.clientX, sy: e.clientY, base: current, additive: e.shiftKey, moved: false }
      return
    }
    const already = current.nodes.includes(hit.id)
    if (e.shiftKey) {
      setSelection(mergeSelection(current, { nodes: [hit.id], edges: [] }, true))
      if (already) return
    } else if (!already) {
      setSelection({ nodes: [hit.id], edges: [] })
    }
    const grab = doc.nodes.find((node) => node.id === hit.id)
    if (!grab) return
    // Rows of an already selected table drag to reorder; the header (or a first click) moves the table.
    const lone = already && !e.shiftKey && current.nodes.length === 1 && !current.edges.length
    const line = grab.kind === 'table' ? tableLines(grab)[tableRowAt(grab, p) ?? -1] : undefined
    if (line && grab.dialect === 'document' && line.row.children?.length) {
      // The chevron in front of a nest's name folds or unfolds it.
      const chevron = grab.x + TABLE_KEY_WIDTH + line.depth * TABLE_INDENT
      if (p.x >= chevron && p.x <= chevron + TABLE_INDENT) {
        commit(updateTableRows(doc, grab.id, toggleCollapsed(grab.rows ?? [], line.row.id)))
        return
      }
    }
    if (lone && line && grab.rows) {
      dragRef.current = { kind: 'row', tableId: grab.id, rowId: line.row.id, base: grab.rows, sy: e.clientY, moved: false, remembered: false }
      return
    }
    const ids = movingIds(doc, e.shiftKey || already ? [...new Set([...selectionRef.current.nodes, hit.id])] : [hit.id])
    dragRef.current = {
      kind: 'move',
      ids,
      grab,
      base: doc,
      sx: e.clientX,
      sy: e.clientY,
      pointerId: e.pointerId,
      moved: false,
      remembered: false,
    }
  }

  const onCanvasDoubleClick = (e: ReactMouseEvent) => {
    if (tool === 'pan' || !file) return
    if ((e.target as HTMLElement).closest('input, textarea, [data-diagram-ui]')) return
    const p = toDoc(e)
    if (!p) return
    const hit = hitDiagram(file.doc, p, DIAGRAM_HIT_TOLERANCE / viewRef.current.zoom)
    if (!hit && tool === 'select') {
      setPicker({ mode: 'canvas', x: e.clientX, y: e.clientY, at: p })
      return
    }
    if (hit?.type !== 'node') return
    const node = file.doc.nodes.find((item) => item.id === hit.id)
    if (node?.kind === 'group' && groupHitZone(node, p) === 'body') return
    dragRef.current = null
    labelNoted.current = false
    setSelection({ nodes: [hit.id], edges: [] })
    setPropsOpen(true)
    const row = node ? tableLineRow(node, tableRowAt(node, p)) : undefined
    if (node && row) {
      // A table cell: the key column cycles PK/FK, the right part edits the type, the rest the name.
      const x = p.x - node.x
      if (x < TABLE_KEY_WIDTH) {
        commit(updateTableRows(file.doc, node.id, updateRow(node.rows ?? [], row.id, (item) => ({ ...item, key: nextKey(item.key) }))))
        return
      }
      setEditing(node.id, 'select', { rowId: row.id, field: x > node.w * 0.6 ? 'type' : 'name' })
      return
    }
    setEditing(hit.id)
  }

  const patchEdge = (id: string, patch: Partial<DiagramEdge>) => {
    const doc = currentDoc()
    if (!doc) return
    const edges = doc.edges.map((edge) => (edge.id === id ? tidyEdge({ ...edge, ...patch }) : edge))
    const styled = edges.find((edge) => edge.id === id)
    if (styled && LINE_STYLE_KEYS.some((key) => key in patch)) lineStyleRef.current = lineStyleOf(styled)
    commit({ ...doc, edges })
  }

  const patchNode = (id: string, patch: Partial<DiagramNode>) => {
    const doc = currentDoc()
    if (!doc) return
    if (patch.rows) {
      // Rows (a dialect switch flattens them) also set the table's height and its lines' rows.
      const { rows, ...rest } = patch
      const restyled = { ...doc, nodes: doc.nodes.map((node) => (node.id === id ? tidyNode({ ...node, ...rest }) : node)) }
      commit(updateTableRows(restyled, id, rows))
      return
    }
    commit({ ...doc, nodes: doc.nodes.map((node) => (node.id === id ? tidyNode({ ...node, ...patch }) : node)) })
  }

  /** Typing groups into one undo step until the field is left. */
  const editLive = (change: (doc: DiagramDoc) => DiagramDoc) => {
    const current = currentDoc()
    if (!current) return
    if (!labelNoted.current) {
      note(current)
      labelNoted.current = true
    }
    updateDiagram(tab.root, tab.path, change(current))
  }
  const editNodeText = (id: string, text: string) =>
    editLive((doc) => ({ ...doc, nodes: doc.nodes.map((node) => (node.id === id ? { ...node, text } : node)) }))
  const editEdgeText = (id: string, text: string) =>
    editLive((doc) => ({ ...doc, edges: doc.edges.map((edge) => (edge.id === id ? { ...edge, text } : edge)) }))
  const editDetail = (kind: 'node' | 'edge', id: string, detail: string) =>
    editLive((doc) =>
      kind === 'node'
        ? { ...doc, nodes: doc.nodes.map((node) => (node.id === id ? { ...node, detail } : node)) }
        : { ...doc, edges: doc.edges.map((edge) => (edge.id === id ? { ...edge, detail } : edge)) },
    )

  const editRows = (id: string, change: (rows: DiagramRow[]) => DiagramRow[]) =>
    editLive((doc) => {
      const table = doc.nodes.find((node) => node.id === id)
      return table ? updateTableRows(doc, id, change(table.rows ?? [])) : doc
    })
  const editCellText = (id: string, cell: DiagramCell, text: string) =>
    editRows(id, (rows) =>
      updateRow(rows, cell.rowId, (row) => (cell.field === 'name' ? { ...row, name: text } : { ...row, type: text.trim() ? text : undefined })),
    )
  // Tab and Enter swap one cell editor for the next; the old one's blur must not end editing.
  const cellMoving = useRef(false)
  const moveCell = (id: string, cell: DiagramCell, move: DiagramCellMove) => {
    const table = currentDoc()?.nodes.find((node) => node.id === id)
    // Tab walks the rows as drawn, nested fields included.
    const lines = table ? tableLines(table) : []
    const index = lines.findIndex((line) => line.row.id === cell.rowId)
    const row = lines[index]?.row
    if (!table || !row || move === 'done') {
      labelNoted.current = false
      setEditing(null)
      return
    }
    cellMoving.current = true
    window.setTimeout(() => {
      cellMoving.current = false
    })
    const document = table.dialect === 'document'
    if (move === 'next' && cell.field === 'name') {
      setEditing(id, 'select', { rowId: cell.rowId, field: 'type' })
    } else if (move === 'next' && lines[index + 1]) {
      setEditing(id, 'select', { rowId: (lines[index + 1] as { row: DiagramRow }).row.id, field: 'name' })
    } else if (move === 'prev' && cell.field === 'type') {
      setEditing(id, 'select', { rowId: cell.rowId, field: 'name' })
    } else if (move === 'prev') {
      if (lines[index - 1]) setEditing(id, 'select', { rowId: (lines[index - 1] as { row: DiagramRow }).row.id, field: 'type' })
    } else if (move === 'indent' || move === 'outdent') {
      if (document) editRows(id, (rows) => (move === 'indent' ? indentRow(rows, cell.rowId) : outdentRow(rows, cell.rowId)))
      setEditing(id, 'end', cell)
    } else if (move === 'delete') {
      editRows(id, (rows) => removeRow(rows, cell.rowId))
      const previous = lines[index - 1] ?? lines[index + 1]
      if (previous) setEditing(id, 'end', { rowId: previous.row.id, field: 'name' })
      else setEditing(null)
    } else {
      // Enter, or Tab past the last cell: a new row below, ready for its name. On a nest
      // in a document table the new field goes inside it, as its first field.
      const fresh: DiagramRow = { id: mintId('r'), name: '' }
      const inside = document && isNestType(row.type)
      editRows(id, (rows) => (inside ? addChildRow(rows, row.id, fresh) : insertRowAfter(rows, row.id, fresh)))
      setEditing(id, 'select', { rowId: fresh.id, field: 'name' })
    }
  }

  // The node views are memoized, so their callbacks must keep one identity.
  const editRef = useRef({ editNodeText, editCellText, moveCell })
  editRef.current = { editNodeText, editCellText, moveCell }
  const nodeEdit = useMemo<DiagramNodeEdit>(
    () => ({
      text: (id, text) => editRef.current.editNodeText(id, text),
      done: () => {
        if (cellMoving.current) return
        labelNoted.current = false
        setEditing(null)
      },
      cellText: (id, cell, text) => editRef.current.editCellText(id, cell, text),
      cellMove: (id, cell, move) => editRef.current.moveCell(id, cell, move),
    }),
    [],
  )

  const revert = () => {
    if (!file?.savedText || file.saving || file.loading) return
    const parsed = parseDiagram(file.savedText)
    if (parsed.error) return
    const current = serializeDiagram(file.doc)
    if (current !== file.savedText) {
      recordCodingHistory(
        { hostId: useKoma.getState().remoteState.hostId ?? 'local', root: tab.root },
        tab.path,
        current,
        'Before revert',
      )
    }
    clearDiagramHistory(key)
    setRev((value) => value + 1)
    setEditing(null)
    const st = useKoma.getState()
    publishDiagramShapeNotes(st.req, tab.root, tab.path, parsed.doc, { previous: file.doc })
    void syncDiagramShapeNotes(
      { hostId: st.remoteState.hostId ?? 'local', root: tab.root },
      tab.path,
      parsed.doc,
      { previous: file.doc },
    )
    updateDiagram(tab.root, tab.path, parsed.doc)
  }

  if (!file) {
    return (
      <div className="flex h-full w-full items-center justify-center text-koma-dim">
        <BrailleSpinner size={18} className="opacity-70" />
      </div>
    )
  }

  const doc = file.doc
  const grid = doc.grid > 0 ? doc.grid : 16
  const byId = new Map(doc.nodes.map((node) => [node.id, node]))
  const connectNode = connectFrom ? byId.get(connectFrom) : undefined
  const selectedNodes = new Set(selection.nodes)
  const selectedEdges = new Set(selection.edges)
  const single: DiagramPanelSelection | null =
    selectionSize(selection) === 1
      ? selection.nodes.length
        ? { type: 'node', id: selection.nodes[0] }
        : { type: 'edge', id: selection.edges[0] }
      : null
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
  void rev
  const z = view.zoom
  const sx = (x: number) => x * z + view.x
  const sy = (y: number) => y * z + view.y
  const screenPoint = (point: DiagramPoint) => ({ x: sx(point.x), y: sy(point.y) })
  const gridPx = grid * z
  const dragging = !!dragCursor
  const hoverNode = hover?.type === 'node' ? byId.get(hover.id) : undefined
  const canvasCursor =
    dragCursor ??
    (tool === 'pan' || spaceDown ? 'grab' : tool !== 'select' ? 'crosshair' : hover?.type === 'edge' ? 'pointer' : hoverNode ? 'move' : 'default')
  const singleNode = single?.type === 'node' ? byId.get(single.id) : undefined
  // Ports and resize handles share the outline, so the selected shape shows only its handles.
  const portsFor =
    tool !== 'select' || editing
      ? undefined
      : connectDrag
        ? connectDrag.row ? undefined : byId.get(connectDrag.fromId)
        : !dragging && hoverNode && hoverNode !== singleNode && hoverNode.kind !== 'table'
          ? hoverNode
          : undefined
  const singleEdge = single?.type === 'edge' ? doc.edges.find((edge) => edge.id === single.id) : undefined
  const connectTarget =
    connectDrag && cursor ? nodeAtPoint(doc.nodes, cursor, connectDrag.fromId) : null
  const routes = doc.edges.map((edge) => {
    const from = byId.get(edge.from)
    const to = byId.get(edge.to)
    return from && to ? edgePoints(from, to, edge) : null
  })
  const hoverRowNode = hoverRow ? byId.get(hoverRow.id) : undefined
  const hoverRowId = hoverRowNode && hoverRow ? tableLineRow(hoverRowNode, hoverRow.index)?.id : undefined
  const arrowsNode =
    arrowsFor && tool === 'select' && !editing && !dragging && !connectDrag && !altHeld && !marquee ? byId.get(arrowsFor) : undefined

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col bg-koma-bg text-koma-fg">
      <EditorChrome
        path={tab.path}
        icon={<Shapes size={13} className="flex-none text-koma-dim" />}
        status={status}
        canSave={canSave}
        canRevert={canRevert}
        saving={file.saving}
        canUndo={history.past.length > 0}
        canRedo={history.future.length > 0}
        onHistory={() => showCodingHistory(tab.root, tab.path)}
        onUndo={undo}
        onRedo={redo}
        onSave={() => saveDiagram(tab.root, tab.path)}
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
      {file.error ? (
        <div className="flex-none border-b border-koma-border px-3 py-1 text-[12px] text-koma-error">{file.error}</div>
      ) : null}
      <div className="flex min-h-0 min-w-0 flex-1">
      <div
        ref={canvasRef}
        className="koma-authored-canvas relative min-h-0 min-w-0 flex-1 overflow-hidden"
        style={{
          ...(doc.snap && gridPx >= 6
            ? {
                backgroundImage:
                  'radial-gradient(circle, color-mix(in srgb, var(--koma-fg, #c8d3f5) 28%, transparent) 1px, transparent 1px)',
                backgroundSize: `${gridPx}px ${gridPx}px`,
                backgroundPosition: `${view.x}px ${view.y}px`,
              }
            : {}),
          cursor: canvasCursor,
        }}
        onPointerDown={onCanvasPointerDown}
        onPointerMove={(e) => {
          trackHover(e)
          if (connectFrom) setCursor(toDoc(e))
        }}
        onPointerLeave={() => setHover(null)}
        onDoubleClick={onCanvasDoubleClick}
        onContextMenu={(e) => {
          if ((e.target as HTMLElement).closest('input, textarea')) return
          e.preventDefault()
          const p = toDoc(e)
          const hit = p ? hitDiagram(doc, p, DIAGRAM_HIT_TOLERANCE / viewRef.current.zoom) : null
          const current = selectionRef.current
          if (hit && !(hit.type === 'node' ? current.nodes : current.edges).includes(hit.id)) {
            setSelection(hit.type === 'node' ? { nodes: [hit.id], edges: [] } : { nodes: [], edges: [hit.id] })
          }
          setChatMenu({ x: e.clientX, y: e.clientY })
        }}
        onDragOver={(e) => {
          if ([...e.dataTransfer.types].some((type) => type === SHAPE_MIME || type === 'text/plain')) {
            e.preventDefault()
            e.dataTransfer.dropEffect = 'copy'
          }
        }}
        onDrop={(e) => {
          const entry = droppedEntry(e.dataTransfer.getData(SHAPE_MIME) || e.dataTransfer.getData('text/plain'))
          if (!entry || file.loading) return
          e.preventDefault()
          const p = toDoc(e)
          if (p) addAt(entry.kind, p.x, p.y, entry.size, entry.dialect)
        }}
      >
        {file.loading ? (
          <div className="absolute inset-0 z-20 flex items-center justify-center">
            <BrailleSpinner size={18} className="opacity-70" />
          </div>
        ) : null}
        <div
          className="pointer-events-none absolute left-0 top-0"
          style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${z})`, transformOrigin: '0 0' }}
        >
          {doc.nodes
            .filter((node) => node.kind === 'group')
            .map((node) => (
              <DiagramNodeView
                key={node.id}
                node={node}
                highlight={connectFrom === node.id || connectTarget?.id === node.id || dropParent === node.id}
                editing={editing === node.id ? editMode : false}
                cell={editing === node.id ? editCell : null}
                edit={nodeEdit}
              />
            ))}
          <svg className="absolute left-0 top-0 h-px w-px overflow-visible">
            {doc.edges.map((edge, index) => {
              const points = routes[index]
              if (!points) return null
              // A jumping line hops over the visible lines drawn before it (draw.io's rule).
              const crossings =
                edge.jump && edge.jump !== 'none' && edge.route !== 'curved'
                  ? routeCrossings(points, routes.slice(0, index).filter((route, i): route is DiagramPoint[] => !!route && doc.edges[i]?.stroke !== false))
                  : undefined
              return (
                <DiagramEdgeView
                  key={edge.id}
                  edge={edge}
                  points={points}
                  selected={selectedEdges.has(edge.id)}
                  markerBase={markerId}
                  crossings={crossings}
                />
              )
            })}
          </svg>
          {doc.edges.map((edge) => {
            const text = edge.text?.trim()
            if (!text) return null
            const from = byId.get(edge.from)
            const to = byId.get(edge.to)
            if (!from || !to) return null
            const at = routePointAt(edgePoints(from, to, edge), edge.labelAt ?? 0.5)
            return (
              <div
                key={`label-${edge.id}`}
                data-diagram-ui=""
                className={`absolute z-10 max-w-[140px] -translate-x-1/2 -translate-y-full truncate rounded border bg-koma-panel px-1.5 text-[11px] leading-5 text-koma-fg ${
                  selectedEdges.has(edge.id) ? 'border-koma-accent' : 'border-koma-border'
                } ${tool === 'select' && !dragging ? 'pointer-events-auto' : ''}`}
                style={{ left: at.x, top: at.y - 4, cursor: 'move' }}
                onPointerDown={(e) => {
                  // The label slides along its line (draw.io label handle).
                  if (e.button !== 0 || spaceRef.current) return
                  e.stopPropagation()
                  e.preventDefault()
                  setSelection({ nodes: [], edges: [edge.id] })
                  setPropsOpen(true)
                  dragRef.current = { kind: 'label', edgeId: edge.id, remembered: false }
                  setDragCursor('move')
                }}
              >
                {text}
              </div>
            )
          })}
          {doc.nodes
            .filter((node) => node.kind !== 'group')
            .map((node) => (
              <DiagramNodeView
                key={node.id}
                node={node}
                highlight={connectFrom === node.id || connectTarget?.id === node.id || dropParent === node.id}
                editing={editing === node.id ? editMode : false}
                cell={editing === node.id ? editCell : null}
                edit={nodeEdit}
              />
            ))}
        </div>
        <svg className="pointer-events-none absolute inset-0 h-full w-full overflow-visible">
          {doc.nodes.map((node) => {
            if (!selectedNodes.has(node.id) || (singleNode && tool === 'select' && !connectDrag)) return null
            const corners = [
              { x: node.x, y: node.y },
              { x: node.x + node.w, y: node.y },
              { x: node.x + node.w, y: node.y + node.h },
              { x: node.x, y: node.y + node.h },
            ].map((point) => screenPoint(rotatePoint(point, nodeCenter(node), node.rotation ?? 0)))
            return (
              <polygon
                key={node.id}
                points={corners.map((point) => `${point.x},${point.y}`).join(' ')}
                fill="none"
                stroke="var(--color-koma-accent)"
                strokeWidth={1}
              />
            )
          })}
          {singleEdge && tool === 'select' ? (
            <EdgeHandles
              edge={singleEdge}
              byId={byId}
              screenPoint={screenPoint}
              dragging={dragging}
              onSegment={(index, points) => {
                dragRef.current = { kind: 'segment', edgeId: singleEdge.id, index, points, remembered: false }
              }}
              onBend={(index, insertAt) => {
                const current = currentDoc()
                if (!current) return
                if (insertAt) {
                  // A virtual bend becomes real the moment it is grabbed.
                  note(current)
                  const edges = current.edges.map((edge) =>
                    edge.id === singleEdge.id ? { ...edge, bends: insertBend(edge.bends, index, insertAt) } : edge,
                  )
                  updateDiagram(tab.root, tab.path, { ...current, edges })
                }
                dragRef.current = { kind: 'bend', edgeId: singleEdge.id, index, remembered: !!insertAt }
                setDragCursor('move')
              }}
              onRemoveBend={(index) => {
                const current = currentDoc()
                const edge = current?.edges.find((item) => item.id === singleEdge.id)
                if (!current || !edge?.bends) return
                const bends = edge.bends.filter((_, i) => i !== index)
                commit({ ...current, edges: current.edges.map((item) => (item.id === edge.id ? tidyEdge({ ...item, bends }) : item)) })
              }}
              onAnchor={(end) => {
                dragRef.current = { kind: 'anchor', edgeId: singleEdge.id, end, remembered: false }
                setDragCursor('crosshair')
              }}
            />
          ) : null}
          {connectNode && cursor ? (
            <line
              x1={sx(sideAnchor(connectNode, nearestSide(connectNode, cursor)).x)}
              y1={sy(sideAnchor(connectNode, nearestSide(connectNode, cursor)).y)}
              x2={sx(cursor.x)}
              y2={sy(cursor.y)}
              stroke="var(--color-koma-accent)"
              strokeWidth={1.5}
              strokeDasharray="4 3"
            />
          ) : null}
          {connectDrag && cursor ? (() => {
            const from = byId.get(connectDrag.fromId)
            if (!from) return null
            const targetEnd = connectTarget ? connectionEnd(connectTarget, cursor, DIAGRAM_PORT_SNAP / z) : null
            const targetRow = connectTarget ? tableRowAt(connectTarget, cursor) : null
            const toward = connectTarget ? nodeCenter(connectTarget) : cursor
            const rowStart = connectDrag.row ? rowAnchor(from, connectDrag.row, toward) : null
            const start = screenPoint(
              rowStart ?? (connectDrag.port == null ? sideAnchor(from, nearestSide(from, toward)) : portAnchor(from, connectDrag.port)),
            )
            const rowEnd =
              connectTarget && targetRow != null && targetRow >= 0
                ? rowAnchor(connectTarget, tableLineRow(connectTarget, targetRow)?.id ?? '', nodeCenter(from))
                : null
            const end = screenPoint(
              rowEnd ??
                (connectTarget && targetEnd
                  ? targetEnd.port != null
                    ? portAnchor(connectTarget, targetEnd.port)
                    : sideAnchor(connectTarget, nearestSide(connectTarget, nodeCenter(from)))
                  : cursor),
            )
            return <line x1={start.x} y1={start.y} x2={end.x} y2={end.y} stroke="var(--color-koma-accent)" strokeWidth={1.5} strokeDasharray="4 3" />
          })() : null}
          {guides.map((guide, index) =>
            guide.axis === 'x' ? (
              <line key={index} x1={sx(guide.at)} x2={sx(guide.at)} y1={sy(guide.from)} y2={sy(guide.to)} stroke="var(--color-koma-accent)" strokeWidth={1} strokeDasharray="3 3" />
            ) : (
              <line key={index} x1={sx(guide.from)} x2={sx(guide.to)} y1={sy(guide.at)} y2={sy(guide.at)} stroke="var(--color-koma-accent)" strokeWidth={1} strokeDasharray="3 3" />
            ),
          )}
          {marquee ? (
            <rect
              x={sx(marquee.x)}
              y={sy(marquee.y)}
              width={Math.max(marquee.w * z, 1)}
              height={Math.max(marquee.h * z, 1)}
              fill="color-mix(in srgb, var(--color-koma-accent) 10%, transparent)"
              stroke="var(--color-koma-accent)"
              strokeWidth={1}
            />
          ) : null}
        </svg>
        {hoverRowNode && hoverRowId && tool === 'select' && !editing && !dragging
          ? (['w', 'e'] as const).map((side) => {
              const y = rowCenterY(hoverRowNode, hoverRowId)
              if (y == null) return null
              const at = screenPoint({ x: side === 'w' ? hoverRowNode.x : hoverRowNode.x + hoverRowNode.w, y })
              return (
                <span
                  key={`row-${side}`}
                  data-diagram-ui=""
                  className="absolute z-10 h-2 w-2 -translate-x-1/2 -translate-y-1/2"
                  style={{ left: at.x, top: at.y, cursor: 'crosshair' }}
                  onPointerDown={(e) => {
                    if (e.button !== 0 || spaceRef.current) return
                    e.stopPropagation()
                    e.preventDefault()
                    dragRef.current = { kind: 'connect', fromId: hoverRowNode.id, port: null, row: hoverRowId }
                    setConnectDrag({ fromId: hoverRowNode.id, port: null, row: hoverRowId })
                    setDragCursor('crosshair')
                    setCursor(toDoc(e))
                    setConnectFrom(null)
                  }}
                >
                  <svg viewBox="0 0 12 12" className="h-full w-full overflow-visible text-koma-accent" aria-hidden="true">
                    <path d="M6 1.5 V10.5 M1.5 6 H10.5" fill="none" stroke="currentColor" strokeWidth="1.5" />
                  </svg>
                </span>
              )
            })
          : null}
        {portsFor
          ? Array.from({ length: DIAGRAM_PORTS }, (_, port) => {
              const at = screenPoint(portAnchor(portsFor, port))
              return (
                <span
                  key={`${portsFor.id}-${port}`}
                  data-diagram-ui=""
                  className="absolute z-10 h-2 w-2 -translate-x-1/2 -translate-y-1/2"
                  style={{ left: at.x, top: at.y, cursor: 'crosshair' }}
                  onPointerDown={(e) => {
                    if (e.button !== 0 || spaceRef.current) return
                    e.stopPropagation()
                    e.preventDefault()
                    dragRef.current = { kind: 'connect', fromId: portsFor.id, port }
                    setConnectDrag({ fromId: portsFor.id, port })
                    setDragCursor('crosshair')
                    setCursor(toDoc(e))
                    setConnectFrom(null)
                  }}
                >
                  <svg viewBox="0 0 12 12" className="h-full w-full overflow-visible text-koma-accent" aria-hidden="true">
                    <path d="M6 1.5 V10.5 M1.5 6 H10.5" fill="none" stroke="currentColor" strokeWidth="1.5" />
                  </svg>
                </span>
              )
            })
          : null}
        {singleNode && tool === 'select' && editing !== singleNode.id && !connectDrag ? (
          <div
            className="pointer-events-none absolute z-10 outline outline-1 outline-koma-accent"
            style={{
              left: sx(singleNode.x),
              top: sy(singleNode.y),
              width: singleNode.w * z,
              height: singleNode.h * z,
              transform: singleNode.rotation ? `rotate(${singleNode.rotation}deg)` : undefined,
            }}
          >
            {HANDLES.filter((handle) => singleNode.kind !== 'table' || handle.id === 'e' || handle.id === 'w').map((handle) => (
              <span
                key={handle.id}
                data-diagram-ui=""
                className={`absolute h-2.5 w-2.5 rounded-full border border-koma-accent bg-koma-panel ${dragging ? '' : 'pointer-events-auto'}`}
                style={{ left: handle.x, top: handle.y, transform: 'translate(-50%, -50%)', cursor: handle.cursor }}
                onPointerDown={(e) => {
                  e.stopPropagation()
                  e.preventDefault()
                  dragRef.current = { kind: 'resize', id: singleNode.id, handle: handle.id, sx: e.clientX, sy: e.clientY, node: singleNode, remembered: false }
                  setDragCursor(handle.cursor)
                }}
              />
            ))}
            {singleNode.kind === 'table' ? null : <button
              type="button"
              title="Rotate"
              aria-label="Rotate"
              data-diagram-ui=""
              className={`absolute -right-3 -top-3 flex h-4 w-4 items-center justify-center rounded-full border border-koma-border bg-koma-panel text-koma-accent ${dragging ? '' : 'pointer-events-auto'}`}
              onPointerDown={(e) => {
                e.stopPropagation()
                e.preventDefault()
                const point = toDoc(e)
                if (!point) return
                setDragCursor('crosshair')
                dragRef.current = {
                  kind: 'rotate',
                  id: singleNode.id,
                  rotation: singleNode.rotation ?? 0,
                  angle: pointerAngle(singleNode, point),
                  node: singleNode,
                  remembered: false,
                }
              }}
            >
              <RotateCw size={10} />
            </button>}
          </div>
        ) : null}
        {arrowsNode ? (
          <HoverArrows
            node={arrowsNode}
            nodes={doc.nodes}
            zoom={z}
            gap={ARROW_GAP + (singleNode === arrowsNode ? 6 : 0)}
            screenPoint={screenPoint}
            onDown={(direction, e) => {
              window.clearTimeout(pickerTimer.current)
              setPicker(null)
              dragRef.current = { kind: 'arrow', sourceId: arrowsNode.id, direction, sx: e.clientX, sy: e.clientY }
            }}
            onEnter={(direction, rect) => {
              window.clearTimeout(pickerTimer.current)
              const sourceId = arrowsNode.id
              pickerTimer.current = window.setTimeout(() => {
                const spot = pickerSpot(direction, rect)
                setPicker({ mode: 'arrow', x: spot.x, y: spot.y, sourceId, direction })
              }, PICKER_DWELL_MS)
            }}
            onLeave={() => window.clearTimeout(pickerTimer.current)}
          />
        ) : null}
        {!file.loading && doc.nodes.length === 0 ? (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center text-[12px] text-koma-fg opacity-35">
            Drag a shape onto the canvas
          </div>
        ) : null}
        <div
          data-diagram-ui=""
          className="absolute bottom-2 right-2 rounded border border-koma-border bg-koma-panel px-1.5 text-[11px] leading-5 text-koma-dim"
          title="Zoom (⌘ + scroll, ⌘0 for 100%, ⇧⌘H to fit)"
          onDoubleClick={() => zoomAround(1 / viewRef.current.zoom)}
        >
          {Math.round(z * 100)}%
        </div>
      </div>
      {propsOpen ? (
        <div
          data-diagram-ui=""
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize properties"
          title="Drag to resize"
          className="w-[5px] flex-none cursor-ew-resize hover:bg-koma-grip"
          onPointerDown={(e) => {
            // Dragging left widens the panel; the width is remembered for this viewer.
            e.preventDefault()
            e.stopPropagation()
            const startX = e.clientX
            const startW = panelWidth
            const onMove = (ev: PointerEvent) => setPanelWidth(clampPanelWidth(startW - pagePoint(ev.clientX - startX)))
            const onUp = (ev: PointerEvent) => {
              window.removeEventListener('pointermove', onMove)
              window.removeEventListener('pointerup', onUp)
              document.body.style.cursor = ''
              document.body.style.userSelect = ''
              savePanelWidth(clampPanelWidth(startW - pagePoint(ev.clientX - startX)))
            }
            document.body.style.cursor = 'ew-resize'
            document.body.style.userSelect = 'none'
            window.addEventListener('pointermove', onMove)
            window.addEventListener('pointerup', onUp)
          }}
          onDoubleClick={() => {
            setPanelWidth(PANEL_DEFAULT)
            savePanelWidth(PANEL_DEFAULT)
          }}
        />
      ) : null}
      {propsOpen ? (
        <aside
          data-diagram-ui=""
          className="flex min-h-0 flex-none flex-col overflow-hidden border-l border-koma-border bg-koma-panel"
          style={{ width: panelWidth }}
          onPointerDown={(e) => e.stopPropagation()}
          onContextMenu={(e) => {
            e.preventDefault()
            e.stopPropagation()
          }}
        >
          <Properties
            doc={doc}
            root={tab.root}
            diagPath={tab.path}
            legacyAssetDir={diagramFolder(tab.path) ?? ''}
            selection={single}
            count={selectionSize(selection)}
            onEdge={(id, patch) => patchEdge(id, patch)}
            onNode={(id, patch) => patchNode(id, patch)}
            onText={(id, text) => editNodeText(id, text)}
            onEdgeText={(id, text) => editEdgeText(id, text)}
            onDetail={(kind, id, detail) => editDetail(kind, id, detail)}
            onRows={(id, rows, live) => {
              const doc = currentDoc()
              if (!doc) return
              if (live) editRows(id, () => rows)
              else commit(updateTableRows(doc, id, rows))
            }}
            onTextDone={() => {
              labelNoted.current = false
            }}
          />
        </aside>
      ) : null}
      </div>
      {picker ? (
        <DiagramShapePicker
          x={picker.x}
          y={picker.y}
          faint={picker.mode === 'arrow'}
          onPick={(entry) => pickShape(picker, entry)}
          onClose={closePicker}
        />
      ) : null}
      {chatMenu ? (
        <div
          className="fixed z-[80] min-w-[160px] rounded border border-koma-border bg-koma-panel py-1 shadow-lg"
          style={{ left: pagePoint(chatMenu.x), top: pagePoint(chatMenu.y) }}
          data-diagram-ui=""
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => e.stopPropagation()}
          onContextMenu={(e) => e.preventDefault()}
        >
          <DiagramRefMenuItems
            onUngroup={
              doc.nodes.some((node) => selection.nodes.includes(node.id) && node.kind === 'group')
                ? () => {
                    const current = currentDoc()
                    if (!current) return
                    const result = ungroupNodes(current, selectionRef.current.nodes)
                    setChatMenu(null)
                    commit(result.doc)
                    setSelection({ nodes: result.kept, edges: [] })
                  }
                : undefined
            }
            onAdd={() => {
              const picked = selectionRef.current
              setChatMenu(null)
              const source = { root: tab.root, path: tab.path }
              if (selectionSize(picked)) addDiagramSelectionToChat(doc, picked, diagramChatTitle(tab.path), source)
              else addDiagramDocToChat(doc, diagramChatTitle(tab.path), 'This diagram is empty.', source)
            }}
            onCopy={() => {
              const picked = selectionRef.current
              setChatMenu(null)
              const source = { root: tab.root, path: tab.path }
              if (selectionSize(picked)) void copyDiagramSelection(doc, picked, diagramChatTitle(tab.path), source)
              else void copyDiagramMermaid(doc, diagramChatTitle(tab.path), 'This diagram is empty.', source)
            }}
          />
        </div>
      ) : null}
      <div className="flex h-8 flex-none items-center gap-1 border-t border-koma-border bg-koma-panel px-2">
        {TOOLS.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            type="button"
            title={label}
            aria-label={label}
            aria-pressed={tool === id}
            onClick={() => {
              setTool(id)
              if (id !== 'connect') setConnectFrom(null)
            }}
            className={`flex h-6 w-6 flex-none items-center justify-center rounded transition ${
              tool === id
                ? 'bg-koma-accent/20 text-koma-accent ring-1 ring-inset ring-koma-accent'
                : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'
            }`}
          >
            <Icon size={15} strokeWidth={tool === id ? 2.25 : 1.6} />
          </button>
        ))}
        <div className="ml-auto flex items-center gap-1.5">
          <span className="text-[12px] text-koma-fg opacity-70">Snap</span>
          <Toggle
            on={doc.snap}
            onChange={(on) => {
              if (file.loading) return
              commit({ ...doc, snap: on })
            }}
          />
        </div>
      </div>
    </div>
  )
}

/** Where the arrow picker opens: just past the arrow, away from the shape. Client pixels. */
function pickerSpot(direction: DiagramDirection, rect: DOMRect): { x: number; y: number } {
  const width = 300
  const height = 90
  if (direction === 'e') return { x: rect.right + 6, y: rect.top + rect.height / 2 - height / 2 }
  if (direction === 'w') return { x: Math.max(4, rect.left - 6 - width), y: rect.top + rect.height / 2 - height / 2 }
  if (direction === 'n') return { x: Math.max(4, rect.left + rect.width / 2 - width / 2), y: Math.max(4, rect.top - 6 - height) }
  return { x: Math.max(4, rect.left + rect.width / 2 - width / 2), y: rect.bottom + 6 }
}

// draw.io's hover arrow outlines (HoverIcons.triangleUp/Right/Down/Left), in their own boxes.
const ARROWS: { direction: DiagramDirection; w: number; h: number; d: string }[] = [
  { direction: 'n', w: 18, h: 28, d: 'M6 26 L12 26 L12 12 L18 12 L9 1 L1 12 L6 12 Z' },
  { direction: 'e', w: 26, h: 18, d: 'M1 6 L14 6 L14 1 L26 9 L14 18 L14 12 L1 12 Z' },
  { direction: 's', w: 18, h: 26, d: 'M6 1 L6 14 L1 14 L9 26 L18 14 L12 14 L12 1 Z' },
  { direction: 'w', w: 28, h: 18, d: 'M1 9 L12 1 L12 6 L26 6 L26 12 L12 12 L12 18 Z' },
]

/**
 * The four arrows around a hovered shape. Click adds a connected copy, a drag starts a
 * line, a short hover offers other shapes. An arrow hides when a neighbor (not much larger
 * than this shape) already sits where it points, as in draw.io.
 */
function HoverArrows({
  node,
  nodes,
  zoom,
  gap,
  screenPoint,
  onDown,
  onEnter,
  onLeave,
}: {
  node: DiagramNode
  nodes: DiagramNode[]
  zoom: number
  gap: number
  screenPoint: (point: DiagramPoint) => DiagramPoint
  onDown: (direction: DiagramDirection, e: ReactPointerEvent) => void
  onEnter: (direction: DiagramDirection, rect: DOMRect) => void
  onLeave: () => void
}) {
  const box = nodeBounds(node)
  const topLeft = screenPoint({ x: box.x, y: box.y })
  const bottomRight = screenPoint({ x: box.x + box.w, y: box.y + box.h })
  const cx = (topLeft.x + bottomRight.x) / 2
  const cy = (topLeft.y + bottomRight.y) / 2
  const blocked = (direction: DiagramDirection) => {
    const reach = (gap + 12) / zoom
    const probe =
      direction === 'n'
        ? { x: box.x + box.w / 2, y: box.y - reach }
        : direction === 's'
          ? { x: box.x + box.w / 2, y: box.y + box.h + reach }
          : direction === 'w'
            ? { x: box.x - reach, y: box.y + box.h / 2 }
            : { x: box.x + box.w + reach, y: box.y + box.h / 2 }
    return nodes.some((other) => {
      if (other.id === node.id) return false
      const b = nodeBounds(other)
      const inside = probe.x >= b.x && probe.x <= b.x + b.w && probe.y >= b.y && probe.y <= b.y + b.h
      return inside && b.w < 3 * box.w && b.h < 3 * box.h
    })
  }
  return (
    <>
      {ARROWS.map((arrow) => {
        if (blocked(arrow.direction)) return null
        const left =
          arrow.direction === 'e' ? bottomRight.x + gap : arrow.direction === 'w' ? topLeft.x - gap - arrow.w : cx - arrow.w / 2
        const top =
          arrow.direction === 's' ? bottomRight.y + gap : arrow.direction === 'n' ? topLeft.y - gap - arrow.h : cy - arrow.h / 2
        return (
          <svg
            key={arrow.direction}
            data-diagram-ui=""
            viewBox={`0 0 ${arrow.w} ${arrow.h}`}
            width={arrow.w}
            height={arrow.h}
            className="absolute z-20 opacity-40 transition-opacity hover:opacity-100"
            style={{ left, top, cursor: 'copy' }}
            onPointerDown={(e) => {
              if (e.button !== 0) return
              e.stopPropagation()
              e.preventDefault()
              onDown(arrow.direction, e)
            }}
            onPointerEnter={(e) => onEnter(arrow.direction, e.currentTarget.getBoundingClientRect())}
            onPointerLeave={onLeave}
          >
            <title>Click to add a connected shape, drag to connect</title>
            <path d={arrow.d} fill="var(--color-koma-accent)" stroke="var(--color-koma-bg)" strokeWidth={1} />
          </svg>
        )
      })}
    </>
  )
}

/**
 * Handles of the selected line, in screen space so they keep their size at any zoom.
 * Elbow lines slide whole segments; straight lines get bends, with faint "virtual" bends at
 * each segment middle that turn real when dragged (draw.io virtualBendsEnabled).
 */
function EdgeHandles({
  edge,
  byId,
  screenPoint,
  dragging,
  onSegment,
  onBend,
  onRemoveBend,
  onAnchor,
}: {
  edge: DiagramEdge
  byId: Map<string, DiagramNode>
  screenPoint: (point: DiagramPoint) => DiagramPoint
  dragging: boolean
  onSegment: (index: number, points: DiagramPoint[]) => void
  /** `insertAt` adds a bend at that doc point first; without it the existing bend moves. */
  onBend: (index: number, insertAt?: DiagramPoint) => void
  onRemoveBend: (index: number) => void
  onAnchor: (end: 'from' | 'to') => void
}) {
  const from = byId.get(edge.from)
  const to = byId.get(edge.to)
  if (!from || !to) return null
  const route = edgePoints(from, to, edge)
  const points = route.map(screenPoint)
  const kind = edgeStyle(edge).route
  const straight = kind === 'straight'
  const active = dragging ? '' : 'pointer-events-auto'
  const grab = (e: ReactPointerEvent, run: () => void) => {
    if (e.button !== 0) return
    e.stopPropagation()
    e.preventDefault()
    run()
  }
  return (
    <g data-diagram-ui="">
      {(kind === 'entity' ? [] : points.slice(0, -1)).map((point, index) => {
        const next = points[index + 1]
        if (!next) return null
        const mid = { x: (point.x + next.x) / 2, y: (point.y + next.y) / 2 }
        if (straight) {
          const docMid = { x: (route[index].x + route[index + 1].x) / 2, y: (route[index].y + route[index + 1].y) / 2 }
          return (
            <circle
              key={`virtual-${index}`}
              className={`${active} opacity-50 hover:opacity-100`}
              cx={mid.x}
              cy={mid.y}
              r={4}
              fill="var(--color-koma-accent)"
              stroke="var(--color-koma-bg)"
              strokeWidth={1}
              style={{ cursor: 'move' }}
              onPointerDown={(e) => grab(e, () => onBend(index, docMid))}
            />
          )
        }
        const horizontal = Math.abs(next.x - point.x) >= Math.abs(next.y - point.y)
        return (
          <circle
            key={`seg-${index}`}
            className={active}
            cx={mid.x}
            cy={mid.y}
            r={4}
            fill="var(--color-koma-accent)"
            stroke="var(--color-koma-bg)"
            strokeWidth={1}
            style={{ cursor: horizontal ? 'ns-resize' : 'ew-resize' }}
            onPointerDown={(e) => grab(e, () => onSegment(index, route))}
          />
        )
      })}
      {straight
        ? (edge.bends ?? []).map((bend, index) => {
            const at = screenPoint(bend)
            return (
              <rect
                key={`bend-${index}`}
                className={active}
                x={at.x - 4}
                y={at.y - 4}
                width={8}
                height={8}
                fill="var(--color-koma-accent)"
                stroke="var(--color-koma-bg)"
                strokeWidth={1}
                style={{ cursor: 'move' }}
                onPointerDown={(e) => grab(e, () => onBend(index))}
                onDoubleClick={(e) => {
                  e.stopPropagation()
                  onRemoveBend(index)
                }}
              >
                <title>Drag to move, double-click to remove</title>
              </rect>
            )
          })
        : null}
      {(['from', 'to'] as const).map((end) => {
        const point = end === 'from' ? points[0] : points[points.length - 1]
        if (!point) return null
        return (
          <circle
            key={end}
            className={active}
            cx={point.x}
            cy={point.y}
            r={5}
            fill="var(--color-koma-bg)"
            stroke={themeStroke(true)}
            strokeWidth={1.5}
            style={{ cursor: 'crosshair' }}
            onPointerDown={(e) => grab(e, () => onAnchor(end))}
          />
        )
      })}
    </g>
  )
}
