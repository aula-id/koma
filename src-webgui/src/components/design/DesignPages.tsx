import { useState } from 'react'
import { Plus, X } from 'lucide-react'
import { designLayerName, type DesignDoc } from '../../lib/design'

export function DesignPages({
  doc,
  activeId,
  onSelect,
  onAdd,
  onRename,
  onDelete,
}: {
  doc: DesignDoc
  activeId?: string | null
  onSelect: (id: string) => void
  onAdd: () => void
  onRename?: (id: string, name: string) => void
  onDelete?: (id: string) => void
}) {
  const screens = doc.screens
  const [editing, setEditing] = useState<string | null>(null)
  return (
    <div className="flex h-8 flex-none items-center gap-1 overflow-x-auto border-t border-koma-border bg-koma-panel px-2">
      {screens.map((screen) => (
        <div key={screen.id} className="flex items-center">
          {editing === screen.id ? (
            <input
              autoFocus
              aria-label="Page name"
              defaultValue={designLayerName(screen)}
              onBlur={(event) => {
                onRename?.(screen.id, event.target.value.trim())
                setEditing(null)
              }}
              onKeyDown={(event) => {
                if (event.key === 'Enter') (event.target as HTMLInputElement).blur()
                if (event.key === 'Escape') setEditing(null)
              }}
              className="h-6 w-24 rounded bg-koma-bg px-1 text-[11px] text-koma-fg outline-none"
            />
          ) : (
            <button
              type="button"
              title={designLayerName(screen)}
              aria-pressed={activeId === screen.id}
              onClick={() => onSelect(screen.id)}
              onDoubleClick={() => setEditing(screen.id)}
              className={`h-6 max-w-32 truncate rounded px-2 text-[11px] ${activeId === screen.id ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`}
            >
              {designLayerName(screen)}
            </button>
          )}
          {screens.length > 1 && onDelete ? (
            <button type="button" title="Delete page" aria-label={`Delete ${designLayerName(screen)}`} onClick={() => onDelete(screen.id)} className="flex h-6 w-5 items-center justify-center text-koma-dim hover:text-koma-fg">
              <X size={10} />
            </button>
          ) : null}
        </div>
      ))}
      <button type="button" title="Add page" aria-label="Add page" onClick={onAdd} className="flex h-6 w-6 items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg">
        <Plus size={13} />
      </button>
    </div>
  )
}
