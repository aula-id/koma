import { useEffect, useMemo, useState, useSyncExternalStore, type DragEvent as ReactDragEvent, type MouseEvent as ReactMouseEvent } from 'react'
import { ChevronRight, Circle, Component, Eye, EyeOff, Frame, Group, Lock, LockOpen, Minus, Spline, Square, Type } from 'lucide-react'
import { designLayerName, designPath, isDesignContainer, type DesignDoc, type DesignNode } from '../lib/design'
import { getDesignHover, setDesignHover, subscribeDesignHover } from '../lib/designUi'

const LAYER_MIME = 'application/x-koma-layer'
let layerDrag: { x: number; y: number; ids: string[] } | null = null

type LayerPlace = 'before' | 'after' | 'inside'
/** `range` extends from the last selected row; `toggle` flips one row. */
type LayerPick = { range: boolean; toggle: boolean }

export function DesignLayers({
  doc,
  selection,
  onSelect,
  onEnter,
  onRename,
  onVisible,
  onLocked,
  onMove,
  onMenu,
}: {
  doc: DesignDoc
  selection: string[]
  onSelect: (ids: string[], toggle: boolean) => void
  onEnter: (id: string) => void
  onRename: (id: string, name: string) => void
  onVisible: (id: string, visible: boolean) => void
  onLocked: (id: string, locked: boolean) => void
  onMove: (ids: string[], parentId: string | null, targetId: string | null, place: LayerPlace) => void
  onMenu: (id: string, clientX: number, clientY: number) => void
}) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())
  const [editing, setEditing] = useState<string | null>(null)
  const selectedId = selection[selection.length - 1] ?? null
  useEffect(() => {
    if (!selectedId) return
    const path = designPath(doc, selectedId) ?? []
    setCollapsed((current) => {
      let changed = false
      const next = new Set(current)
      for (const node of path) {
        if (node.id !== selectedId && next.delete(node.id)) changed = true
      }
      return changed ? next : current
    })
  }, [doc, selectedId])
  useEffect(() => {
    if (!selectedId) return
    document.querySelector(`[data-layer-id="${CSS.escape(selectedId)}"]`)?.scrollIntoView({ block: 'nearest' })
  }, [collapsed, selectedId])
  const screens = doc.screens.slice().reverse()
  // Rows top to bottom, as shown, so Shift ranges and multi-row drags follow what the user sees.
  const order = useMemo(() => {
    const ids: string[] = []
    const walk = (nodes: DesignNode[]) => {
      for (const node of nodes.slice().reverse()) {
        ids.push(node.id)
        if (node.children?.length && !collapsed.has(node.id)) walk(node.children)
      }
    }
    walk(doc.screens)
    return ids
  }, [collapsed, doc.screens])
  const pick = (id: string, how: LayerPick) => {
    const anchor = selection[selection.length - 1]
    const from = anchor ? order.indexOf(anchor) : -1
    const to = order.indexOf(id)
    if (how.range && from >= 0 && to >= 0) {
      onSelect(order.slice(Math.min(from, to), Math.max(from, to) + 1), false)
      return
    }
    onSelect([id], how.toggle)
  }
  const dragIds = (id: string) => (selection.includes(id) ? order.filter((item) => selection.includes(item)) : [id])
  return (
    <aside className="flex min-h-0 flex-1 flex-col" onContextMenu={(event) => event.preventDefault()}>
      <div className="min-h-0 flex-1 overflow-y-auto pb-2">
        {screens.length === 0 ? <p className="px-3 text-[12px] text-koma-dim">No layers</p> : null}
        <LayerList
          nodes={screens}
          parentId={null}
          depth={0}
          collapsed={collapsed}
          editing={editing}
          selection={selection}
          onToggle={(id) => {
            setCollapsed((current) => {
              const next = new Set(current)
              if (next.has(id)) next.delete(id)
              else next.add(id)
              return next
            })
          }}
          onEdit={setEditing}
          onSelect={pick}
          dragIds={dragIds}
          onEnter={onEnter}
          onRename={(id, name) => {
            setEditing(null)
            onRename(id, name)
          }}
          onVisible={onVisible}
          onLocked={onLocked}
          onMove={onMove}
          onMenu={onMenu}
        />
      </div>
    </aside>
  )
}

