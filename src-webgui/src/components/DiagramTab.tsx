import { useEffect, useId, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { Check, Circle, Diamond, MousePointer2, RotateCw, Shapes, Spline, Square, Type } from 'lucide-react'
import { useKoma, type Tab } from '../store/koma'
import { recordCodingHistory } from '../lib/coding-recovery'
import { fileKey } from '../store/coding'
import { isTabVisible } from '../store/editorGroups'
import { BrailleSpinner } from './BrailleSpinner'
import { showCodingHistory } from './CodingHistory'
import { EditorChrome } from './EditorChrome'
import { Select, Toggle } from './panels/form'
import {
  SHAPE_MIME,
  defaultNodeSize,
  edgeRoute,
  edgeStyle,
  nearestSide,
  nextRotation,
  parseDiagram,
  pointerAngle,
  resizeNode,
  routePath,
  serializeDiagram,
  sideAnchor,
  slideSegment,
  snap,
  type DiagramDash,
  type DiagramDoc,
  type DiagramEdge,
  type DiagramHandle,
  type DiagramKind,
  type DiagramNode,
  type DiagramPoint,
  type DiagramRoute,
} from '../lib/diagram'

type Tool = 'select' | 'connect' | DiagramKind
type Selection = { type: 'node' | 'edge'; id: string }
type Drag =
  | { kind: 'move'; id: string; sx: number; sy: number; ox: number; oy: number; moved: boolean; remembered: boolean }
  | { kind: 'resize'; id: string; handle: DiagramHandle; sx: number; sy: number; node: DiagramNode; remembered: boolean }
  | { kind: 'rotate'; id: string; rotation: number; angle: number; node: DiagramNode; remembered: boolean }
  | { kind: 'segment'; edgeId: string; index: number; points: DiagramPoint[]; remembered: boolean }
  | { kind: 'anchor'; edgeId: string; end: 'from' | 'to'; remembered: boolean }
  | { kind: 'pan'; sx: number; sy: number; px: number; py: number }

const TOOLS: { id: Tool; label: string; icon: typeof Square }[] = [
  { id: 'select', label: 'Select', icon: MousePointer2 },
  { id: 'rect', label: 'Rectangle', icon: Square },
  { id: 'ellipse', label: 'Ellipse', icon: Circle },
  { id: 'diamond', label: 'Diamond', icon: Diamond },
  { id: 'text', label: 'Text', icon: Type },
  { id: 'connect', label: 'Connect', icon: Spline },
]

const HANDLES: { id: DiagramHandle; className: string; cursor: string }[] = [
  { id: 'nw', className: 'left-0 top-0', cursor: 'nwse-resize' },
  { id: 'n', className: 'left-1/2 top-0', cursor: 'ns-resize' },
  { id: 'ne', className: 'right-0 top-0', cursor: 'nesw-resize' },
  { id: 'e', className: 'right-0 top-1/2', cursor: 'ew-resize' },
  { id: 'se', className: 'right-0 bottom-0', cursor: 'nwse-resize' },
  { id: 's', className: 'left-1/2 bottom-0', cursor: 'ns-resize' },
  { id: 'sw', className: 'left-0 bottom-0', cursor: 'nesw-resize' },
  { id: 'w', className: 'left-0 top-1/2', cursor: 'ew-resize' },
]

function mintId(prefix: string): string {
  return `${prefix}${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`
}

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
  const markerId = useId().replace(/:/g, '')

  useEffect(() => {
    pastRef.current = []
    futureRef.current = []
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
      const node = doc.nodes.find((item) => item.id === (drag.end === 'from' ? edge?.from : edge?.to))
      if (!edge || !node) return
      const side = nearestSide(node, point)
      apply(drag, {
        ...doc,
        edges: doc.edges.map((item) =>
          item.id === edge.id
            ? tidyEdge({ ...item, bends: [], ...(drag.end === 'from' ? { fromSide: side } : { toSide: side }) })
            : item,
        ),
      }, doc)
    }
    const up = () => {
      dragRef.current = null
    }
    const onRestore = (event: Event) => {
      const detail = (event as CustomEvent<{ root?: string; path?: string }>).detail
      if (detail?.root !== tab.root || detail.path !== tab.path) return
      const current = useKoma.getState().diagram.docs[key]?.doc
      if (current) noteRef.current(current)
      setEditing(null)
      setSelection(null)
      setConnectFrom(null)
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
    window.addEventListener('koma-diagram-restore', onRestore)
    return () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      window.removeEventListener('koma-diagram-restore', onRestore)
    }
  }, [key, tab.path, tab.root])

  useEffect(() => {
    if (!active) return
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null
      const typing = !!target?.closest('input, textarea, [contenteditable="true"]')
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
      if (e.key === 'Escape') {
        setConnectFrom(null)
        setSelection(null)
        setEditing(null)
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
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [active, file, saveDiagram, selection, tab.path, tab.root, updateDiagram])

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
    if (id) setSelection({ type: 'node', id })
    setTool('select')
    setConnectFrom(null)
  }

  const onCanvasPointerDown = (e: ReactPointerEvent) => {
    if (e.button !== 0) return
    if ((e.target as HTMLElement).closest('[data-diagram-node], [data-diagram-edge], [data-diagram-ui]')) return
    setSelection(null)
    setEditing(null)
    const p = toDoc(e)
    if (!p || !file || file.loading) return
    if (tool !== 'select' && tool !== 'connect') {
      addAt(tool, p.x, p.y)
      return
    }
    if (tool === 'connect') {
      setConnectFrom(null)
      return
    }
    dragRef.current = { kind: 'pan', sx: e.clientX, sy: e.clientY, px: panRef.current.x, py: panRef.current.y }
  }

  const onNodePointerDown = (e: ReactPointerEvent, node: DiagramNode) => {
    if (e.button !== 0 || !file) return
    e.stopPropagation()
    if (editing === node.id) return
    if (tool === 'connect') {
      if (!connectFrom) {
        setConnectFrom(node.id)
        setSelection({ type: 'node', id: node.id })
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
      return
    }
    setSelection({ type: 'node', id: node.id })
    setConnectFrom(null)
    dragRef.current = { kind: 'move', id: node.id, sx: e.clientX, sy: e.clientY, ox: node.x, oy: node.y, moved: false, remembered: false }
  }

  const onEdgePointerDown = (e: ReactPointerEvent, edge: DiagramEdge) => {
    if (e.button !== 0) return
    e.stopPropagation()
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
      />
      {file.error ? (
        <div className="flex-none border-b border-koma-border px-3 py-1 text-[12px] text-koma-error">{file.error}</div>
      ) : null}
      <div
        ref={canvasRef}
        className={`relative min-h-0 flex-1 overflow-hidden ${tool === 'select' ? 'cursor-grab' : 'cursor-crosshair'}`}
        style={
          doc.snap
            ? {
                backgroundImage:
                  'radial-gradient(circle, color-mix(in srgb, var(--koma-fg, #c8d3f5) 28%, transparent) 1px, transparent 1px)',
                backgroundSize: `${grid}px ${grid}px`,
                backgroundPosition: `${pan.x}px ${pan.y}px`,
              }
            : undefined
        }
        onPointerDown={onCanvasPointerDown}
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
              const marker = (name: string, reverse: boolean) => (
                <marker
                  key={name}
                  id={`${id}-${name}`}
                  markerWidth="8"
                  markerHeight="8"
                  refX={reverse ? 1 : 7}
                  refY="3"
                  orient={reverse ? 'auto-start-reverse' : 'auto'}
                >
                  <path d="M0,0 L7,3 L0,6" fill="none" stroke={color} strokeWidth="1.2" />
                </marker>
              )
              return (
                <g key={edge.id}>
                  {style.end === 'arrow' ? (
                    <marker id={`${id}-end`} markerWidth="8" markerHeight="8" refX="7" refY="3" orient="auto">
                      <path d="M0,0 L7,3 L0,6 Z" fill={color} />
                    </marker>
                  ) : style.end === 'open' ? (
                    marker('end', false)
                  ) : null}
                  {style.start === 'arrow' ? (
                    <marker id={`${id}-start`} markerWidth="8" markerHeight="8" refX="1" refY="3" orient="auto-start-reverse">
                      <path d="M0,0 L7,3 L0,6 Z" fill={color} />
                    </marker>
                  ) : style.start === 'open' ? (
                    marker('start', true)
                  ) : null}
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
        </svg>
        {doc.nodes.map((node) => {
          const selected = selection?.type === 'node' && selection.id === node.id
          const shape =
            node.kind === 'ellipse'
              ? 'rounded-full border border-koma-border bg-koma-panel'
              : node.kind === 'text'
                ? ''
                : node.kind === 'diamond'
                  ? ''
                  : 'rounded border border-koma-border bg-koma-panel'
          return (
            <div
              key={node.id}
              data-diagram-node=""
              className={`absolute flex items-center justify-center ${shape} ${connectFrom === node.id ? 'ring-1 ring-koma-accent' : ''}`}
              style={{
                left: pan.x + node.x,
                top: pan.y + node.y,
                width: node.w,
                height: node.h,
                transform: node.rotation ? `rotate(${node.rotation}deg)` : undefined,
              }}
              onPointerDown={(e) => onNodePointerDown(e, node)}
              onDoubleClick={(e) => {
                e.stopPropagation()
                dragRef.current = null
                labelNoted.current = false
                setEditing(node.id)
                setSelection({ type: 'node', id: node.id })
              }}
            >
              {node.kind === 'diamond' ? (
                <svg className="absolute inset-0 h-full w-full overflow-visible">
                  <polygon
                    points={`${node.w / 2},1 ${node.w - 1},${node.h / 2} ${node.w / 2},${node.h - 1} 1,${node.h / 2}`}
                    fill="var(--color-koma-panel)"
                    stroke={connectFrom === node.id ? 'var(--color-koma-accent)' : 'var(--color-koma-border)'}
                  />
                </svg>
              ) : null}
              {editing === node.id ? (
                <input
                  autoFocus
                  defaultValue={node.text}
                  className="z-10 h-7 w-[90%] rounded border border-koma-border bg-koma-bg px-2 text-center text-[12px] text-koma-fg outline-none"
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
                <span className="pointer-events-none z-10 truncate px-2 text-center text-[12px]">{node.text || ' '}</span>
              )}
              {selected && tool === 'select' && editing !== node.id ? (
                <>
                  {HANDLES.map((handle) => (
                    <span
                      key={handle.id}
                      className={`absolute z-10 h-2 w-2 -translate-x-1/2 -translate-y-1/2 rounded-full border border-koma-bg bg-koma-accent ${handle.className}`}
                      style={{ cursor: handle.cursor }}
                      onPointerDown={(e) => {
                        e.stopPropagation()
                        e.preventDefault()
                        dragRef.current = { kind: 'resize', id: node.id, handle: handle.id, sx: e.clientX, sy: e.clientY, node, remembered: false }
                      }}
                      onDoubleClick={(e) => e.stopPropagation()}
                    />
                  ))}
                  <button
                    type="button"
                    title="Rotate"
                    aria-label="Rotate"
                    className="absolute -right-3 -top-3 z-10 flex h-4 w-4 items-center justify-center rounded-full border border-koma-border bg-koma-panel text-koma-accent"
                    onPointerDown={(e) => {
                      e.stopPropagation()
                      e.preventDefault()
                      const point = toDoc(e)
                      if (!point) return
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
        {selection?.type === 'edge' && tool === 'select' ? (
          <LineOptions
            edge={doc.edges.find((edge) => edge.id === selection.id)}
            nodes={byId}
            pan={pan}
            canvas={canvasRef.current}
            onChange={(patch) => patchEdge(selection.id, patch)}
          />
        ) : null}
        {!file.loading && doc.nodes.length === 0 ? (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center text-[12px] text-koma-fg opacity-35">
            Drag a shape onto the canvas
          </div>
        ) : null}
      </div>
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
            className={`flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg transition ${
              tool === id ? 'bg-koma-hover opacity-100' : 'opacity-70 hover:bg-koma-hover hover:opacity-100'
            }`}
          >
            <Icon size={14} strokeWidth={1.6} />
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

function LineOptions({
  edge,
  nodes,
  pan,
  canvas,
  onChange,
}: {
  edge: DiagramEdge | undefined
  nodes: Map<string, DiagramNode>
  pan: { x: number; y: number }
  canvas: HTMLDivElement | null
  onChange: (patch: Partial<DiagramEdge>) => void
}) {
  if (!edge) return null
  const from = nodes.get(edge.from)
  const to = nodes.get(edge.to)
  if (!from || !to || !canvas) return null
  const style = edgeStyle(edge)
  const points = edgeRoute(from, to, edge)
  const mid = points[Math.floor(points.length / 2)] ?? points[0]
  if (!mid) return null
  const width = canvas.clientWidth
  const height = canvas.clientHeight
  const left = Math.min(Math.max(8, mid.x + pan.x - 116), Math.max(8, width - 240))
  const top = Math.min(Math.max(8, mid.y + pan.y - 78), Math.max(8, height - 96))
  return (
    <div
      data-diagram-ui=""
      className="absolute z-30 flex w-[232px] flex-col gap-1 rounded border border-koma-border bg-koma-panel p-1.5"
      style={{ left, top }}
      onPointerDown={(e) => e.stopPropagation()}
    >
      <div className="flex items-center gap-1">
        <button
          type="button"
          aria-pressed={style.stroke}
          title="Line"
          onClick={() => onChange({ stroke: style.stroke ? false : undefined })}
          className={`flex h-5 flex-none items-center gap-1 rounded px-1.5 text-[11px] ${
            style.stroke ? 'bg-koma-hover text-koma-fg' : 'text-koma-fg opacity-45 hover:bg-koma-hover hover:opacity-80'
          }`}
        >
          {style.stroke ? <Check size={11} className="text-koma-accent" /> : <span className="h-[11px] w-[11px] rounded-sm border border-koma-border" />}
          Line
        </button>
        <div className="min-w-0 flex-1">
          <Select
            value={style.corner}
            options={[
              { value: 'sharp', label: 'Sharp' },
              { value: 'rounded', label: 'Rounded' },
            ]}
            onChange={(corner) => onChange({ corner })}
          />
        </div>
        <input
          type="color"
          aria-label="Line color"
          title="Line color"
          value={style.color || '#c8d3f5'}
          className="h-5 w-5 flex-none cursor-pointer rounded border border-koma-border bg-transparent p-0"
          onChange={(e) => onChange({ color: e.target.value })}
        />
      </div>
      <div className="flex items-center gap-1">
        <div className="min-w-0 flex-1">
          <Select
            value={style.start}
            triggerTitle="Start"
            options={[
              { value: 'none', label: 'No start' },
              { value: 'arrow', label: 'Arrow start' },
              { value: 'open', label: 'Open start' },
            ]}
            onChange={(start) => onChange({ start })}
          />
        </div>
        <div className="min-w-0 flex-1">
          <Select
            value={style.dash}
            triggerTitle="Pattern"
            options={[
              { value: 'solid', label: 'Solid' },
              { value: 'dashed', label: 'Dashed' },
              { value: 'dotted', label: 'Dotted' },
            ]}
            onChange={(dash) => onChange({ dash })}
          />
        </div>
        <div className="w-16 flex-none">
          <Select
            value={String(style.width) as '1' | '1.5' | '2' | '3'}
            triggerTitle="Width"
            options={[
              { value: '1', label: '1 pt' },
              { value: '1.5', label: '1.5 pt' },
              { value: '2', label: '2 pt' },
              { value: '3', label: '3 pt' },
            ]}
            onChange={(width) => onChange({ width: Number(width) })}
          />
        </div>
      </div>
      <div className="flex items-center gap-1">
        <div className="min-w-0 flex-1">
          <Select
            value={style.end}
            triggerTitle="End"
            options={[
              { value: 'none', label: 'No end' },
              { value: 'arrow', label: 'Arrow end' },
              { value: 'open', label: 'Open end' },
            ]}
            onChange={(end) => onChange({ end })}
          />
        </div>
        <div className="min-w-0 flex-1">
          <Select
            value={style.route}
            triggerTitle="Route"
            options={[
              { value: 'orthogonal' satisfies DiagramRoute, label: 'Elbow' },
              { value: 'straight', label: 'Straight' },
            ]}
            onChange={(route) => onChange({ route, bends: [] })}
          />
        </div>
      </div>
    </div>
  )
}
