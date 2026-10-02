import { useEffect, useRef, useState, type ReactNode } from 'react'
import { ChevronDown, ChevronUp } from 'lucide-react'

export type DesignMenuItem = {
  id: string
  label: string
  shortcut?: string
  disabled?: boolean
  danger?: boolean
  children?: DesignMenuItem[]
}

const MENU_MAX = 500
const SCROLL_SPEED = 7

function HoverScroll({ children, maxHeight }: { children: ReactNode; maxHeight: number }) {
  const scrollerRef = useRef<HTMLDivElement>(null)
  const dirRef = useRef<0 | -1 | 1>(0)
  const [hover, setHover] = useState(false)
  const [canUp, setCanUp] = useState(false)
  const [canDown, setCanDown] = useState(false)

  const sync = () => {
    const el = scrollerRef.current
    if (!el) return
    setCanUp(el.scrollTop > 1)
    setCanDown(el.scrollTop + el.clientHeight < el.scrollHeight - 1)
  }

  useEffect(() => {
    const el = scrollerRef.current
    if (!el) return
    sync()
    const observer = new ResizeObserver(sync)
    observer.observe(el)
    if (el.firstElementChild) observer.observe(el.firstElementChild)
    return () => observer.disconnect()
  }, [children])

  useEffect(() => {
    let frame = 0
    const tick = () => {
      const el = scrollerRef.current
      if (el && dirRef.current) {
        el.scrollTop += dirRef.current * SCROLL_SPEED
        sync()
      }
      frame = requestAnimationFrame(tick)
    }
    frame = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(frame)
  }, [])

  return (
    <div
      className="relative"
      style={{ maxHeight }}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => {
        setHover(false)
        dirRef.current = 0
      }}
    >
      <div
        ref={scrollerRef}
        className="overflow-hidden"
        style={{ maxHeight }}
        onWheel={(event) => event.preventDefault()}
        onScroll={sync}
      >
        {children}
      </div>
      {hover && canUp ? (
        <div
          role="button"
          aria-label="Scroll up"
          className="absolute inset-x-0 top-0 z-10 flex h-7 cursor-pointer items-center justify-center bg-gradient-to-b from-koma-panel via-koma-panel/90 to-transparent text-koma-fg"
          onMouseEnter={() => { dirRef.current = -1 }}
          onMouseLeave={() => { dirRef.current = 0 }}
        >
          <ChevronUp size={16} />
        </div>
      ) : null}
      {hover && canDown ? (
        <div
          role="button"
          aria-label="Scroll down"
          className="absolute inset-x-0 bottom-0 z-10 flex h-7 cursor-pointer items-center justify-center bg-gradient-to-t from-koma-panel via-koma-panel/90 to-transparent text-koma-fg"
          onMouseEnter={() => { dirRef.current = 1 }}
          onMouseLeave={() => { dirRef.current = 0 }}
        >
          <ChevronDown size={16} />
        </div>
      ) : null}
    </div>
  )
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
  const maxHeight = Math.min(MENU_MAX, Math.max(120, window.innerHeight - 16))
  const left = Math.min(Math.max(8, x), window.innerWidth - 228)
  const top = y + maxHeight > window.innerHeight - 8 ? Math.max(8, window.innerHeight - maxHeight - 8) : y
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    const onPointer = (event: PointerEvent) => {
      if (event.button === 2) return
      const target = event.target as Element | null
      if (target?.closest('[role="menuitem"]')) return
      onClose()
    }
    window.addEventListener('keydown', onKey)
    window.addEventListener('pointerdown', onPointer, true)
    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('pointerdown', onPointer, true)
    }
  }, [onClose])
  return (
    <>
      <div
        className="fixed inset-0 z-40"
        onPointerDown={(event) => {
          if (event.button === 2) return
          event.preventDefault()
          onClose()
        }}
      />
      <div
        ref={ref}
        role="menu"
        className="fixed z-50 w-[220px] overflow-hidden rounded-md border border-koma-border bg-koma-panel text-[12px] text-koma-fg shadow-lg"
        style={{ left, top }}
        onPointerDown={(event) => {
          if (event.button !== 0) return
          if ((event.target as Element).closest('[role="menuitem"]')) return
          onClose()
        }}
      >
        <HoverScroll maxHeight={maxHeight}>
          <div className="py-1">
            {items.map((item) => (
              <MenuRow key={item.id} item={item} onPick={onPick} maxHeight={maxHeight} />
            ))}
          </div>
        </HoverScroll>
      </div>
    </>
  )
}

function MenuRow({ item, onPick, maxHeight }: { item: DesignMenuItem; onPick: (id: string) => void; maxHeight: number }) {
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
        <div className="invisible absolute left-full top-0 z-50 ml-1 w-[200px] overflow-hidden rounded-md border border-koma-border bg-koma-panel shadow-lg group-hover:visible">
          <HoverScroll maxHeight={maxHeight}>
            <div className="py-1">
              {item.children.map((child) => (
                <MenuRow key={child.id} item={child} onPick={onPick} maxHeight={maxHeight} />
              ))}
            </div>
          </HoverScroll>
        </div>
      ) : null}
    </div>
  )
}
