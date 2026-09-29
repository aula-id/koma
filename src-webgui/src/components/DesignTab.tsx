import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react'
import { Frame, Hand, Minus, MousePointer2, Plus, Square, Type, PanelRightClose, PanelRightOpen } from 'lucide-react'
import {
  DESIGN_MIME,
  copyTree,
  createNode,
  deleteDesignNode,
  frameAtPoint,
  hitDesign,
  insertDesignNode,
  locateDesign,
  nodeChrome,
  nodeOrigin,
  parseDesign,
  placeDesignNode,
  resizeDesignNode,
  resolveRef,
  serializeDesign,
  snapDesign,
  textStyle,
  updateDesignNode,
  type DesignDoc,
  type DesignHandle,
  type DesignNode,
  type DesignWeight,
} from '../lib/design'
import { fileKey } from '../store/coding'
import { isTabVisible } from '../store/editorGroups'
import { useKoma } from '../store/koma'
import type { Tab } from '../store/types/tabs'
import { showCodingHistory } from './CodingHistory'
import { EditorChrome } from './EditorChrome'

const UNDO_CAP = 50
const ZOOM_MIN = 0.25
const ZOOM_MAX = 4
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

type Tool = 'select' | 'frame' | 'rect' | 'text' | 'pan'
type View = { panX: number; panY: number; zoom: number }
type Drag =
  | { kind: 'move'; id: string; startX: number; startY: number; originX: number; originY: number; remembered: boolean }
  | { kind: 'resize'; id: string; handle: DesignHandle; startX: number; startY: number; node: DesignNode; remembered: boolean }
  | { kind: 'pan'; lastX: number; lastY: number }

let copiedShape: { node: DesignNode; parentId: string | null } | null = null
let mintSeq = 0

