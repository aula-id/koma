import { useEffect, useRef, useState, type InputHTMLAttributes, type ReactNode } from 'react'
import { Check, ChevronDown } from 'lucide-react'

// Reusable inline-form primitives for the sidebar CRUD panels. Themed on koma-*.

export function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1 px-3 py-1.5">
      <span className="text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-50">
        {label}
      </span>
      {children}
    </label>
  )
}

export function TextInput(props: InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      {...props}
      className="h-7 rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none placeholder:text-koma-fg placeholder:opacity-35 focus:border-koma-grip"
    />
  )
}

export function Toggle({ on, onChange }: { on: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      onClick={() => onChange(!on)}
      className={`relative h-4 w-7 flex-none rounded-full transition-colors ${on ? 'bg-emerald-500/70' : 'bg-koma-grip'}`}
    >
      <span
        className={`absolute top-0.5 h-3 w-3 rounded-full bg-white transition-all ${on ? 'left-[14px]' : 'left-0.5'}`}
      />
    </button>
  )
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T
  options: { value: T; label: string }[]
  onChange: (v: T) => void
}) {
  return (
    <div className="flex rounded border border-koma-border p-0.5">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          onClick={() => onChange(o.value)}
          className={`flex-1 rounded px-2 py-0.5 text-[11px] transition-colors ${
            value === o.value
              ? 'bg-koma-hover text-koma-fg opacity-100'
              : 'text-koma-fg opacity-55 hover:opacity-80'
          }`}
        >
          {o.label}
        </button>
      ))}
    </div>
  )
}

// Multi-select chips with a checkmark on selected options.
export function Chips<T extends string>({
  value,
  options,
  onToggle,
}: {
  value: T[]
  options: { value: T; label: string }[]
  onToggle: (v: T) => void
}) {
  return (
    <div className="flex flex-wrap gap-1">
      {options.map((o) => {
        const on = value.includes(o.value)
        return (
          <button
            key={o.value}
            type="button"
            onClick={() => onToggle(o.value)}
            className={`flex items-center gap-1 rounded border px-1.5 py-0.5 text-[11px] transition-colors ${
              on
                ? 'border-koma-grip bg-koma-hover text-koma-fg'
                : 'border-koma-border text-koma-fg opacity-55 hover:opacity-80'
            }`}
          >
            {on && <Check size={11} className="flex-none" />}
            {o.label}
          </button>
        )
      })}
    </div>
  )
}

// Dropdown select: button trigger + floating menu (a real "select" control, not
// a permanently-expanded radio pile). Options commit via onMouseDown+
// preventDefault (not onClick) so focus never races between the trigger/menu —
// the classic combobox fix. Closes + blurs on pick, Esc, or outside click.
export function Select<T extends string>({
  value,
  options,
  onChange,
  placeholder,
}: {
  value: T | ''
  options: { value: T; label: string }[]
  onChange: (v: T) => void
  placeholder?: string
}) {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    window.addEventListener('mousedown', onDoc)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onDoc)
      window.removeEventListener('keydown', onKey)
    }
  }, [open])

  const selected = options.find((o) => o.value === value)
  const pick = (v: T) => {
    onChange(v)
    setOpen(false)
    ;(document.activeElement as HTMLElement | null)?.blur()
  }

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        disabled={options.length === 0}
        className="flex h-7 w-full items-center justify-between rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg disabled:opacity-40"
      >
        <span className={`truncate ${selected ? '' : 'opacity-40'}`}>
          {selected?.label ?? placeholder ?? 'Select…'}
        </span>
        <ChevronDown size={13} className="flex-none opacity-60" />
      </button>
      {open && options.length > 0 && (
        <div className="absolute left-0 right-0 top-[calc(100%+4px)] z-10 max-h-40 overflow-y-auto rounded border border-koma-border bg-koma-panel py-1 shadow-xl">
          {options.map((o) => (
            <button
              key={o.value}
              type="button"
              onMouseDown={(e) => {
                e.preventDefault()
                pick(o.value)
              }}
              className={`flex w-full items-center gap-2 px-2 py-1 text-left text-[12px] transition-colors ${
                o.value === value
                  ? 'bg-koma-hover text-koma-fg'
                  : 'text-koma-fg opacity-75 hover:bg-koma-hover hover:opacity-100'
              }`}
            >
              {o.value === value ? (
                <Check size={12} className="flex-none" />
              ) : (
                <span className="w-3 flex-none" />
              )}
              <span className="truncate">{o.label}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  )
}

// Live-filter combobox (mirrors koma's model-id omnisearch): typing filters
// `options`; picking one commits it via onMouseDown+preventDefault (same
// focus-race fix as Select); typing something with no matches still commits as
// raw text — the text input's value IS the committed value, no separate
// confirm step. Closes + blurs on pick, Esc, or outside click.
export function Combobox({
  value,
  onChange,
  options,
  placeholder,
}: {
  value: string
  onChange: (v: string) => void
  options: string[]
  placeholder?: string
}) {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  const filtered = value.trim()
    ? options.filter((o) => o.toLowerCase().includes(value.trim().toLowerCase()))
    : options

  useEffect(() => {
    if (!open) return
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    window.addEventListener('mousedown', onDoc)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onDoc)
      window.removeEventListener('keydown', onKey)
    }
  }, [open])

  const pick = (v: string) => {
    onChange(v)
    setOpen(false)
    ;(document.activeElement as HTMLElement | null)?.blur()
  }

  return (
    <div ref={ref} className="relative">
      <TextInput
        value={value}
        placeholder={placeholder}
        onFocus={() => setOpen(true)}
        onChange={(e) => {
          onChange(e.target.value)
          setOpen(true)
        }}
      />
      {open && (
        <div className="absolute left-0 right-0 top-[calc(100%+4px)] z-10 max-h-40 overflow-y-auto rounded border border-koma-border bg-koma-panel py-1 shadow-xl">
          {filtered.length > 0 ? (
            filtered.map((o) => (
              <button
                key={o}
                type="button"
                onMouseDown={(e) => {
                  e.preventDefault()
                  pick(o)
                }}
                className="flex w-full items-center px-2 py-1 text-left text-[12px] text-koma-fg opacity-75 transition-colors hover:bg-koma-hover hover:opacity-100"
              >
                <span className="truncate">{o}</span>
              </button>
            ))
          ) : value.trim() ? (
            <div className="px-2 py-1 text-[11px] text-koma-fg opacity-45">Use “{value.trim()}”</div>
          ) : (
            <div className="px-2 py-1 text-[11px] text-koma-fg opacity-35">No matches</div>
          )}
        </div>
      )}
    </div>
  )
}
