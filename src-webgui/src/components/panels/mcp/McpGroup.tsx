import { useId, type ReactNode } from 'react'
import { ChevronRight } from 'lucide-react'

/** Label above the control, optional helper line below, one field per row. */
export function FormRow({
  label,
  hint,
  error,
  htmlFor,
  children,
}: {
  label: string
  hint?: ReactNode
  error?: string
  htmlFor?: string
  children: ReactNode
}) {
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <label htmlFor={htmlFor} className="text-[11px] font-semibold text-koma-fg opacity-75">
        {label}
      </label>
      {children}
      {error && <span className="text-[11px] text-red-400">{error}</span>}
      {hint && <span className="text-[11px] leading-snug text-koma-fg opacity-55">{hint}</span>}
    </div>
  )
}

/** Collapsible group: title plus a one-line summary so set values are visible when closed. */
export function FormGroup({
  title,
  summary,
  open,
  onToggle,
  children,
}: {
  title: string
  summary: string
  open: boolean
  onToggle: () => void
  children: ReactNode
}) {
  const bodyId = useId()
  return (
    <section className="rounded border border-koma-border">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        aria-controls={open ? bodyId : undefined}
        className="flex w-full min-w-0 items-center gap-1.5 px-2 py-2 text-left hover:bg-koma-hover"
      >
        <ChevronRight
          size={14}
          strokeWidth={2}
          className={`flex-none text-koma-fg opacity-70 transition-transform ${open ? 'rotate-90' : ''}`}
        />
        <span className="flex min-w-0 flex-col">
          <span className="truncate text-[12px] font-semibold text-koma-fg">{title}</span>
          <span className="truncate text-[11px] text-koma-fg opacity-60">{summary}</span>
        </span>
      </button>
      {open && (
        <div id={bodyId} className="flex min-w-0 flex-col gap-4 border-t border-koma-border px-2 py-3">
          {children}
        </div>
      )}
    </section>
  )
}