function mintId(prefix: string): string {
  mintSeq += 1
  return `${prefix}${Date.now().toString(36)}${mintSeq}`
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
  const dragRef = useRef<Drag | null>(null)
  const labelNoted = useRef(false)
  const spaceRef = useRef(false)
  const toolRef = useRef<Tool>('select')
  const viewRef = useRef<View>({ panX: 40, panY: 40, zoom: 1 })
  const updateRef = useRef(updateDesign)
  updateRef.current = updateDesign
  const [view, setView] = useState<View>(viewRef.current)
  const [tool, setTool] = useState<Tool>('select')
  const [selection, setSelection] = useState<string | null>(null)
  const [editing, setEditing] = useState<string | null>(null)
  const [propsOpen, setPropsOpen] = useState(false)
  const [spaceDown, setSpaceDown] = useState(false)
  const [dragCursor, setDragCursor] = useState<string | null>(null)
  const [rev, setRev] = useState(0)
  toolRef.current = tool

  useEffect(() => {
    pastRef.current = []
    futureRef.current = []
    setSelection(null)
    setEditing(null)
    const next = { panX: 40, panY: 40, zoom: 1 }
    viewRef.current = next
    setView(next)
    setRev((value) => value + 1)
  }, [key])

  const note = (doc: DesignDoc) => {
    pastRef.current.push(doc)
    if (pastRef.current.length > UNDO_CAP) pastRef.current.shift()
    futureRef.current = []
    setRev((value) => value + 1)
  }
  const noteRef = useRef(note)
  noteRef.current = note

  const commit = (next: DesignDoc) => {
    const current = useKoma.getState().design.docs[key]?.doc
    if (!current || serializeDesign(current) === serializeDesign(next)) return
    note(current)
    updateDesign(tab.root, tab.path, next)
  }

  const undo = () => {
    const prev = pastRef.current.pop()
    if (!prev) return
    const current = useKoma.getState().design.docs[key]?.doc
    if (current) futureRef.current.push(current)
    setRev((value) => value + 1)
    setEditing(null)
    updateDesign(tab.root, tab.path, prev)
  }
  const redo = () => {
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
    const move = (event: PointerEvent) => {
      const drag = dragRef.current
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
      const doc = useKoma.getState().design.docs[key]?.doc
      if (!doc) return
      const zoom = viewRef.current.zoom || 1
      const dx = (event.clientX - drag.startX) / zoom
      const dy = (event.clientY - drag.startY) / zoom
      const located = locateDesign(doc, drag.id)
      if (!located) return
      const nextNode = drag.kind === 'move'
        ? { ...located.node, x: snapDesign(drag.originX + dx, doc.grid, doc.snap), y: snapDesign(drag.originY + dy, doc.grid, doc.snap) }
        : resizeDesignNode(drag.node, drag.handle, dx, dy, doc.grid, doc.snap)
      const next = updateDesignNode(doc, drag.id, () => nextNode)
      if (serializeDesign(next) === serializeDesign(doc)) return
      if (!drag.remembered) {
        noteRef.current(doc)
        drag.remembered = true
      }
      updateRef.current(tab.root, tab.path, next)
    }
    const up = (event: PointerEvent) => {
      const drag = dragRef.current
      dragRef.current = null
      setDragCursor(null)
      const doc = useKoma.getState().design.docs[key]?.doc
      if (!doc || !drag || drag.kind === 'pan') return
      if (drag.kind !== 'move') return
      const point = toDoc(event.clientX, event.clientY)
      const located = locateDesign(doc, drag.id)
      const origin = nodeOrigin(doc, drag.id)
      if (!point || !located || !origin) return
      const target = frameAtPoint(doc, point.x, point.y, drag.id)
      if (target === located.parentId || (target == null && (located.parentId == null || located.node.kind !== 'frame'))) return
      const parentOrigin = target ? nodeOrigin(doc, target) : { x: 0, y: 0 }
      if (!parentOrigin) return
      const x = snapDesign(origin.x - parentOrigin.x, doc.grid, doc.snap)
      const y = snapDesign(origin.y - parentOrigin.y, doc.grid, doc.snap)
      const next = placeDesignNode(doc, drag.id, target, x, y)
      if (serializeDesign(next) === serializeDesign(doc)) return
      if (!drag.remembered) noteRef.current(doc)
      updateRef.current(tab.root, tab.path, next)
    }
    window.addEventListener('koma-design-restore', onRestore)
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
    window.addEventListener('pointercancel', up)
    return () => {
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
      if (meta && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'c') {
        const doc = useKoma.getState().design.docs[key]?.doc
        if (!doc || !selection) return
        const located = locateDesign(doc, selection)
        if (!located || located.node.kind === 'instance') return
        copiedShape = { node: located.node, parentId: located.parentId }
        event.preventDefault()
        return
      }
      if (meta && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'v') {
        if (!copiedShape) return
        event.preventDefault()
        const doc = useKoma.getState().design.docs[key]?.doc
        if (!doc || file?.loading) return
        const step = doc.snap && doc.grid > 0 ? doc.grid : 16
        const pasted = copyTree(copiedShape.node, () => mintId('n'), step, step)
        const parentId = copiedShape.parentId && locateDesign(doc, copiedShape.parentId) ? copiedShape.parentId : null
        let usedParent = parentId
        let next = doc
        if (parentId) next = insertDesignNode(doc, parentId, pasted)
        else if (pasted.kind === 'frame') next = insertDesignNode(doc, null, pasted)
        else if (doc.screens[0]) {
          usedParent = doc.screens[0].id
          next = insertDesignNode(doc, usedParent, pasted)
        } else {
          const screen = createNode('frame', mintId('f'), pasted.x, pasted.y)
          pasted.x = 16
          pasted.y = 16
          screen.children = [pasted]
          usedParent = screen.id
          next = insertDesignNode(doc, null, screen)
        }
        copiedShape = { node: pasted, parentId: usedParent }
        commit(next)
        setSelection(pasted.id)
        setEditing(null)
        setPropsOpen(true)
        return
      }
      if (event.key === 'Delete' || event.key === 'Backspace') {
        if (!selection) return
        event.preventDefault()
        const doc = useKoma.getState().design.docs[key]?.doc
        if (!doc) return
        commit(deleteDesignNode(doc, selection))
        setSelection(null)
        setEditing(null)
      }
      if (event.key === 'Escape') setEditing(null)
    }
    window.addEventListener('keydown', onKey)
    window.addEventListener('keyup', onKey)
    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('keyup', onKey)
    }
  }, [active, file?.loading, key, saveDesign, selection, tab.path, tab.root])

  const placeAt = (kind: 'frame' | 'rect' | 'text', point: { x: number; y: number }) => {
    const doc = useKoma.getState().design.docs[key]?.doc
    if (!doc) return
    const parent = frameAtPoint(doc, point.x, point.y, '')
    const node = createNode(kind, mintId(kind[0]), 0, 0)
    let next = doc
    if (parent) {
      const origin = nodeOrigin(doc, parent)
      if (!origin) return
      node.x = snapDesign(point.x - origin.x, doc.grid, doc.snap)
      node.y = snapDesign(point.y - origin.y, doc.grid, doc.snap)
      next = insertDesignNode(doc, parent, node)
    } else if (kind === 'frame') {
      node.x = snapDesign(point.x, doc.grid, doc.snap)
      node.y = snapDesign(point.y, doc.grid, doc.snap)
      next = insertDesignNode(doc, null, node)
    } else {
      const screen = createNode('frame', mintId('f'), snapDesign(point.x, doc.grid, doc.snap), snapDesign(point.y, doc.grid, doc.snap))
      node.x = 16
      node.y = 16
      screen.children = [node]
      next = insertDesignNode(doc, null, screen)
    }
    commit(next)
    setSelection(node.id)
    setEditing(null)
    setPropsOpen(true)
    setTool('select')
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
    if (tool === 'frame' || tool === 'rect' || tool === 'text') {
      placeAt(tool, point)
      return
    }
    setSelection(null)
  }

  const onDrop = (event: React.DragEvent<HTMLDivElement>) => {
    event.preventDefault()
    const kind = event.dataTransfer.getData(DESIGN_MIME)
    if (kind !== 'frame' && kind !== 'rect' && kind !== 'text') return
    const point = toDoc(event.clientX, event.clientY)
    if (!point) return
    placeAt(kind, point)
  }

  if (!file) {
    return <div className="flex h-full items-center justify-center text-[12px] text-koma-dim">Loading…</div>
  }

  const doc = file.doc
  const located = selection ? locateDesign(doc, selection) : null
  const selected = located?.node ?? null
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
    if (!selection) return
    const current = useKoma.getState().design.docs[key]?.doc
    if (!current) return
    const next = updateDesignNode(current, selection, fn)
    if (serializeDesign(next) === serializeDesign(current)) return
    if (noteOnce) {
      if (!labelNoted.current) {
        note(current)
        labelNoted.current = true
      }
      updateDesign(tab.root, tab.path, next)
      return
    }
    commit(next)
  }

  return (
    <div className="flex h-full min-h-0 min-w-0 bg-koma-bg text-koma-fg">
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
        <div
          ref={canvasRef}
          className={`relative min-h-0 flex-1 overflow-hidden ${spaceDown || tool === 'pan' ? 'cursor-grab' : ''}`}
          onPointerDown={onCanvasPointerDown}
          onDragOver={(event) => event.preventDefault()}
          onDrop={onDrop}
          style={{
            backgroundImage: doc.snap ? 'radial-gradient(circle, var(--color-koma-border) 1px, transparent 1px)' : undefined,
            backgroundSize: doc.snap ? `${grid * view.zoom}px ${grid * view.zoom}px` : undefined,
            backgroundPosition: `${view.panX}px ${view.panY}px`,
          }}
        >
          <div className="pointer-events-none absolute left-0 top-0" style={{ transform: `translate(${view.panX}px, ${view.panY}px) scale(${view.zoom})`, transformOrigin: '0 0' }}>
            {doc.screens.map((screen) => (
              <DesignNodeView
                key={screen.id}
                doc={doc}
                node={screen}
                selectedId={selection}
                editing={editing}
                dragCursor={dragCursor}
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
                  setSelection(id)
                  setPropsOpen(true)
                  const located = locateDesign(useKoma.getState().design.docs[key]?.doc ?? doc, id)
                  dragRef.current = {
                    kind: 'move',
                    id,
                    startX: event.clientX,
                    startY: event.clientY,
                    originX: located?.node.x ?? 0,
                    originY: located?.node.y ?? 0,
                    remembered: false,
                  }
                  setDragCursor('grabbing')
                }}
                onResize={(id, handle, event) => {
                  event.preventDefault()
                  event.stopPropagation()
                  setSelection(id)
                  const located = locateDesign(useKoma.getState().design.docs[key]?.doc ?? doc, id)
                  if (!located) return
                  dragRef.current = {
                    kind: 'resize',
                    id,
                    handle,
                    startX: event.clientX,
                    startY: event.clientY,
                    node: { ...located.node },
                    remembered: false,
                  }
                  setDragCursor(HANDLES.find((item) => item.id === handle)?.cursor ?? 'grabbing')
                }}
                onEdit={(id) => {
                  setSelection(id)
                  setEditing(id)
                  setPropsOpen(true)
                  labelNoted.current = false
                }}
                onText={(id, text) => {
                  const current = useKoma.getState().design.docs[key]?.doc
                  if (!current) return
                  const next = updateDesignNode(current, id, (node) => ({ ...node, text }))
                  if (serializeDesign(next) === serializeDesign(current)) return
                  if (!labelNoted.current) {
                    note(current)
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
          </div>
          {doc.screens.length === 0 && !file.loading ? (
            <div className="pointer-events-none absolute inset-0 flex items-center justify-center text-[12px] text-koma-fg opacity-35">
              Drag a frame onto the canvas
            </div>
          ) : null}
        </div>
        <div className="flex h-8 flex-none items-center gap-1 border-t border-koma-border bg-koma-panel px-2">
          <ToolButton label="Select" selected={tool === 'select'} onClick={() => setTool('select')}>
            <MousePointer2 size={15} strokeWidth={2.25} />
          </ToolButton>
          <ToolButton label="Frame" selected={tool === 'frame'} onClick={() => setTool('frame')}>
            <Frame size={15} strokeWidth={2.25} />
          </ToolButton>
          <ToolButton label="Rectangle" selected={tool === 'rect'} onClick={() => setTool('rect')}>
            <Square size={15} strokeWidth={2.25} />
          </ToolButton>
          <ToolButton label="Text" selected={tool === 'text'} onClick={() => setTool('text')}>
            <Type size={15} strokeWidth={2.25} />
          </ToolButton>
          <ToolButton label="Move canvas" selected={tool === 'pan'} onClick={() => setTool('pan')}>
            <Hand size={15} strokeWidth={2.25} />
          </ToolButton>
          <div className="ml-auto flex items-center gap-1">
            <ToolButton label="Zoom out" selected={false} onClick={() => {
              const rect = canvasRef.current?.getBoundingClientRect()
              if (!rect) return
              zoomAt(rect.left + rect.width / 2, rect.top + rect.height / 2, view.zoom / 1.1)
            }}>
              <Minus size={15} strokeWidth={2.25} />
            </ToolButton>
            <span className="w-10 text-center text-[11px] text-koma-dim">{Math.round(view.zoom * 100)}%</span>
            <ToolButton label="Zoom in" selected={false} onClick={() => {
              const rect = canvasRef.current?.getBoundingClientRect()
              if (!rect) return
              zoomAt(rect.left + rect.width / 2, rect.top + rect.height / 2, view.zoom * 1.1)
            }}>
              <Plus size={15} strokeWidth={2.25} />
            </ToolButton>
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
          <div className="flex h-8 flex-none items-center px-3 text-[12px] text-koma-fg">
            {selected ? (selected.kind === 'frame' ? 'Frame' : selected.kind === 'rect' ? 'Rectangle' : selected.kind === 'text' ? 'Text' : 'Instance') : 'Properties'}
          </div>
          {selected ? (
            <NodeSettings
              node={selected}
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
            <p className="px-3 text-[12px] text-koma-dim">Select a frame or a text</p>
          )}
        </aside>
      ) : null}
    </div>
  )
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
  selectedId,
  editing,
  dragCursor,
  onSelect,
  onResize,
  onEdit,
  onText,
  onTextBlur,
}: {
  doc: DesignDoc
  node: DesignNode
  selectedId: string | null
  editing: string | null
  dragCursor: string | null
  onSelect: (id: string, event: ReactPointerEvent<HTMLDivElement>) => void
  onResize: (id: string, handle: DesignHandle, event: ReactPointerEvent<HTMLButtonElement>) => void
  onEdit: (id: string) => void
  onText: (id: string, text: string) => void
  onTextBlur: () => void
}) {
  const chrome = nodeChrome(node)
  const style = textStyle(node)
  const selected = node.id === selectedId
  const fill = paintCss(doc, chrome.fill, 'var(--color-koma-panel)')
  const stroke = paintCss(doc, chrome.stroke, 'var(--color-koma-border)')
  const radius = typeof chrome.radius === 'number' ? chrome.radius : Number(resolveRef(doc, chrome.radius)) || 0
  return (
    <div
      className="pointer-events-auto absolute"
      style={{
        left: node.x,
        top: node.y,
        width: node.w,
        height: node.h,
        opacity: chrome.opacity,
        background: fill,
        border: chrome.stroke === 'none' ? undefined : `${chrome.strokeWidth}px solid ${stroke}`,
        borderRadius: radius,
        outline: selected ? '1px solid var(--color-koma-accent)' : undefined,
        outlineOffset: -0.5,
        cursor: dragCursor ? undefined : 'grab',
      }}
      onPointerDown={(event) => onSelect(node.id, event)}
      onDoubleClick={(event) => {
        if (node.kind !== 'text') return
        event.stopPropagation()
        onEdit(node.id)
      }}
    >
      {node.kind === 'text' && editing === node.id ? (
        <input
          autoFocus
          value={node.text ?? ''}
          onChange={(event) => onText(node.id, event.target.value)}
          onBlur={onTextBlur}
          onPointerDown={(event) => event.stopPropagation()}
          className="z-10 h-full w-full border-0 bg-transparent px-1 text-koma-fg shadow-none outline-none"
          style={{ fontSize: style.fontSize, fontWeight: weightCss(style.weight), textAlign: style.align, color: paintCss(doc, style.color, 'var(--color-koma-fg)') }}
        />
      ) : node.kind === 'text' ? (
        <div
          className="flex h-full w-full items-center px-1"
          style={{ fontSize: style.fontSize, fontWeight: weightCss(style.weight), justifyContent: style.align === 'center' ? 'center' : style.align === 'right' ? 'flex-end' : 'flex-start', color: paintCss(doc, style.color, 'var(--color-koma-fg)') }}
        >
          <span className="truncate">{node.text || 'Text'}</span>
        </div>
      ) : node.name ? (
        <span className="pointer-events-none absolute left-2 top-1 truncate text-[11px] text-koma-dim">{node.name}</span>
      ) : null}
      {node.children?.map((child) => (
        <DesignNodeView
          key={child.id}
          doc={doc}
          node={child}
          selectedId={selectedId}
          editing={editing}
          dragCursor={dragCursor}
          onSelect={onSelect}
          onResize={onResize}
          onEdit={onEdit}
          onText={onText}
          onTextBlur={onTextBlur}
        />
      ))}
      {selected && !dragCursor ? (
        HANDLES.map((handle) => (
          <button
            key={handle.id}
            type="button"
            aria-label={`Resize ${handle.id}`}
            className="absolute h-2 w-2 rounded-full border border-koma-accent bg-koma-bg"
            style={{ left: handle.x, top: handle.y, transform: 'translate(-50%, -50%)', cursor: handle.cursor }}
            onPointerDown={(event) => onResize(node.id, handle.id, event)}
          />
        ))
      ) : null}
    </div>
  )
}

