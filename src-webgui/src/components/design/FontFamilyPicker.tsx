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

const FONT_QUERY = /^[\w][\w\s,-]{0,80}$/

/**
 * Searchable font dropdown for the design inspector.
 * Menu is portaled (never an always-visible inline list) and option labels always
 * use the UI font so zoom/preview faces never look pixelated.
 */
export function FontFamilyPicker({ value, mixed, options, disabled, onChange }: FontFamilyPickerProps) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const ref = useRef<HTMLDivElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const searchRef = useRef<HTMLInputElement>(null)
  const rect = useAnchorRect(open && !disabled, ref)

  const q = query.trim().toLowerCase()
  const filtered = useMemo(() => {
    const list = q ? options.filter((name) => name.toLowerCase().includes(q)) : options
    return list.slice(0, 80)
  }, [options, q])

  const showCustom =
    !!q && FONT_QUERY.test(query.trim()) && !options.some((name) => name.toLowerCase() === q)

  useEffect(() => {
    if (!open) {
      setQuery('')
      return
    }
    const t = window.setTimeout(() => searchRef.current?.focus(), 0)
    const onDoc = (e: MouseEvent) => {
      const target = e.target as Node
      if (ref.current?.contains(target) || menuRef.current?.contains(target)) return
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

  const pick = (family: string | null) => {
    onChange(family)
    setOpen(false)
    setQuery('')
  }

  const label = mixed ? 'Mixed' : value.trim() || 'UI font'

  return (
    <div ref={ref} className="relative min-w-0 flex-1">
      <button
        type="button"
        disabled={disabled}
        aria-label="Font family"
        aria-expanded={open}
        aria-haspopup="listbox"
        onClick={() => {
          if (disabled) return
          setOpen((was) => !was)
        }}
        className="flex h-7 w-full items-center justify-between gap-1 rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg disabled:opacity-40"
      >
        <span className={`min-w-0 truncate ${mixed || !value.trim() ? 'text-koma-dim' : ''}`}>{label}</span>
        <ChevronDown size={13} className={`flex-none opacity-60 ${open ? 'rotate-180' : ''}`} />
      </button>
      {open && !disabled && rect
        ? createPortal(
            <div
              ref={menuRef}
              role="listbox"
              style={{
                position: 'fixed',
                top: rect.bottom + 4,
                left: rect.left,
                width: Math.max(rect.width, 220),
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
                  autoComplete="off"
                  spellCheck={false}
                  onChange={(e) => setQuery(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      const family = query.trim()
                      if (!family) {
                        e.preventDefault()
                        pick(null)
                        return
                      }
                      if (!FONT_QUERY.test(family)) return
                      e.preventDefault()
                      const hit = filtered.find((name) => name.toLowerCase() === family.toLowerCase())
                      pick(hit ?? family)
                    }
                  }}
                  className="h-7 w-full rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
                />
              </div>
              <div className="max-h-48 overflow-y-auto py-1">
                <button
                  type="button"
                  role="option"
                  aria-selected={!value}
                  onMouseDown={(e) => {
                    e.preventDefault()
                    pick(null)
                  }}
                  className={`flex w-full items-center gap-2 px-2 py-1.5 text-left text-[12px] ${!value ? 'bg-koma-hover text-koma-fg' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`}
                >
                  {!value ? <Check size={12} className="flex-none text-koma-accent" /> : <span className="w-3 flex-none" />}
                  UI font
                </button>
                {showCustom ? (
                  <button
                    type="button"
                    role="option"
                    onMouseDown={(e) => {
                      e.preventDefault()
                      pick(query.trim())
                    }}
                    className="flex w-full items-center gap-2 px-2 py-1.5 text-left text-[12px] text-koma-fg hover:bg-koma-hover"
                  >
                    <span className="w-3 flex-none" />
                    Use &ldquo;{query.trim()}&rdquo;
                  </button>
                ) : null}
                {filtered.map((name) => (
                  <button
                    key={name}
                    type="button"
                    role="option"
                    aria-selected={value === name}
                    title={name}
                    onMouseDown={(e) => {
                      e.preventDefault()
                      pick(name)
                    }}
                    className={`flex w-full items-center gap-2 px-2 py-1.5 text-left text-[12px] ${value === name ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-fg hover:bg-koma-hover'}`}
                  >
                    {value === name ? <Check size={12} className="flex-none" /> : <span className="w-3 flex-none" />}
                    <span className="truncate">{name}</span>
                  </button>
                ))}
                {q && !showCustom && filtered.length === 0 ? (
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
