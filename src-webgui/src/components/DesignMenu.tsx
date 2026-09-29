import { useEffect, useRef } from 'react'

export type DesignMenuItem = {
  id: string
  label: string
  shortcut?: string
  disabled?: boolean
  danger?: boolean
  children?: DesignMenuItem[]
}

export function DesignMenu({
  x,
  y,
  items,
  onPick,
  onClose,
}: {
  x: number
  y: number
  items: DesignMenuItem[]
  onPick: (id: string) => void
  onClose: () => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    const onPointer = (event: PointerEvent) => {
      if (ref.current?.contains(event.target as Node)) return
      onClose()
    }
    window.addEventListener('keydown', onKey)
    window.addEventListener('pointerdown', onPointer)
    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('pointerdown', onPointer)
    }
  }, [onClose])
  const left = Math.min(x, window.innerWidth - 240)
  const top = Math.min(y, window.innerHeight - 360)
  return (
    <div
      ref={ref}
      role="menu"
      className="fixed z-50 w-[220px] rounded-md border border-koma-border bg-koma-panel py-1 text-[12px] text-koma-fg shadow-lg"
      style={{ left, top }}
    >
      {items.map((item) => (
        <MenuRow key={item.id} item={item} onPick={onPick} />
      ))}
    </div>
  )
}

function MenuRow({ item, onPick }: { item: DesignMenuItem; onPick: (id: string) => void }) {
  if (item.id === 'divider' || item.id.startsWith('divider-')) return <div className="my-1 h-px bg-koma-border" role="separator" />
  return (
    <div className="group relative">
      <button
        type="button"
        role="menuitem"
        disabled={item.disabled}
        onClick={() => {
          if (!item.children?.length) onPick(item.id)
        }}
        className={`flex h-7 w-full items-center px-3 text-left ${item.disabled ? 'text-koma-dim/50' : 'hover:bg-koma-hover'} ${item.danger ? 'text-koma-error' : ''}`}
      >
        <span className="min-w-0 flex-1 truncate">{item.label}</span>
        {item.children?.length ? <span className="text-koma-dim">›</span> : null}
        {item.shortcut ? <span className="ml-3 text-koma-dim">{item.shortcut}</span> : null}
      </button>
      {item.children?.length ? (
        <div className="invisible absolute left-full top-0 z-50 ml-1 w-[200px] rounded-md border border-koma-border bg-koma-panel py-1 shadow-lg group-hover:visible">
          {item.children.map((child) => (
            <MenuRow key={child.id} item={child} onPick={onPick} />
          ))}
        </div>
      ) : null}
    </div>
  )
}