function NodeSettings({
  node,
  onPatch,
  onType,
  onTypeFocus,
  onTypeBlur,
}: {
  node: DesignNode
  onPatch: (fn: (node: DesignNode) => DesignNode) => void
  onType: (fn: (node: DesignNode) => DesignNode) => void
  onTypeFocus: () => void
  onTypeBlur: () => void
}) {
  const chrome = nodeChrome(node)
  const style = textStyle(node)
  const shaped = node.kind === 'frame' || node.kind === 'rect'
  const setField = (patch: Partial<DesignNode>, clear: (keyof DesignNode)[] = []) => {
    onPatch((current) => {
      const next: DesignNode = { ...current, ...patch }
      for (const key of clear) delete next[key]
      return next
    })
  }
  const paintChange = (field: 'fill' | 'stroke', next: string | null) => {
    const fallback = field === 'fill' ? '#1a1d27' : '#8b93b8'
    if (next === 'none') setField({ [field]: 'none' })
    else if (next == null) {
      if (shaped) setField({}, [field])
      else setField({ [field]: fallback })
    } else setField({ [field]: next })
  }
  return (
    <div className="flex flex-col gap-3 px-3 pb-3 text-[12px]">
      {node.kind !== 'rect' ? (
        <label className="flex flex-col gap-1">
          <span className="text-koma-dim">{node.kind === 'text' ? 'Text' : 'Label'}</span>
          <input
            value={node.kind === 'text' ? node.text ?? '' : node.name ?? ''}
            onFocus={onTypeFocus}
            onBlur={onTypeBlur}
            onChange={(event) => {
              const value = event.target.value
              onType((current) => (current.kind === 'text' ? { ...current, text: value } : { ...current, name: value || undefined }))
            }}
            className="h-7 rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
          />
        </label>
      ) : null}
      <PaintRow
        label="Fill"
        value={chrome.fill}
        fallback="#1a1d27"
        onChange={(next) => paintChange('fill', next)}
      />
      <PaintRow
        label="Border"
        value={chrome.stroke}
        fallback="#8b93b8"
        onChange={(next) => paintChange('stroke', next)}
      />
      <label className="flex flex-col gap-1">
        <span className="text-koma-dim">Radius</span>
        <input
          type="number"
          min={0}
          value={typeof node.radius === 'number' ? node.radius : 0}
          onChange={(event) => {
            const radius = Number(event.target.value)
            if (!Number.isFinite(radius) || radius <= 0) setField({}, ['radius'])
            else setField({ radius })
          }}
          className="h-7 rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
        />
      </label>
      {node.kind === 'text' ? (
        <>
          <label className="flex flex-col gap-1">
            <span className="text-koma-dim">Size</span>
            <input
              type="number"
              min={8}
              value={style.fontSize}
              onChange={(event) => {
                const fontSize = Number(event.target.value)
                if (!Number.isFinite(fontSize) || fontSize <= 0 || fontSize === 13) setField({}, ['fontSize'])
                else setField({ fontSize })
              }}
              className="h-7 rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
            />
          </label>
          <Choices
            label="Weight"
            value={style.weight}
            options={[
              { value: 'regular', label: 'Regular' },
              { value: 'medium', label: 'Medium' },
              { value: 'bold', label: 'Bold' },
            ]}
            onChange={(weight) => setField(weight === 'regular' ? {} : { weight }, weight === 'regular' ? ['weight'] : [])}
          />
          <Choices
            label="Align"
            value={style.align}
            options={[
              { value: 'left', label: 'Left' },
              { value: 'center', label: 'Center' },
              { value: 'right', label: 'Right' },
            ]}
            onChange={(align) => setField(align === 'left' ? {} : { textAlign: align }, align === 'left' ? ['textAlign'] : [])}
          />
          <PaintRow
            label="Color"
            value={style.color || 'none'}
            fallback="#c8d3f5"
            onChange={(next) => setField(next && next !== 'none' ? { color: next } : {}, next && next !== 'none' ? [] : ['color'])}
          />
        </>
      ) : null}
    </div>
  )
}

