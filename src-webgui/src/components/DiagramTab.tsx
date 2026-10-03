import { useEffect, useId, useRef, useState, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react'
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
import { DiagramMarker, DiagramRefMenuItems } from './DiagramVisual'
import { InlineMarkdownInput, MarkdownNote } from './MarkdownNote'
import { inlineToHtml } from '../lib/markdownNote'
import {
  addDiagramAreaToChat,
  addDiagramDocToChat,
  copyDiagramArea,
  copyDiagramMermaid,
  diagramChatTitle,
} from '../lib/diagramChat'
import { pointInDiagramRect, type DiagramRect } from '../lib/diagramMermaid'
import { syncDiagramShapeNotes } from '../lib/diagramNoteAttach'
import { diagramFolder, diagramShapeFolder, publishDiagramShapeNotes, shapeHeadingFor } from '../lib/diagramNotes'
import {
  SHAPE_MIME,
  copyNode,
  defaultNodeSize,
  edgeRoute,
  edgeStyle,
  DIAGRAM_PORTS,
  nearestPort,
  nearestSide,
  nextRotation,
  nodeAtPoint,
  nodeStyle,
  parseDiagram,
  pointerAngle,
  portAnchor,
  portOffset,
  resizeNode,
  routeMidpoint,
  routePath,
  serializeDiagram,
  sideAnchor,
  slideSegment,
  snap,
  type DiagramArrow,
  type DiagramDash,
  type DiagramDoc,
  type DiagramEdge,
  type DiagramHandle,
  type DiagramKind,
  type DiagramNode,
  type DiagramPoint,
  type DiagramRoute,
} from '../lib/diagram'

type Tool = 'select' | 'pan' | 'connect' | DiagramKind
type Selection = { type: 'node' | 'edge'; id: string }
type Drag =
  | { kind: 'move'; id: string; sx: number; sy: number; ox: number; oy: number; moved: boolean; remembered: boolean }
  | { kind: 'resize'; id: string; handle: DiagramHandle; sx: number; sy: number; node: DiagramNode; remembered: boolean }
  | { kind: 'rotate'; id: string; rotation: number; angle: number; node: DiagramNode; remembered: boolean }
  | { kind: 'segment'; edgeId: string; index: number; points: DiagramPoint[]; remembered: boolean }
  | { kind: 'anchor'; edgeId: string; end: 'from' | 'to'; remembered: boolean }
  | { kind: 'connect'; fromId: string; port: number }
  | { kind: 'pan'; sx: number; sy: number; px: number; py: number }
  | { kind: 'marquee'; x: number; y: number }

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

function mintId(prefix: string): string {
  return `${prefix}${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`
}

/** Last copied card. Lines are never stored here. Shared across diagram tabs. */
let copiedShape: DiagramNode | null = null

function placeNode(doc: DiagramDoc, kind: DiagramKind, x: number, y: number): DiagramDoc {
  const size = defaultNodeSize(kind)
  const node: DiagramNode = {
    id: mintId('n'),
    kind,
    x: snap(x - size.w / 2, doc.grid, doc.snap),
    y: snap(y - size.h / 2, doc.grid, doc.snap),
    ...size,
  }
  return { ...doc, nodes: [...doc.nodes, node] }
}

function isKind(value: string): value is DiagramKind {
  return value === 'rect' || value === 'ellipse' || value === 'diamond' || value === 'text'
}

function dashArray(dash: DiagramDash, width: number): string | undefined {
  if (dash === 'dashed') return `${Math.max(4, width * 4)} ${Math.max(3, width * 2.5)}`
  if (dash === 'dotted') return `${Math.max(1, width)} ${Math.max(2, width * 1.8)}`
  return undefined
}

function themeStroke(selected: boolean): string {
  return selected
    ? 'var(--color-koma-accent)'
    : 'color-mix(in srgb, var(--koma-fg, #c8d3f5) 45%, transparent)'
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

export function DiagramTab({ tab }: { tab: Extract<Tab, { kind: 'diagram' }> }) {
  const key = fileKey(tab.root, tab.path)
  const file = useKoma((s) => s.diagram.docs[key])
  const updateDiagram = useKoma((s) => s.updateDiagram)
  const saveDiagram = useKoma((s) => s.saveDiagram)
  const active = useKoma((s) => s.ui.activeTabId === tab.id && isTabVisible(s.ui, tab.id))
  const canvasRef = useRef<HTMLDivElement>(null)
  const dragRef = useRef<Drag | null>(null)
  const pastRef = useRef<DiagramDoc[]>([])
  const futureRef = useRef<DiagramDoc[]>([])
  const labelNoted = useRef(false)
  const updateRef = useRef(updateDiagram)
  updateRef.current = updateDiagram
  const [pan, setPan] = useState({ x: 0, y: 0 })
  const panRef = useRef(pan)
  panRef.current = pan
  const [tool, setTool] = useState<Tool>('select')
  const [selection, setSelection] = useState<Selection | null>(null)
  const [connectFrom, setConnectFrom] = useState<string | null>(null)
  const [editing, setEditing] = useState<string | null>(null)
  const [cursor, setCursor] = useState<{ x: number; y: number } | null>(null)
  const [rev, setRev] = useState(0)
  const [area, setArea] = useState<DiagramRect | null>(null)
  const [chatMenu, setChatMenu] = useState<{ x: number; y: number; area: DiagramRect | null } | null>(null)
  const [hoverId, setHoverId] = useState<string | null>(null)
  const [connectDrag, setConnectDrag] = useState<{ fromId: string; port: number } | null>(null)
  const [propsOpen, setPropsOpen] = useState(false)
  const [spaceDown, setSpaceDown] = useState(false)
  const [dragCursor, setDragCursor] = useState<string | null>(null)
  const spaceRef = useRef(false)
  const markerId = useId().replace(/:/g, '')

  useEffect(() => {
    pastRef.current = []
    futureRef.current = []
    setArea(null)
    setChatMenu(null)
    setHoverId(null)
    setConnectDrag(null)
    setDragCursor(null)
    setRev((value) => value + 1)
  }, [key])

  const note = (doc: DiagramDoc) => {
    pastRef.current.push(doc)
    if (pastRef.current.length > 50) pastRef.current.shift()
    futureRef.current = []
    setRev((value) => value + 1)
  }
  const noteRef = useRef(note)
  noteRef.current = note

  const commit = (next: DiagramDoc) => {
    const current = useKoma.getState().diagram.docs[key]?.doc
    if (!current || serializeDiagram(current) === serializeDiagram(next)) return
    note(current)
    updateDiagram(tab.root, tab.path, next)
  }

  const undo = () => {
    const prev = pastRef.current.pop()
    if (!prev) return
    const current = useKoma.getState().diagram.docs[key]?.doc
    if (current) futureRef.current.push(current)
    setRev((value) => value + 1)
    setEditing(null)
    updateDiagram(tab.root, tab.path, prev)
  }
  const redo = () => {
    const next = futureRef.current.pop()
    if (!next) return
    const current = useKoma.getState().diagram.docs[key]?.doc
    if (current) pastRef.current.push(current)
    setRev((value) => value + 1)
    setEditing(null)
    updateDiagram(tab.root, tab.path, next)
  }
  useEffect(() => {
    if (!dragCursor) return
    const previous = document.body.style.cursor
    document.body.style.cursor = dragCursor
    return () => {
      document.body.style.cursor = previous
    }
  }, [dragCursor])

  const undoRef = useRef(undo)
  const redoRef = useRef(redo)
  undoRef.current = undo
  redoRef.current = redo

  useEffect(() => {
    const docPoint = (clientX: number, clientY: number) => {
      const rect = canvasRef.current?.getBoundingClientRect()
      if (!rect) return null
      return { x: clientX - rect.left - panRef.current.x, y: clientY - rect.top - panRef.current.y }
    }
    const apply = (drag: Drag, next: DiagramDoc, current: DiagramDoc) => {
      if (serializeDiagram(current) === serializeDiagram(next)) return
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
        setPan({ x: drag.px + (e.clientX - drag.sx), y: drag.py + (e.clientY - drag.sy) })
        return
      }
      if (drag.kind === 'marquee') {
        const point = docPoint(e.clientX, e.clientY)
        if (!point) return
        setArea({
          x: Math.min(drag.x, point.x),
          y: Math.min(drag.y, point.y),
          w: Math.abs(point.x - drag.x),
          h: Math.abs(point.y - drag.y),
        })
        return
      }
      if (drag.kind === 'connect') {
        const point = docPoint(e.clientX, e.clientY)
        if (point) setCursor(point)
        return
      }
      const current = useKoma.getState().diagram.docs[key]
      if (!current) return
      const doc = current.doc
      if (drag.kind === 'move') {
        const dx = e.clientX - drag.sx
        const dy = e.clientY - drag.sy
        if (!drag.moved) {
          if (Math.abs(dx) < 3 && Math.abs(dy) < 3) return
          drag.moved = true
        }
        apply(drag, {
          ...doc,
          nodes: doc.nodes.map((node) =>
            node.id === drag.id
              ? { ...node, x: snap(drag.ox + dx, doc.grid, doc.snap), y: snap(drag.oy + dy, doc.grid, doc.snap) }
              : node,
          ),
        }, doc)
        return
      }
      if (drag.kind === 'resize') {
        const dx = e.clientX - drag.sx
        const dy = e.clientY - drag.sy
        const resized = resizeNode(drag.node, drag.handle, dx, dy, doc.grid, doc.snap)
        apply(drag, { ...doc, nodes: doc.nodes.map((node) => (node.id === drag.id ? resized : node)) }, doc)
        return
      }
      const point = docPoint(e.clientX, e.clientY)
      if (!point) return
      if (drag.kind === 'rotate') {
        const rotation = nextRotation(drag.rotation, drag.angle, pointerAngle(drag.node, point), doc.snap) || undefined
        apply(drag, {
          ...doc,
          nodes: doc.nodes.map((node) => (node.id === drag.id ? { ...node, rotation } : node)),
        }, doc)
        return
      }
      if (drag.kind === 'segment') {
        const start = drag.points[drag.index]
        const end = drag.points[drag.index + 1]
        if (!start || !end) return
        const horizontal = Math.abs(end.x - start.x) >= Math.abs(end.y - start.y)
        const slid = slideSegment(drag.points, drag.index, snap(horizontal ? point.y : point.x, doc.grid, doc.snap))
        apply(drag, {
          ...doc,
          edges: doc.edges.map((edge) => (edge.id === drag.edgeId ? tidyEdge({ ...edge, bends: slid.slice(1, -1) }) : edge)),
        }, doc)
        return
      }
      const edge = doc.edges.find((item) => item.id === drag.edgeId)
      if (!edge) return
      const otherId = drag.end === 'from' ? edge.to : edge.from
      const currentId = drag.end === 'from' ? edge.from : edge.to
      const node = nodeAtPoint(doc.nodes, point, otherId) ?? doc.nodes.find((item) => item.id === currentId)
      if (!node || node.id === otherId) return
      const port = nearestPort(node, point)
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
    const up = (e: PointerEvent) => {
      setDragCursor(null)
      const drag = dragRef.current
      dragRef.current = null
      if (drag?.kind === 'marquee') {
        setArea((prev) => (prev && (prev.w >= 4 || prev.h >= 4) ? prev : null))
        return
      }
      if (drag?.kind !== 'connect') return
      setConnectDrag(null)
      setCursor(null)
      const point = docPoint(e.clientX, e.clientY)
      const current = useKoma.getState().diagram.docs[key]?.doc
      if (!point || !current) return
      const target = nodeAtPoint(current.nodes, point, drag.fromId)
      if (!target) return
      const toPort = nearestPort(target, point)
      const existing = current.edges.find((edge) => edge.from === drag.fromId && edge.to === target.id && edge.fromPort === drag.port && edge.toPort === toPort)
      if (existing) {
        setSelection({ type: 'edge', id: existing.id })
        setPropsOpen(true)
        return
      }
      const id = mintId('e')
      noteRef.current(current)
      updateRef.current(tab.root, tab.path, {
        ...current,
        edges: [...current.edges, tidyEdge({ id, from: drag.fromId, to: target.id, fromPort: drag.port, toPort })],
      })
      setSelection({ type: 'edge', id })
      setPropsOpen(true)
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
      setSelection(null)
      setConnectFrom(null)
    }
    const cancel = () => {
      setDragCursor(null)
      if (dragRef.current?.kind === 'connect') {
        setConnectDrag(null)
        setCursor(null)
      }
      dragRef.current = null
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
    window.addEventListener('pointercancel', cancel)
    window.addEventListener('koma-diagram-restore', onRestore)
    return () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      window.removeEventListener('pointercancel', cancel)
      window.removeEventListener('koma-diagram-restore', onRestore)
    }
  }, [key, tab.path, tab.root])

  useEffect(() => {
    if (!active) return
    const onKey = (e: KeyboardEvent) => {
      if (isTerminalInputTarget(e.target)) return
      const target = e.target as HTMLElement | null
      const typing = !!target?.closest('input, textarea, [contenteditable="true"]')
      if (e.code === 'Space' && !typing) {
        spaceRef.current = true
        setSpaceDown(true)
        e.preventDefault()
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 's' && !e.altKey) {
        e.preventDefault()
        saveDiagram(tab.root, tab.path)
        return
      }
      if (typing) return
      if ((e.metaKey || e.ctrlKey) && !e.altKey && e.key.toLowerCase() === 'z') {
        e.preventDefault()
        if (e.shiftKey) redoRef.current()
        else undoRef.current()
        return
      }
      if ((e.metaKey || e.ctrlKey) && !e.altKey && e.key.toLowerCase() === 'y') {
        e.preventDefault()
        redoRef.current()
        return
      }
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 'c') {
        if (selection?.type !== 'node') return
        const node = useKoma.getState().diagram.docs[key]?.doc.nodes.find((item) => item.id === selection.id)
        if (!node) return
        e.preventDefault()
        copiedShape = tidyNode({ ...node })
        return
      }
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 'v') {
        const entry = useKoma.getState().diagram.docs[key]
        if (!copiedShape || !entry || entry.loading) return
        e.preventDefault()
        const step = entry.doc.snap && entry.doc.grid > 0 ? entry.doc.grid : 16
        const pasted = tidyNode(copyNode(copiedShape, mintId('n'), step, step))
        copiedShape = pasted
        commit({ ...entry.doc, nodes: [...entry.doc.nodes, pasted] })
        setSelection({ type: 'node', id: pasted.id })
        setEditing(null)
        setConnectFrom(null)
        setPropsOpen(true)
        return
      }
      if (e.key === 'Escape') {
        setConnectFrom(null)
        setConnectDrag(null)
        dragRef.current = null
        setSelection(null)
        setEditing(null)
        setArea(null)
        setChatMenu(null)
        setCursor(null)
        return
      }
      if ((e.key === 'Delete' || e.key === 'Backspace') && selection && file) {
        e.preventDefault()
        const doc = file.doc
        if (selection.type === 'node') {
          commit({
            ...doc,
            nodes: doc.nodes.filter((node) => node.id !== selection.id),
            edges: doc.edges.filter((edge) => edge.from !== selection.id && edge.to !== selection.id),
          })
        } else {
          commit({ ...doc, edges: doc.edges.filter((edge) => edge.id !== selection.id) })
        }
        setSelection(null)
        setConnectFrom(null)
      }
    }
    const onKeyUp = (e: KeyboardEvent) => {
      if (e.code !== 'Space') return
      spaceRef.current = false
      setSpaceDown(false)
    }
    const onBlur = () => {
      spaceRef.current = false
      setSpaceDown(false)
    }
    window.addEventListener('keydown', onKey)
    window.addEventListener('keyup', onKeyUp)
    window.addEventListener('blur', onBlur)
    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('keyup', onKeyUp)
      window.removeEventListener('blur', onBlur)
      spaceRef.current = false
    }
  }, [active, file, key, saveDiagram, selection, tab.path, tab.root, updateDiagram])

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

  const toDoc = (e: { clientX: number; clientY: number }) => {
    const rect = canvasRef.current?.getBoundingClientRect()
    if (!rect) return null
    return { x: e.clientX - rect.left - panRef.current.x, y: e.clientY - rect.top - panRef.current.y }
  }

  const addAt = (kind: DiagramKind, x: number, y: number) => {
    if (!file) return
    const next = placeNode(file.doc, kind, x, y)
    commit(next)
    const id = next.nodes[next.nodes.length - 1]?.id
    if (id) {
      setSelection({ type: 'node', id })
      setPropsOpen(true)
    }
    setTool('select')
    setConnectFrom(null)
  }

  const onCanvasPointerDownCapture = (e: ReactPointerEvent) => {
    if (tool !== 'pan' || e.button !== 0) return
    if ((e.target as HTMLElement).closest('input, textarea')) return
    e.preventDefault()
    e.stopPropagation()
    dragRef.current = { kind: 'pan', sx: e.clientX, sy: e.clientY, px: panRef.current.x, py: panRef.current.y }
    setDragCursor('grabbing')
  }

  const onCanvasPointerDown = (e: ReactPointerEvent) => {
    if (e.button === 1 || (e.button === 0 && spaceRef.current)) {
      if ((e.target as HTMLElement).closest('input, textarea')) return
      e.preventDefault()
      dragRef.current = { kind: 'pan', sx: e.clientX, sy: e.clientY, px: panRef.current.x, py: panRef.current.y }
      setDragCursor('grabbing')
      return
    }
    if (e.button !== 0) return
    if ((e.target as HTMLElement).closest('[data-diagram-node], [data-diagram-edge], [data-diagram-ui]')) return
    setSelection(null)
    setEditing(null)
    const p = toDoc(e)
    if (!p || !file || file.loading) return
    if (tool !== 'select' && tool !== 'connect' && tool !== 'pan') {
      addAt(tool, p.x, p.y)
      return
    }
    if (tool === 'connect') {
      setConnectFrom(null)
      return
    }
    setArea(null)
    setChatMenu(null)
    dragRef.current = { kind: 'marquee', x: p.x, y: p.y }
  }

  const onNodePointerDown = (e: ReactPointerEvent, node: DiagramNode) => {
    if (e.button !== 0 || !file) return
    e.stopPropagation()
    setArea(null)
    if (editing === node.id) return
    if (tool === 'connect') {
      if (!connectFrom) {
        setConnectFrom(node.id)
        setSelection({ type: 'node', id: node.id })
        setPropsOpen(true)
        return
      }
      if (connectFrom !== node.id) {
        const exists = file.doc.edges.some((edge) => edge.from === connectFrom && edge.to === node.id)
        if (!exists) {
          commit({ ...file.doc, edges: [...file.doc.edges, { id: mintId('e'), from: connectFrom, to: node.id }] })
        }
      }
      setConnectFrom(null)
      setSelection({ type: 'node', id: node.id })
      setPropsOpen(true)
      return
    }
    setSelection({ type: 'node', id: node.id })
    setPropsOpen(true)
    setConnectFrom(null)
    dragRef.current = { kind: 'move', id: node.id, sx: e.clientX, sy: e.clientY, ox: node.x, oy: node.y, moved: false, remembered: false }
    setDragCursor('grabbing')
  }

  const onEdgePointerDown = (e: ReactPointerEvent, edge: DiagramEdge) => {
    if (e.button !== 0) return
    e.stopPropagation()
    setArea(null)
    setPropsOpen(true)
    setTool('select')
    setConnectFrom(null)
    setSelection({ type: 'edge', id: edge.id })
  }

  const patchEdge = (id: string, patch: Partial<DiagramEdge>) => {
    if (!file) return
    commit({
      ...file.doc,
      edges: file.doc.edges.map((edge) => (edge.id === id ? tidyEdge({ ...edge, ...patch }) : edge)),
    })
  }

  const patchNode = (id: string, patch: Partial<DiagramNode>) => {
    if (!file) return
    commit({
      ...file.doc,
      nodes: file.doc.nodes.map((node) => (node.id === id ? tidyNode({ ...node, ...patch }) : node)),
    })
  }

  const editNodeText = (id: string, text: string) => {
    const current = useKoma.getState().diagram.docs[key]?.doc
    if (!current) return
    if (!labelNoted.current) {
      note(current)
      labelNoted.current = true
    }
    updateDiagram(tab.root, tab.path, {
      ...current,
      nodes: current.nodes.map((node) => (node.id === id ? { ...node, text } : node)),
    })
  }

  const editEdgeText = (id: string, text: string) => {
    const current = useKoma.getState().diagram.docs[key]?.doc
    if (!current) return
    if (!labelNoted.current) {
      note(current)
      labelNoted.current = true
    }
    updateDiagram(tab.root, tab.path, {
      ...current,
      edges: current.edges.map((edge) => (edge.id === id ? { ...edge, text } : edge)),
    })
  }

  const editDetail = (kind: 'node' | 'edge', id: string, detail: string) => {
    const current = useKoma.getState().diagram.docs[key]?.doc
    if (!current) return
    if (!labelNoted.current) {
      note(current)
      labelNoted.current = true
    }
    updateDiagram(
      tab.root,
      tab.path,
      kind === 'node'
        ? { ...current, nodes: current.nodes.map((node) => (node.id === id ? { ...node, detail } : node)) }
        : { ...current, edges: current.edges.map((edge) => (edge.id === id ? { ...edge, detail } : edge)) },
    )
  }

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
    pastRef.current = []
    futureRef.current = []
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

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col bg-koma-bg text-koma-fg">
      <EditorChrome
        path={tab.path}
        icon={<Shapes size={13} className="flex-none text-koma-dim" />}
        status={status}
        canSave={canSave}
        canRevert={canRevert}
        saving={file.saving}
        canUndo={pastRef.current.length > 0}
        canRedo={futureRef.current.length > 0}
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
        className={`relative min-h-0 min-w-0 flex-1 overflow-hidden ${dragCursor ? '' : tool === 'pan' || spaceDown ? 'cursor-grab' : 'cursor-crosshair'}`}
        style={{
          ...(doc.snap
            ? {
                backgroundImage:
                  'radial-gradient(circle, color-mix(in srgb, var(--koma-fg, #c8d3f5) 28%, transparent) 1px, transparent 1px)',
                backgroundSize: `${grid}px ${grid}px`,
                backgroundPosition: `${pan.x}px ${pan.y}px`,
              }
            : {}),
          ...(dragCursor ? { cursor: dragCursor } : {}),
        }}
        onPointerDownCapture={onCanvasPointerDownCapture}
        onPointerDown={onCanvasPointerDown}
        onContextMenu={(e) => {
          if ((e.target as HTMLElement).closest('input, textarea')) return
          e.preventDefault()
          const p = toDoc(e)
          setChatMenu({ x: e.clientX, y: e.clientY, area: area && p && pointInDiagramRect(p, area) ? area : null })
        }}
        onPointerMove={(e) => {
          if (!connectFrom) return
          setCursor(toDoc(e))
        }}
        onDragOver={(e) => {
          if ([...e.dataTransfer.types].some((type) => type === SHAPE_MIME || type === 'text/plain')) {
            e.preventDefault()
            e.dataTransfer.dropEffect = 'copy'
          }
        }}
        onDrop={(e) => {
          const raw = e.dataTransfer.getData(SHAPE_MIME) || e.dataTransfer.getData('text/plain')
          if (!isKind(raw) || file.loading) return
          e.preventDefault()
          const p = toDoc(e)
          if (p) addAt(raw, p.x, p.y)
        }}
      >
        {file.loading ? (
          <div className="absolute inset-0 z-20 flex items-center justify-center">
            <BrailleSpinner size={18} className="opacity-70" />
          </div>
        ) : null}
        <svg className="pointer-events-none absolute inset-0 h-full w-full overflow-visible">
          <defs>
            {doc.edges.map((edge) => {
              const style = edgeStyle(edge)
              const selected = selection?.type === 'edge' && selection.id === edge.id
              const color = style.color || themeStroke(selected)
              const id = `${markerId}-${edge.id.replace(/[^a-zA-Z0-9_-]/g, '')}`
              return (
                <g key={edge.id}>
                  {style.end === 'none' ? null : <DiagramMarker key="end" id={`${id}-end`} at="end" kind={style.end} color={color} />}
                  {style.start === 'none' ? null : <DiagramMarker key="start" id={`${id}-start`} at="start" kind={style.start} color={color} />}
                </g>
              )
            })}
          </defs>
          {doc.edges.map((edge) => {
            const from = byId.get(edge.from)
            const to = byId.get(edge.to)
            if (!from || !to) return null
            const selected = selection?.type === 'edge' && selection.id === edge.id
            const style = edgeStyle(edge)
            const points = edgeRoute(from, to, edge).map((point) => ({ x: point.x + pan.x, y: point.y + pan.y }))
            const path = routePath(points, style.corner === 'rounded')
            const color = style.color || themeStroke(selected)
            const id = `${markerId}-${edge.id.replace(/[^a-zA-Z0-9_-]/g, '')}`
            return (
              <g key={edge.id} data-diagram-edge="" className="pointer-events-auto" onPointerDown={(e) => onEdgePointerDown(e, edge)}>
                <path d={path} stroke="transparent" strokeWidth={12} fill="none" />
                {style.stroke ? (
                  <path
                    d={path}
                    stroke={color}
                    strokeWidth={style.width}
                    strokeDasharray={dashArray(style.dash, style.width)}
                    strokeLinejoin={style.corner === 'rounded' ? 'round' : 'miter'}
                    fill="none"
                    markerEnd={style.end === 'none' ? undefined : `url(#${id}-${style.end === 'open' ? 'end' : 'end'})`}
                    markerStart={style.start === 'none' ? undefined : `url(#${id}-start)`}
                  />
                ) : null}
                {selected && tool === 'select'
                  ? points.slice(0, -1).map((point, index) => {
                      const next = points[index + 1]
                      if (!next) return null
                      const mid = { x: (point.x + next.x) / 2, y: (point.y + next.y) / 2 }
                      const docPoints = edgeRoute(from, to, edge)
                      const horizontal = Math.abs(next.x - point.x) >= Math.abs(next.y - point.y)
                      return (
                        <circle
                          key={`seg-${index}`}
                          cx={mid.x}
                          cy={mid.y}
                          r={4}
                          fill="var(--color-koma-accent)"
                          stroke="var(--color-koma-bg)"
                          strokeWidth={1}
                          style={{ cursor: horizontal ? 'ns-resize' : 'ew-resize' }}
                          onPointerDown={(e) => {
                            e.stopPropagation()
                            e.preventDefault()
                            dragRef.current = { kind: 'segment', edgeId: edge.id, index, points: docPoints, remembered: false }
                          }}
                        />
                      )
                    })
                  : null}
                {selected && tool === 'select' ? (
                  <>
                    <circle
                      cx={points[0]?.x}
                      cy={points[0]?.y}
                      r={5}
                      fill="var(--color-koma-bg)"
                      stroke="var(--color-koma-accent)"
                      strokeWidth={1.5}
                      style={{ cursor: 'crosshair' }}
                      onPointerDown={(e) => {
                        e.stopPropagation()
                        e.preventDefault()
                        dragRef.current = { kind: 'anchor', edgeId: edge.id, end: 'from', remembered: false }
                      }}
                    />
                    <circle
                      cx={points[points.length - 1]?.x}
                      cy={points[points.length - 1]?.y}
                      r={5}
                      fill="var(--color-koma-bg)"
                      stroke="var(--color-koma-accent)"
                      strokeWidth={1.5}
                      style={{ cursor: 'crosshair' }}
                      onPointerDown={(e) => {
                        e.stopPropagation()
                        e.preventDefault()
                        dragRef.current = { kind: 'anchor', edgeId: edge.id, end: 'to', remembered: false }
                      }}
                    />
                  </>
                ) : null}
              </g>
            )
          })}
          {connectNode && cursor ? (
            <line
              x1={pan.x + sideAnchor(connectNode, nearestSide(connectNode, cursor)).x}
              y1={pan.y + sideAnchor(connectNode, nearestSide(connectNode, cursor)).y}
              x2={pan.x + cursor.x}
              y2={pan.y + cursor.y}
              stroke="var(--color-koma-accent)"
              strokeWidth={1.5}
              strokeDasharray="4 3"
            />
          ) : null}
          {connectDrag && cursor ? (
            <ConnectPreview nodes={doc.nodes} pan={pan} fromId={connectDrag.fromId} port={connectDrag.port} cursor={cursor} />
          ) : null}
        </svg>
        {doc.edges.map((edge) => {
          const text = edge.text?.trim()
          if (!text) return null
          const from = byId.get(edge.from)
          const to = byId.get(edge.to)
          if (!from || !to) return null
          const at = routeMidpoint(edgeRoute(from, to, edge))
          return (
            <div
              key={`label-${edge.id}`}
              className="pointer-events-none absolute z-10 max-w-[140px] -translate-x-1/2 -translate-y-full truncate rounded border border-koma-border bg-koma-panel px-1.5 text-[11px] leading-5 text-koma-fg"
              style={{ left: pan.x + at.x, top: pan.y + at.y - 4 }}
            >
              {text}
            </div>
          )
        })}
        {doc.nodes.map((node) => {
          const selected = selection?.type === 'node' && selection.id === node.id
          const paint = nodeStyle(node)
          const connect = connectFrom === node.id
          const boxed = node.kind === 'rect' || node.kind === 'text'
          const nodeCursor = dragCursor || editing === node.id || tool === 'connect' ? '' : 'cursor-grab'
          return (
            <div
              key={node.id}
              data-diagram-node=""
              className={`absolute flex items-center justify-center ${nodeCursor}`}
              style={{
                left: pan.x + node.x,
                top: pan.y + node.y,
                width: node.w,
                height: node.h,
                transform: node.rotation ? `rotate(${node.rotation}deg)` : undefined,
                ...(boxed
                  ? {
                      background: paint.fill ? paint.fillColor || 'var(--color-koma-panel)' : 'transparent',
                      borderRadius: node.kind === 'rect' ? 4 : 0,
                      outline: connect || paint.stroke
                        ? `1px solid ${connect ? 'var(--color-koma-accent)' : paint.strokeColor || 'var(--color-koma-border)'}`
                        : 'none',
                      outlineOffset: '-0.5px',
                    }
                  : {}),
              }}
              onPointerEnter={() => setHoverId(node.id)}
              onPointerLeave={() => setHoverId((current) => (current === node.id ? null : current))}
              onPointerDown={(e) => onNodePointerDown(e, node)}
              onDoubleClick={(e) => {
                if (tool === 'pan') return
                e.stopPropagation()
                dragRef.current = null
                labelNoted.current = false
                setEditing(node.id)
                setSelection({ type: 'node', id: node.id })
                setPropsOpen(true)
              }}
            >
              {node.kind === 'diamond' || node.kind === 'ellipse' ? (
                <svg className="pointer-events-none absolute inset-0 h-full w-full overflow-visible">
                  {node.kind === 'diamond' ? (
                    <polygon
                      points={`${node.w / 2},0 ${node.w},${node.h / 2} ${node.w / 2},${node.h} 0,${node.h / 2}`}
                      fill={paint.fill ? paint.fillColor || 'var(--color-koma-panel)' : 'none'}
                      stroke={connect ? 'var(--color-koma-accent)' : paint.stroke ? paint.strokeColor || 'var(--color-koma-border)' : 'none'}
                      strokeWidth={connect || paint.stroke ? 1 : 0}
                    />
                  ) : (
                    <ellipse
                      cx={node.w / 2}
                      cy={node.h / 2}
                      rx={node.w / 2}
                      ry={node.h / 2}
                      fill={paint.fill ? paint.fillColor || 'var(--color-koma-panel)' : 'none'}
                      stroke={connect ? 'var(--color-koma-accent)' : paint.stroke ? paint.strokeColor || 'var(--color-koma-border)' : 'none'}
                      strokeWidth={connect || paint.stroke ? 1 : 0}
                    />
                  )}
                </svg>
              ) : null}
              {editing === node.id ? (
                <input
                  autoFocus
                  defaultValue={node.text}
                  className="z-10 w-full border-0 bg-transparent px-2 text-center text-[12px] shadow-none outline-none"
                  style={{ color: paint.textColor || 'var(--color-koma-fg)' }}
                  onPointerDown={(e) => e.stopPropagation()}
                  onChange={(e) => {
                    const text = e.target.value
                    const current = useKoma.getState().diagram.docs[key]?.doc
                    if (!current) return
                    if (!labelNoted.current) {
                      note(current)
                      labelNoted.current = true
                    }
                    updateDiagram(tab.root, tab.path, {
                      ...current,
                      nodes: current.nodes.map((item) => (item.id === node.id ? { ...item, text } : item)),
                    })
                  }}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === 'Escape') {
                      e.preventDefault()
                      setEditing(null)
                    }
                  }}
                  onBlur={() => setEditing(null)}
                />
              ) : (
                <span
                  className="pointer-events-none z-10 truncate px-2 text-center text-[12px]"
                  style={{ color: paint.textColor || 'var(--color-koma-fg)' }}
                  dangerouslySetInnerHTML={{ __html: inlineToHtml(node.text || ' ') }}
                />
              )}
              {tool === 'select' && editing !== node.id && (connectDrag?.fromId === node.id || (hoverId === node.id && (!selected || connectDrag))) ? (
                Array.from({ length: DIAGRAM_PORTS }, (_, port) => {
                  const at = portOffset(node, port)
                  return (
                    <span
                      key={port}
                      data-diagram-port=""
                      className="absolute z-10 h-2 w-2 -translate-x-1/2 -translate-y-1/2"
                      style={{ left: `${at.x * 100}%`, top: `${at.y * 100}%`, cursor: 'crosshair' }}
                      onPointerDown={(e) => {
                        if (e.button !== 0 || spaceRef.current) return
                        e.stopPropagation()
                        e.preventDefault()
                        dragRef.current = { kind: 'connect', fromId: node.id, port }
                        setConnectDrag({ fromId: node.id, port })
                        setDragCursor('crosshair')
                        setCursor(toDoc(e))
                        setConnectFrom(null)
                      }}
                      onDoubleClick={(e) => e.stopPropagation()}
                    >
                      <svg viewBox="0 0 12 12" className="h-full w-full overflow-visible text-koma-accent" aria-hidden="true">
                        <path d="M6 1.5 V10.5 M1.5 6 H10.5" fill="none" stroke="currentColor" strokeWidth="1.5" />
                      </svg>
                    </span>
                  )
                })
              ) : null}
              {selected && tool === 'select' && editing !== node.id && !connectDrag ? (
                <>
                  {HANDLES.map((handle) => (
                    <span
                      key={handle.id}
                      className={`absolute z-10 h-2.5 w-2.5 rounded-full border border-koma-accent bg-koma-panel ${dragCursor ? 'pointer-events-none' : ''}`}
                      style={{ left: handle.x, top: handle.y, transform: 'translate(-50%, -50%)', cursor: handle.cursor }}
                      onPointerDown={(e) => {
                        e.stopPropagation()
                        e.preventDefault()
                        dragRef.current = { kind: 'resize', id: node.id, handle: handle.id, sx: e.clientX, sy: e.clientY, node, remembered: false }
                        setDragCursor(handle.cursor)
                      }}
                      onDoubleClick={(e) => e.stopPropagation()}
                    />
                  ))}
                  <button
                    type="button"
                    title="Rotate"
                    aria-label="Rotate"
                    className={`absolute -right-3 -top-3 z-10 flex h-4 w-4 items-center justify-center rounded-full border border-koma-border bg-koma-panel text-koma-accent ${dragCursor ? 'pointer-events-none' : ''}`}
                    onPointerDown={(e) => {
                      e.stopPropagation()
                      e.preventDefault()
                      const point = toDoc(e)
                      if (!point) return
                      setDragCursor('crosshair')
                      dragRef.current = {
                        kind: 'rotate',
                        id: node.id,
                        rotation: node.rotation ?? 0,
                        angle: pointerAngle(node, point),
                        node,
                        remembered: false,
                      }
                    }}
                  >
                    <RotateCw size={10} />
                  </button>
                </>
              ) : null}
            </div>
          )
        })}
        {area ? (
          <div
            data-diagram-ui=""
            className="pointer-events-none absolute z-20 border border-koma-accent bg-koma-accent/10"
            style={{ left: pan.x + area.x, top: pan.y + area.y, width: Math.max(area.w, 1), height: Math.max(area.h, 1) }}
          />
        ) : null}
        {!file.loading && doc.nodes.length === 0 ? (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center text-[12px] text-koma-fg opacity-35">
            Drag a shape onto the canvas
          </div>
        ) : null}
      </div>
      {propsOpen ? (
        <aside
          data-diagram-ui=""
          className="flex w-[260px] min-h-0 flex-none flex-col overflow-hidden border-l border-koma-border bg-koma-panel"
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
            selection={selection}
            onEdge={(id, patch) => patchEdge(id, patch)}
            onNode={(id, patch) => patchNode(id, patch)}
            onText={(id, text) => editNodeText(id, text)}
            onEdgeText={(id, text) => editEdgeText(id, text)}
            onDetail={(kind, id, detail) => editDetail(kind, id, detail)}
            onTextDone={() => {
              labelNoted.current = false
            }}
          />
        </aside>
      ) : null}
      </div>
      {chatMenu ? (
        <div
          className="fixed z-[80] min-w-[160px] rounded border border-koma-border bg-koma-panel py-1 shadow-lg"
          style={{ left: chatMenu.x, top: chatMenu.y }}
          data-diagram-ui=""
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => e.stopPropagation()}
          onContextMenu={(e) => e.preventDefault()}
        >
          <DiagramRefMenuItems
            onAdd={() => {
              const target = chatMenu.area
              setChatMenu(null)
              const source = { root: tab.root, path: tab.path }
              if (target) addDiagramAreaToChat(doc, target, diagramChatTitle(tab.path), source)
              else addDiagramDocToChat(doc, diagramChatTitle(tab.path), 'This diagram is empty.', source)
            }}
            onCopy={() => {
              const target = chatMenu.area
              setChatMenu(null)
              const source = { root: tab.root, path: tab.path }
              if (target) void copyDiagramArea(doc, target, diagramChatTitle(tab.path), source)
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

function ConnectPreview({
  nodes,
  pan,
  fromId,
  port,
  cursor,
}: {
  nodes: DiagramNode[]
  pan: { x: number; y: number }
  fromId: string
  port: number
  cursor: DiagramPoint
}) {
  const from = nodes.find((node) => node.id === fromId)
  if (!from) return null
  const start = portAnchor(from, port)
  const target = nodeAtPoint(nodes, cursor, fromId)
  const end = target ? portAnchor(target, nearestPort(target, cursor)) : cursor
  return (
    <line
      x1={pan.x + start.x}
      y1={pan.y + start.y}
      x2={pan.x + end.x}
      y2={pan.y + end.y}
      stroke="var(--color-koma-accent)"
      strokeWidth={1.5}
      strokeDasharray="4 3"
    />
  )
}

function LineGlyph({ children }: { children: ReactNode }) {
  return (
    <svg viewBox="0 0 28 14" className="h-3.5 w-7 flex-none" aria-hidden="true">
      {children}
    </svg>
  )
}

const LINE_HEADS: { value: DiagramArrow; name: string }[] = [
  { value: 'none', name: 'No' },
  { value: 'arrow', name: 'Arrow' },
  { value: 'open', name: 'Open arrow' },
  { value: 'diamond', name: 'Diamond' },
  { value: 'diamondOpen', name: 'Open diamond' },
  { value: 'dot', name: 'Dot' },
  { value: 'circle', name: 'Circle' },
  { value: 'bar', name: 'Bar' },
]

function ArrowGlyph({ side, kind }: { side: 'start' | 'end'; kind: DiagramArrow }) {
  const p = (x: number) => (side === 'end' ? x : 28 - x)
  const reach = kind === 'none' ? 24 : kind === 'arrow' || kind === 'open' ? 16 : kind === 'diamond' || kind === 'diamondOpen' ? 14 : kind === 'bar' ? 23 : 18
  const shaft = `M${p(4)} 7 H${p(reach)}`
  const triangle = `M${p(15.5)} 3 L${p(24)} 7 L${p(15.5)} 11`
  const diamond = `M${p(24)} 7 L${p(19)} 3 L${p(14)} 7 L${p(19)} 11 Z`
  return (
    <LineGlyph>
      <path d={shaft} fill="none" stroke="currentColor" strokeWidth="1.5" />
      {kind === 'arrow' ? <path d={`${triangle} Z`} fill="currentColor" /> : null}
      {kind === 'open' ? <path d={triangle} fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="miter" /> : null}
      {kind === 'diamond' ? <path d={diamond} fill="currentColor" /> : null}
      {kind === 'diamondOpen' ? <path d={diamond} fill="none" stroke="currentColor" strokeWidth="1.5" /> : null}
      {kind === 'dot' ? <circle cx={p(21)} cy="7" r="3" fill="currentColor" /> : null}
      {kind === 'circle' ? <circle cx={p(21)} cy="7" r="3" fill="none" stroke="currentColor" strokeWidth="1.5" /> : null}
      {kind === 'bar' ? <path d={`M${p(23)} 3 V11`} fill="none" stroke="currentColor" strokeWidth="1.5" /> : null}
    </LineGlyph>
  )
}

function Choices<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string
  value: T
  options: { value: T; label: string; icon?: ReactNode }[]
  onChange: (value: T) => void
}) {
  const grid = options.length > 4
  return (
    <div className="flex flex-col gap-1">
      <span className="text-[10px] font-semibold uppercase tracking-wider text-koma-dim">{label}</span>
      <div className={grid ? 'grid grid-cols-4 gap-0.5 rounded border border-koma-border p-0.5' : 'flex rounded border border-koma-border p-0.5'}>
        {options.map((option) => {
          const on = value === option.value
          return (
            <button
              key={option.value}
              type="button"
              title={option.label}
              aria-label={option.label}
              aria-pressed={on}
              onClick={() => onChange(option.value)}
              className={`flex h-7 items-center justify-center rounded ${grid ? '' : 'min-w-0 flex-1 px-1'} ${
                on ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'
              }`}
            >
              {option.icon ?? <span className="truncate text-[11px]">{option.label}</span>}
            </button>
          )
        })}
      </div>
    </div>
  )
}

function ColorWell({
  label,
  value,
  fallback,
  picker,
  onChange,
}: {
  label: string
  value: string
  fallback: string
  picker: string
  onChange: (hex: string) => void
}) {
  return (
    <label className="relative h-7 w-7 flex-none cursor-pointer overflow-hidden rounded border border-koma-border" title={label}>
      <span className="absolute inset-0" style={{ background: value || fallback }} />
      <input
        type="color"
        aria-label={label}
        value={/^#[0-9a-fA-F]{6}$/.test(value) ? value : picker}
        className="absolute inset-0 cursor-pointer opacity-0"
        onChange={(e) => onChange(e.target.value)}
      />
    </label>
  )
}

function PaintRow({
  label,
  on,
  color,
  fallback,
  picker,
  onToggle,
  onColor,
}: {
  label: string
  on: boolean
  color: string
  fallback: string
  picker: string
  onToggle: (on: boolean) => void
  onColor: (hex: string) => void
}) {
  return (
    <div className="flex h-8 items-center gap-2">
      <span className="min-w-0 flex-1 text-[12px] text-koma-fg">{label}</span>
      <Toggle on={on} onChange={onToggle} />
      <ColorWell label={`${label} color`} value={color} fallback={fallback} picker={picker} onChange={onColor} />
    </div>
  )
}

function Properties({
  doc,
  root,
  diagPath,
  legacyAssetDir,
  selection,
  onEdge,
  onNode,
  onText,
  onEdgeText,
  onDetail,
  onTextDone,
}: {
  doc: DiagramDoc
  root: string
  diagPath: string
  legacyAssetDir: string
  selection: Selection | null
  onEdge: (id: string, patch: Partial<DiagramEdge>) => void
  onNode: (id: string, patch: Partial<DiagramNode>) => void
  onText: (id: string, text: string) => void
  onEdgeText: (id: string, text: string) => void
  onDetail: (kind: 'node' | 'edge', id: string, detail: string) => void
  onTextDone: () => void
}) {
  const [tab, setTab] = useState<'style' | 'detail'>('style')
  const node = selection?.type === 'node' ? doc.nodes.find((item) => item.id === selection.id) : undefined
  const edge = selection?.type === 'edge' ? doc.edges.find((item) => item.id === selection.id) : undefined
  const target = node ?? edge
  const selected = node ? 'node' : edge ? 'edge' : null
  const shapeKind = node ? 'node' : edge ? 'edge' : null
  const assetDir =
    shapeKind && target ? diagramShapeFolder(diagPath, shapeKind, target.id) ?? '' : ''
  const detailTitle =
    target && shapeKind
      ? node
        ? `Detail · ${shapeHeadingFor(node.text, node.id)} (\`${node.id}\`)`
        : `Detail · line (\`${edge!.id}\`)`
      : 'Detail'
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-8 flex-none items-center gap-1 border-b border-koma-border px-2">
        {selected ? (
          <>
            <PanelTab label={node ? 'Card' : 'Line'} on={tab === 'style'} onClick={() => setTab('style')} />
            <PanelTab label="Detail" on={tab === 'detail'} onClick={() => setTab('detail')} />
          </>
        ) : (
          <span className="px-1 text-[12px] text-koma-fg">Properties</span>
        )}
      </div>
      {selected && tab === 'detail' && target && shapeKind ? (
        <>
          <p className="flex-none border-b border-koma-border px-3 py-1.5 text-[10px] text-koma-dim">{detailTitle}</p>
          <MarkdownNote
            key={target.id}
            value={target.detail ?? ''}
            root={root}
            assetDir={assetDir}
            legacyAssetDir={legacyAssetDir}
            onChange={(detail) => onDetail(shapeKind, target.id, detail)}
            onDone={onTextDone}
          />
        </>
      ) : node ? (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <CardSettings node={node} onChange={(patch) => onNode(node.id, patch)} onText={(text) => onText(node.id, text)} onTextDone={onTextDone} />
        </div>
      ) : edge ? (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <LineSettings edge={edge} onChange={(patch) => onEdge(edge.id, patch)} onText={(text) => onEdgeText(edge.id, text)} onTextDone={onTextDone} />
        </div>
      ) : (
        <p className="px-3 py-2 text-[12px] text-koma-dim">Select a card or a line</p>
      )}
    </div>
  )
}

function PanelTab({ label, on, onClick }: { label: string; on: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      aria-pressed={on}
      onClick={onClick}
      className={`h-6 rounded px-2 text-[12px] ${on ? 'bg-koma-bg text-koma-fg' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`}
    >
      {label}
    </button>
  )
}

function CardSettings({
  node,
  onChange,
  onText,
  onTextDone,
}: {
  node: DiagramNode
  onChange: (patch: Partial<DiagramNode>) => void
  onText: (text: string) => void
  onTextDone: () => void
}) {
  const style = nodeStyle(node)
  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="flex flex-col gap-1">
        <span className="text-[10px] font-semibold uppercase tracking-wider text-koma-dim">Label</span>
        <div className="flex items-start gap-2">
          <InlineMarkdownInput
            key={node.id}
            text={node.text}
            aria-label="Label"
            className="min-h-7 min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-2 py-1 text-[12px] text-koma-fg outline-none focus:border-koma-fg/40 [&_strong]:font-semibold [&_em]:italic [&_code]:rounded [&_code]:bg-koma-panel2 [&_code]:px-0.5"
            style={{ color: style.textColor || undefined }}
            onFocus={onTextDone}
            onBlur={onTextDone}
            onChange={onText}
          />
          <ColorWell
            label="Label color"
            value={style.textColor}
            fallback="var(--color-koma-fg)"
            picker="#c8d3f5"
            onChange={(textColor) => onChange({ textColor })}
          />
        </div>
      </div>
      <PaintRow
        label="Fill"
        on={style.fill}
        color={style.fillColor}
        fallback="var(--color-koma-panel)"
        picker="#1a1d27"
        onToggle={(fill) => onChange({ fill })}
        onColor={(fillColor) => onChange({ fillColor, fill: true })}
      />
      <PaintRow
        label="Border"
        on={style.stroke}
        color={style.strokeColor}
        fallback="var(--color-koma-border)"
        picker="#8b93b8"
        onToggle={(stroke) => onChange({ stroke })}
        onColor={(strokeColor) => onChange({ strokeColor, stroke: true })}
      />
    </div>
  )
}