function LayerList({
  nodes,
  parentId,
  depth,
  collapsed,
  editing,
  selection,
  onToggle,
  onEdit,
  onSelect,
  dragIds,
  onEnter,
  onRename,
  onVisible,
  onLocked,
  onMove,
  onMenu,
}: {
  nodes: DesignNode[]
  parentId: string | null
  depth: number
  collapsed: Set<string>
  editing: string | null
  selection: string[]
  onToggle: (id: string) => void
  onEdit: (id: string | null) => void
  onSelect: (id: string, how: LayerPick) => void
  dragIds: (id: string) => string[]
  onEnter: (id: string) => void
  onRename: (id: string, name: string) => void
  onVisible: (id: string, visible: boolean) => void
  onLocked: (id: string, locked: boolean) => void
  onMove: (ids: string[], parentId: string | null, targetId: string | null, place: LayerPlace) => void
  onMenu: (id: string, clientX: number, clientY: number) => void
}) {
  return (
    <>
      {nodes.map((node) => {
        const children = node.children ?? []
        const open = !collapsed.has(node.id)
        const container = isDesignContainer(node.kind) && children.length > 0
        return (
          <div key={node.id}>
            <LayerRow
              node={node}
              depth={depth}
              selected={selection.includes(node.id)}
              editing={editing === node.id}
              container={container}
              open={open}
              onToggle={() => onToggle(node.id)}
              onEdit={() => onEdit(node.id)}
              onSelect={(how) => onSelect(node.id, how)}
              dragIds={() => dragIds(node.id)}
              onEnter={() => onEnter(node.id)}
              onRename={(name) => onRename(node.id, name)}
              onVisible={() => onVisible(node.id, node.visible === false)}
              onLocked={() => onLocked(node.id, !node.locked)}
              onMenu={(event) => onMenu(node.id, event.clientX, event.clientY)}
              onDrop={(ids, place) => {
                const moving = ids.filter((id) => id !== node.id)
                if (!moving.length) return
                if (place === 'inside') onMove(moving, node.id, null, 'inside')
                else onMove(moving, parentId, node.id, place)
              }}
            />
            {container && open ? (
              <LayerList
                nodes={children.slice().reverse()}
                parentId={node.id}
                depth={depth + 1}
                collapsed={collapsed}
                editing={editing}
                selection={selection}
                onToggle={onToggle}
                onEdit={onEdit}
                onSelect={onSelect}
                dragIds={dragIds}
                onEnter={onEnter}
                onRename={onRename}
                onVisible={onVisible}
                onLocked={onLocked}
                onMove={onMove}
                onMenu={onMenu}
              />
            ) : null}
          </div>
        )
      })}
    </>
  )
}