function PaintRow({ label, value, fallback, onChange }: { label: string; value: string; fallback: string; onChange: (next: string | null) => void }) {
  const on = value !== 'none'
  const hex = value.startsWith('#') ? value : fallback
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center justify-between">
        <span className="text-koma-dim">{label}</span>
        <button
          type="button"
          aria-pressed={on}
          onClick={() => onChange(on ? 'none' : null)}
          className={`h-6 rounded px-2 ${on ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover'}`}
        >
          {on ? 'On' : 'Off'}
        </button>
      </div>
      <label className="relative h-7 overflow-hidden rounded border border-koma-border">
        <span className="absolute inset-0" style={{ background: value.startsWith('#') ? value : on ? 'var(--color-koma-panel)' : 'transparent' }} />
        <input
          type="color"
          aria-label={`${label} color`}
          value={hex}
          onChange={(event) => onChange(event.target.value.toLowerCase())}
          className="absolute inset-0 cursor-pointer opacity-0"
        />
      </label>
    </div>
  )
}

function Choices<T extends string>({ label, value, options, onChange }: { label: string; value: T; options: { value: T; label: string }[]; onChange: (value: T) => void }) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-koma-dim">{label}</span>
      <div className="flex gap-0.5">
        {options.map((option) => (
          <button
            key={option.value}
            type="button"
            aria-pressed={option.value === value}
            onClick={() => onChange(option.value)}
            className={`h-7 flex-1 rounded text-[12px] ${option.value === value ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover'}`}
          >
            {option.label}
          </button>
        ))}
      </div>
    </div>
  )
}