function LineSettings({
  edge,
  onChange,
  onText,
  onTextDone,
}: {
  edge: DiagramEdge
  onChange: (patch: Partial<DiagramEdge>) => void
  onText: (text: string) => void
  onTextDone: () => void
}) {
  const style = edgeStyle(edge)
  return (
    <div className="flex flex-col gap-3 p-3">
      <label className="flex flex-col gap-1">
        <span className="text-[10px] font-semibold uppercase tracking-wider text-koma-dim">Label</span>
        <input
          key={edge.id}
          value={edge.text ?? ''}
          placeholder="Label"
          aria-label="Label"
          onFocus={onTextDone}
          onChange={(e) => onText(e.target.value)}
          onBlur={onTextDone}
          className="h-7 w-full rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none focus:border-koma-fg/40"
        />
      </label>
      <PaintRow
        label="Stroke"
        on={style.stroke}
        color={style.color}
        fallback="var(--color-koma-fg)"
        picker="#c8d3f5"
        onToggle={(stroke) => onChange({ stroke: stroke ? undefined : false })}
        onColor={(color) => onChange({ color, stroke: undefined })}
      />
      <Choices
        label="Start"
        value={style.start}
        options={LINE_HEADS.map((head) => ({
          value: head.value,
          label: `${head.name} start`,
          icon: <ArrowGlyph side="start" kind={head.value} />,
        }))}
        onChange={(start) => onChange({ start })}
      />
      <Choices
        label="End"
        value={style.end}
        options={LINE_HEADS.map((head) => ({
          value: head.value,
          label: `${head.name} end`,
          icon: <ArrowGlyph side="end" kind={head.value} />,
        }))}
        onChange={(end) => onChange({ end })}
      />
      <Choices
        label="Pattern"
        value={style.dash}
        options={[
          {
            value: 'solid',
            label: 'Solid',
            icon: (
              <LineGlyph>
                <path d="M3 7 H25" fill="none" stroke="currentColor" strokeWidth="1.6" />
              </LineGlyph>
            ),
          },
          {
            value: 'dashed',
            label: 'Dashed',
            icon: (
              <LineGlyph>
                <path d="M3 7 H25" fill="none" stroke="currentColor" strokeWidth="1.6" strokeDasharray="5 3" />
              </LineGlyph>
            ),
          },
          {
            value: 'dotted',
            label: 'Dotted',
            icon: (
              <LineGlyph>
                <path d="M4 7 H24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeDasharray="0.1 4" />
              </LineGlyph>
            ),
          },
        ]}
        onChange={(dash) => onChange({ dash })}
      />
      <Choices
        label="Width"
        value={String(style.width)}
        options={[
          { value: '1', label: '1' },
          { value: '1.5', label: '1.5' },
          { value: '2', label: '2' },
          { value: '3', label: '3' },
        ]}
        onChange={(width) => onChange({ width: Number(width) })}
      />
      <Choices
        label="Corner"
        value={style.corner}
        options={[
          { value: 'sharp', label: 'Sharp' },
          { value: 'rounded', label: 'Round' },
        ]}
        onChange={(corner) => onChange({ corner })}
      />
      <Choices
        label="Route"
        value={style.route}
        options={[
          { value: 'orthogonal' satisfies DiagramRoute, label: 'Elbow' },
          { value: 'straight', label: 'Straight' },
        ]}
        onChange={(route) => onChange({ route, bends: [] })}
      />
    </div>
  )
}