function LayerRow({
  node,
  depth,
  selected,
  editing,
  container,
  open,
  onToggle,
  onEdit,
  onSelect,
  dragIds,
  onEnter,
  onRename,
  onVisible,
  onLocked,
  onDrop,
  onMenu,
}: {
  node: DesignNode
  depth: number
  selected: boolean
  editing: boolean
  container: boolean
  open: boolean
  onToggle: () => void
  onEdit: () => void
  onSelect: (how: LayerPick) => void
  dragIds: () => string[]
  onEnter: () => void
  onRename: (name: string) => void
  onVisible: () => void
  onLocked: () => void
  onDrop: (ids: string[], place: LayerPlace) => void
  onMenu: (event: ReactMouseEvent<HTMLDivElement>) => void
}) {
  const [place, setPlace] = useState<LayerPlace | null>(null)
  const hovered = useSyncExternalStore(subscribeDesignHover, () => getDesignHover() === node.id)
  const hidden = node.visible === false
  const dropAt = (event: ReactDragEvent<HTMLDivElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect()
    const ratio = (event.clientY - bounds.top) / Math.max(1, bounds.height)
    if (isDesignContainer(node.kind) && ratio > 0.25 && ratio < 0.75) return 'inside' as const
    return ratio < 0.5 ? 'before' as const : 'after' as const
  }
  return (
    <div
      draggable={!editing}
      onDragStart={(event) => {
        layerDrag = { x: event.clientX, y: event.clientY, ids: dragIds() }
        event.dataTransfer.setData(LAYER_MIME, node.id)
        event.dataTransfer.setData('text/plain', node.id)
        event.dataTransfer.effectAllowed = 'move'
      }}
      onDragOver={(event) => {
        if (!event.dataTransfer.types.includes(LAYER_MIME)) return
        event.preventDefault()
        event.dataTransfer.dropEffect = 'move'
        setPlace(dropAt(event))
      }}
      onDragLeave={() => setPlace(null)}
      onDrop={(event) => {
        event.preventDefault()
        event.stopPropagation()
        const next = place ?? dropAt(event)
        setPlace(null)
        const start = layerDrag
        layerDrag = null
        if (start && Math.hypot(event.clientX - start.x, event.clientY - start.y) < 8) return
        const dragId = event.dataTransfer.getData(LAYER_MIME)
        const ids = start?.ids.length ? start.ids : dragId ? [dragId] : []
        if (ids.length) onDrop(ids, next)
      }}
      data-layer-id={node.id}
      className={`group relative flex h-7 items-center gap-1 pr-1 text-[12px] ${selected ? 'bg-koma-accent/20 text-koma-fg' : hovered ? 'bg-koma-hover text-koma-fg/80' : 'text-koma-fg/80'} ${hidden ? 'opacity-45' : ''}`}
      style={{ paddingLeft: 8 + depth * 14 }}
      onMouseEnter={() => setDesignHover(node.id)}
      onMouseLeave={() => setDesignHover(null)}
      onClick={(event) => onSelect({ range: event.shiftKey, toggle: event.metaKey || event.ctrlKey })}
      onContextMenu={(event) => {
        event.preventDefault()
        event.stopPropagation()
        onMenu(event)
      }}
      onDoubleClick={(event) => {
        event.stopPropagation()
        if (event.ctrlKey && (isDesignContainer(node.kind) || node.kind === 'instance')) {
          onEnter()
          return
        }
        onEdit()
      }}
    >
      {place === 'before' ? <span className="absolute inset-x-1 top-0 h-px bg-koma-accent" /> : null}
      {place === 'after' ? <span className="absolute inset-x-1 bottom-0 h-px bg-koma-accent" /> : null}
      {place === 'inside' ? <span className="absolute inset-x-1 inset-y-0.5 rounded ring-1 ring-inset ring-koma-accent" /> : null}
      <button
        type="button"
        aria-label={open ? 'Collapse' : 'Expand'}
        className={`flex h-4 w-4 flex-none items-center justify-center ${container ? '' : 'invisible'}`}
        onClick={(event) => {
          event.stopPropagation()
          onToggle()
        }}
      >
        <ChevronRight size={12} className={open ? 'rotate-90' : ''} />
      </button>
      <LayerIcon kind={node.kind} />
      {editing ? (
        <input
          autoFocus
          defaultValue={node.name ?? ''}
          placeholder={designLayerName(node)}
          aria-label="Layer name"
          onClick={(event) => event.stopPropagation()}
          onBlur={(event) => onRename(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') onRename((event.target as HTMLInputElement).value)
            if (event.key === 'Escape') onRename(node.name ?? '')
          }}
          className="h-5 min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-1 text-[12px] text-koma-fg outline-none"
        />
      ) : (
        <span className="min-w-0 flex-1 truncate">{designLayerName(node)}</span>
      )}
      <button
        type="button"
        aria-label={node.locked ? 'Unlock' : 'Lock'}
        aria-pressed={!!node.locked}
        className={`flex h-5 w-5 flex-none items-center justify-center ${node.locked ? 'text-koma-fg' : 'opacity-0 group-hover:opacity-100'}`}
        onClick={(event) => {
          event.stopPropagation()
          onLocked()
        }}
      >
        {node.locked ? <Lock size={12} /> : <LockOpen size={12} />}
      </button>
      <button
        type="button"
        aria-label={hidden ? 'Show' : 'Hide'}
        aria-pressed={!hidden}
        className={`flex h-5 w-5 flex-none items-center justify-center ${hidden ? 'text-koma-fg' : 'opacity-0 group-hover:opacity-100'}`}
        onClick={(event) => {
          event.stopPropagation()
          onVisible()
        }}
      >
        {hidden ? <EyeOff size={12} /> : <Eye size={12} />}
      </button>
    </div>
  )
}

function LayerIcon({ kind }: { kind: DesignNode['kind'] }) {
  const props = { size: 12, strokeWidth: 2 }
  if (kind === 'frame') return <Frame {...props} />
  if (kind === 'group') return <Group {...props} />
  if (kind === 'ellipse') return <Circle {...props} />
  if (kind === 'line') return <Minus {...props} />
  if (kind === 'vector') return <Spline {...props} />
  if (kind === 'text') return <Type {...props} />
  if (kind === 'instance') return <Component {...props} className="text-[#9747ff]" />
  return <Square {...props} />
}
