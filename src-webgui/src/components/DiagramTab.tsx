import { useEffect, useId, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { Circle, Diamond, MousePointer2, Spline, Square, Type } from 'lucide-react'
import { useKoma, type Tab } from '../store/koma'
import { fileKey } from '../store/coding'
import { isTabVisible } from '../store/editorGroups'
import { BrailleSpinner } from './BrailleSpinner'
import { Toggle } from './panels/form'
import {
  SHAPE_MIME,
  defaultNodeSize,
  snap,
  type DiagramDoc,
  type DiagramEdge,
  type DiagramKind,
  type DiagramNode,
} from '../lib/diagram'

type Tool = 'select' | 'connect' | DiagramKind
type Selection = { type: 'node' | 'edge'; id: string }
type Drag =
  | { kind: 'move'; id: string; sx: number; sy: number; ox: number; oy: number; moved: boolean }
  | { kind: 'resize'; id: string; sx: number; sy: number; ow: number; oh: number }
  | { kind: 'pan'; sx: number; sy: number; px: number; py: number }

const TOOLS: { id: Tool; label: string; icon: typeof Square }[] = [
  { id: 'select', label: 'Select', icon: MousePointer2 },
  { id: 'rect', label: 'Rectangle', icon: Square },
  { id: 'ellipse', label: 'Ellipse', icon: Circle },
  { id: 'diamond', label: 'Diamond', icon: Diamond },
  { id: 'text', label: 'Text', icon: Type },
  { id: 'connect', label: 'Connect', icon: Spline },
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

export function DiagramTab({ tab }: { tab: Extract<Tab, { kind: 'diagram' }> }) {
  const key = fileKey(tab.root, tab.path)
  const file = useKoma((s) => s.diagram.docs[key])
  const updateDiagram = useKoma((s) => s.updateDiagram)
  const saveDiagram = useKoma((s) => s.saveDiagram)
  const active = useKoma((s) => s.ui.activeTabId === tab.id && isTabVisible(s.ui, tab.id))
  const canvasRef = useRef<HTMLDivElement>(null)
  const dragRef = useRef<Drag | null>(null)
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
  const markerId = useId().replace(/:/g, '')

  useEffect(() => {
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
      const dx = e.clientX - drag.sx
      const dy = e.clientY - drag.sy
      if (drag.kind === 'move' && !drag.moved) {
        if (Math.abs(dx) < 3 && Math.abs(dy) < 3) return
        drag.moved = true
      }
      const nodes = doc.nodes.map((n) => {
        if (n.id !== drag.id) return n
        if (drag.kind === 'move') {
          return {
            ...n,
            x: snap(drag.ox + dx, doc.grid, doc.snap),
            y: snap(drag.oy + dy, doc.grid, doc.snap),
          }
        }
        return {
          ...n,
          w: Math.max(32, snap(drag.ow + dx, doc.grid, doc.snap)),
          h: Math.max(24, snap(drag.oh + dy, doc.grid, doc.snap)),
        }
      })
      updateRef.current(tab.root, tab.path, { ...doc, nodes })
    }
    const up = () => {
      dragRef.current = null
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
    return () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
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
          updateDiagram(tab.root, tab.path, {
            ...doc,
            nodes: doc.nodes.filter((n) => n.id !== selection.id),
            edges: doc.edges.filter((edge) => edge.from !== selection.id && edge.to !== selection.id),
          })
        } else {
          updateDiagram(tab.root, tab.path, {
            ...doc,
            edges: doc.edges.filter((edge) => edge.id !== selection.id),
          })
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
    updateDiagram(tab.root, tab.path, next)
    const id = next.nodes[next.nodes.length - 1]?.id
    if (id) setSelection({ type: 'node', id })
    setTool('select')
    setConnectFrom(null)
  }

  const onCanvasPointerDown = (e: ReactPointerEvent) => {
    if (e.button !== 0) return
    if ((e.target as HTMLElement).closest('[data-diagram-node], [data-diagram-edge]')) return
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
          updateDiagram(tab.root, tab.path, {
            ...file.doc,
            edges: [...file.doc.edges, { id: mintId('e'), from: connectFrom, to: node.id }],
          })
        }
      }
      setConnectFrom(null)
      setSelection({ type: 'node', id: node.id })
      return
    }
    setSelection({ type: 'node', id: node.id })
    setConnectFrom(null)
    dragRef.current = { kind: 'move', id: node.id, sx: e.clientX, sy: e.clientY, ox: node.x, oy: node.y, moved: false }
  }

  const onEdgePointerDown = (e: ReactPointerEvent, edge: DiagramEdge) => {
    if (e.button !== 0) return
    e.stopPropagation()
    setTool('select')
    setConnectFrom(null)
    setSelection({ type: 'edge', id: edge.id })
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
  const byId = new Map(doc.nodes.map((n) => [n.id, n]))
  const connectNode = connectFrom ? byId.get(connectFrom) : undefined

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col bg-koma-bg text-koma-fg">
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
          if ([...e.dataTransfer.types].some((t) => t === SHAPE_MIME || t === 'text/plain')) {
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
            <marker id={markerId} markerWidth="8" markerHeight="8" refX="7" refY="3" orient="auto">
              <path d="M0,0 L7,3 L0,6 Z" className="fill-koma-fg opacity-50" />
            </marker>
          </defs>
          {doc.edges.map((edge) => {
            const from = byId.get(edge.from)
            const to = byId.get(edge.to)
            if (!from || !to) return null
            const selected = selection?.type === 'edge' && selection.id === edge.id
            const x1 = pan.x + from.x + from.w / 2
            const y1 = pan.y + from.y + from.h / 2
            const x2 = pan.x + to.x + to.w / 2
            const y2 = pan.y + to.y + to.h / 2
            return (
              <g key={edge.id} data-diagram-edge="" className="pointer-events-auto" onPointerDown={(e) => onEdgePointerDown(e, edge)}>
                <line x1={x1} y1={y1} x2={x2} y2={y2} stroke="transparent" strokeWidth={12} />
                <line
                  x1={x1}
                  y1={y1}
                  x2={x2}
                  y2={y2}
                  stroke={selected ? 'var(--color-koma-accent)' : 'color-mix(in srgb, var(--koma-fg, #c8d3f5) 45%, transparent)'}
                  strokeWidth={selected ? 2 : 1.5}
                  markerEnd={`url(#${markerId})`}
                />
              </g>
            )
          })}
          {connectNode && cursor ? (
            <line
              x1={pan.x + connectNode.x + connectNode.w / 2}
              y1={pan.y + connectNode.y + connectNode.h / 2}
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
              className={`absolute flex items-center justify-center ${shape} ${selected && node.kind !== 'diamond' ? 'ring-1 ring-koma-accent' : ''} ${connectFrom === node.id ? 'ring-1 ring-koma-accent' : ''}`}
              style={{ left: pan.x + node.x, top: pan.y + node.y, width: node.w, height: node.h }}
              onPointerDown={(e) => onNodePointerDown(e, node)}
              onDoubleClick={(e) => {
                e.stopPropagation()
                dragRef.current = null
                setEditing(node.id)
              }}
            >
              {node.kind === 'diamond' ? (
                <svg className="absolute inset-0 h-full w-full overflow-visible">
                  <polygon
                    points={`${node.w / 2},1 ${node.w - 1},${node.h / 2} ${node.w / 2},${node.h - 1} 1,${node.h / 2}`}
                    fill="var(--color-koma-panel)"
                    stroke={selected || connectFrom === node.id ? 'var(--color-koma-accent)' : 'var(--color-koma-border)'}
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
                    updateDiagram(tab.root, tab.path, {
                      ...current,
                      nodes: current.nodes.map((n) => (n.id === node.id ? { ...n, text } : n)),
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
              {selected && tool === 'select' ? (
                <span
                  className="absolute -bottom-1 -right-1 z-10 h-2.5 w-2.5 cursor-nwse-resize rounded-sm bg-koma-accent"
                  onPointerDown={(e) => {
                    e.stopPropagation()
                    dragRef.current = { kind: 'resize', id: node.id, sx: e.clientX, sy: e.clientY, ow: node.w, oh: node.h }
                  }}
                />
              ) : null}
            </div>
          )
        })}
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
              updateDiagram(tab.root, tab.path, { ...doc, snap: on })
            }}
          />
        </div>
      </div>
    </div>
  )
}
