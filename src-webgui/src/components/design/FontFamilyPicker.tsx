import { useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Check, ChevronDown } from 'lucide-react'
import { useAnchorRect } from '../panels/form'

type FontFamilyPickerProps = {
  value: string
  mixed?: boolean
  options: string[]
  disabled?: boolean
  onChange: (family: string | null) => void
}

/** Searchable font-family combobox for the design inspector (portal menu). */
export function FontFamilyPicker({ value, mixed, options, disabled, onChange }: FontFamilyPickerProps) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const ref = useRef<HTMLDivElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const searchRef = useRef<HTMLInputElement>(null)
  const rect = useAnchorRect(open && !disabled, ref)

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    const list = q ? options.filter((name) => name.toLowerCase().includes(q)) : options
    return list.slice(0, 80)
  }, [options, query])

  useEffect(() => {
    if (!open) {
      setQuery('')
      return
    }
    const t = window.setTimeout(() => searchRef.current?.focus(), 0)
    const onDoc = (e: MouseEvent) => {
      const t = e.target as Node
      if (ref.current?.contains(t) || menuRef.current?.contains(t)) return
      setOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    window.addEventListener('mousedown', onDoc)
    window.addEventListener('keydown', onKey)
    return () => {
      window.clearTimeout(t)
      window.removeEventListener('mousedown', onDoc)
      window.removeEventListener('keydown', onKey)
    }
  }, [open])

  const label = mixed ? 'Mixed' : value.trim() || 'UI font'

  const pick = (family: string | null) => {
    onChange(family)
    setOpen(false)
    setQuery('')
  }

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        disabled={disabled}
        aria-label="Font family"
        onClick={() => {
          if (disabled) return
          setOpen((o) => !o)
        }}
        className="flex h-7 w-full items-center justify-between rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg disabled:opacity-40"
      >
        <span className={`truncate ${mixed ? 'opacity-60' : ''}`}>{label}</span>
        <ChevronDown size={13} className="flex-none opacity-60" />
      </button>
      {open && !disabled && rect
        ? createPortal(
            <div
              ref={menuRef}
              style={{
                position: 'fixed',
                top: rect.bottom + 4,
                left: rect.left,
                width: Math.max(rect.width, 200),
                zIndex: 90,
              }}
              className="overflow-hidden rounded border border-koma-border bg-koma-panel shadow-xl"
            >
              <div className="border-b border-koma-border p-1.5">
                <input
                  ref={searchRef}
                  type="search"
                  value={query}
                  placeholder="Search fonts…"
                  aria-label="Search fonts"
                  onChange={(e) => setQuery(e.target.value)}
                  onKeyDown={(e) => {
                    const family = query.trim()
                    if (e.key !== 'Enter' || !family || !/^[\w][\w\s,-]{0,80}$/.test(family)) return
                    e.preventDefault()
                    pick(family)
                  }}
                  className="h-7 w-full rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
                />
              </div>
              <div className="max-h-48 overflow-y-auto py-1">
                <button
                  type="button"
                  onMouseDown={(e) => {
                    e.preventDefault()
                    pick(null)
                  }}
                  className={`flex w-full items-center gap-2 px-2 py-1 text-left text-[12px] ${!value ? 'bg-koma-hover text-koma-fg' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`}
                >
                  {!value ? <Check size={12} className="flex-none" /> : <span className="w-3 flex-none" />}
                  UI font
                </button>
                {filtered.map((name) => (
                  <button
                    key={name}
                    type="button"
                    title={name}
                    onMouseDown={(e) => {
                      e.preventDefault()
                      pick(name)
                    }}
                    className={`flex w-full items-center gap-2 px-2 py-1 text-left text-[12px] ${value === name ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-fg hover:bg-koma-hover'}`}
                    style={{ fontFamily: name }}
                  >
                    {value === name ? <Check size={12} className="flex-none" /> : <span className="w-3 flex-none" />}
                    <span className="truncate">{name}</span>
                  </button>
                ))}
                {filtered.length === 0 ? (
                  <p className="px-2 py-2 text-[11px] text-koma-dim">No matching fonts</p>
                ) : null}
              </div>
            </div>,
            document.body,
          )
        : null}
    </div>
  )
}
