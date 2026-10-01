import { Plus } from 'lucide-react'
import { designLayerName, type DesignDoc } from '../../lib/design'

export function DesignPages({
  doc,
  activeId,
  onSelect,
  onAdd,
}: {
  doc: DesignDoc
  activeId?: string | null
  onSelect: (id: string) => void
  onAdd: () => void
}) {
  const screens = doc.screens
  return (
    <div className="flex h-8 flex-none items-center gap-1 overflow-x-auto border-t border-koma-border bg-koma-panel px-2">
      {screens.map((screen) => (
        <button
          key={screen.id}
          type="button"
          title={designLayerName(screen)}
          aria-pressed={activeId === screen.id}
          onClick={() => onSelect(screen.id)}
          className={`h-6 max-w-32 truncate rounded px-2 text-[11px] ${activeId === screen.id ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`}
        >
          {designLayerName(screen)}
        </button>
      ))}
      <button type="button" title="Add page" aria-label="Add page" onClick={onAdd} className="flex h-6 w-6 items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg">
        <Plus size={13} />
      </button>
    </div>
  )
}
