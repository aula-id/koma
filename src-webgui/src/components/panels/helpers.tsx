import { useEffect, useId, useLayoutEffect, useRef, useState, type FormEvent, type KeyboardEvent, type ReactNode } from 'react'
import { useKoma } from '../../store/koma'
import { Plus, Pencil, Trash2, Check, X, ChevronLeft, CircleHelp, FolderOpen, Settings, type LucideIcon } from 'lucide-react'

// Shared presentational atoms for sidebar CRUD panels.

export function IconBtn({
  children,
  label,
  onClick,
  tone,
  faded,
}: {
  children: ReactNode
  label: string
  onClick?: () => void
  tone?: 'emerald' | 'red'
  faded?: boolean
}) {
  const hover = tone === 'emerald' ? 'hover:!text-emerald-500' : tone === 'red' ? 'hover:!text-red-500' : ''
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      title={label}
      className={`flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg transition ${
        faded ? 'opacity-0 group-hover:opacity-60 group-has-[:focus-visible]:opacity-60 focus-visible:!opacity-100' : 'opacity-70'
      } hover:!opacity-100 ${hover}`}
    >
      {children}
    </button>
  )
}

type RowProps = {
  leading?: ReactNode
  title: string
  subtitle?: string
  right?: ReactNode
  confirmLabel: string
  armed: boolean
  onEdit?: () => void
  onArm: () => void
  onDisarm: () => void
  onConfirm: () => void
}

export function Row({ leading, title, subtitle, right, confirmLabel, armed, onEdit, onArm, onDisarm, onConfirm }: RowProps) {
  if (armed) {
    return (
      <div className="flex min-h-[42px] items-center gap-2 px-3 py-1.5">
        <span className="flex-1 truncate text-[12px] text-koma-fg">{confirmLabel}</span>
        <IconBtn label="Confirm" tone="emerald" onClick={onConfirm}>
          <Check size={14} />
        </IconBtn>
        <IconBtn label="Cancel" tone="red" onClick={onDisarm}>
          <X size={14} />
        </IconBtn>
      </div>
    )
  }
  return (
    <div className="group flex min-h-[42px] items-center gap-2.5 px-3 py-1.5 hover:bg-koma-hover">
      {leading}
      {/* Idle: title uses the full row. Hover expands the action gutter below,
          which shrinks this flex slot so ellipsis moves to the button edge. */}
      <button onClick={onEdit} disabled={!onEdit} className="min-w-0 flex-1 text-left disabled:cursor-default">
        <div className="truncate text-[13px] text-koma-fg">{title}</div>
        {subtitle && <div className="truncate text-[11px] text-koma-fg opacity-45">{subtitle}</div>}
      </button>
      {right && <div className="flex-none">{right}</div>}
      {/* Idle: max-w-0 so actions take no flex width (no permanent empty gutter).
          Hover: expand and show. opacity alone would still reserve space. */}
      <div
        className={`flex max-w-0 flex-none items-center overflow-hidden opacity-0 transition-[max-width,opacity] duration-100 group-hover:opacity-100 group-has-[:focus-visible]:opacity-100 ${
          onEdit
            ? 'group-hover:max-w-[44px] group-has-[:focus-visible]:max-w-[44px]'
            : 'group-hover:max-w-[20px] group-has-[:focus-visible]:max-w-[20px]'
        }`}
      >
        {onEdit && (
          <IconBtn label="Edit" onClick={onEdit}>
            <Pencil size={13} />
          </IconBtn>
        )}
        <IconBtn label="Delete" tone="red" onClick={onArm}>
          <Trash2 size={13} />
        </IconBtn>
      </div>
    </div>
  )
}

export function Empty({ children }: { children: ReactNode }) {
  return <div className="px-5 py-2 text-[12px] leading-snug text-koma-fg opacity-75">{children}</div>
}

export type EmptyAction = {
  /** Sentence case, one verb, no trailing full stop, max 24 chars. */
  label: string
  onClick: () => void
  icon?: ReactNode
  disabled?: boolean
  /** Shown as visible text under the button and linked via aria-describedby. */
  disabledReason?: string
}

const emptyBtn =
  'flex min-h-[28px] w-full items-center justify-center gap-1.5 rounded px-3 py-1.5 text-center text-[12px] leading-tight focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-koma-accent disabled:cursor-not-allowed disabled:opacity-50'

// Welcome view for a panel (or a panel's first-run state) with nothing in it.
// At most one primary and one secondary action. A single empty section of an
// otherwise populated panel uses the quiet one-line `Empty` instead.
export function EmptyState({
  icon: Icon,
  title,
  children,
  primary,
  secondary,
  hint,
  footer,
}: {
  icon?: LucideIcon
  title: string
  children: ReactNode
  primary?: EmptyAction
  secondary?: EmptyAction
  hint?: ReactNode
  footer?: ReactNode
}) {
  const id = useId()
  const button = (a: EmptyAction, kind: 'primary' | 'secondary', suffix: string) => {
    const reasonId = `${id}-${suffix}`
    const showReason = a.disabled && a.disabledReason
    return (
      <>
        <button
          type="button"
          onClick={a.onClick}
          disabled={a.disabled}
          aria-describedby={showReason ? reasonId : undefined}
          className={`${emptyBtn} ${
            kind === 'primary'
              ? 'bg-koma-accent font-medium text-koma-bg hover:opacity-90'
              : 'border border-koma-border text-koma-fg opacity-90 hover:bg-koma-hover hover:opacity-100'
          }`}
        >
          {a.icon ?? (kind === 'primary' ? <Plus size={14} /> : null)}
          {a.label}
        </button>
        {showReason && (
          <p id={reasonId} className="text-[11px] leading-snug text-koma-fg opacity-75">
            {a.disabledReason}
          </p>
        )}
      </>
    )
  }
  return (
    <div role="region" aria-label={title} className="flex flex-col items-center gap-2 px-4 py-6 text-center">
      {Icon && <Icon size={24} strokeWidth={1.5} aria-hidden className="mb-1 text-koma-dim" />}
      <div className="text-[13px] font-semibold text-koma-fg">{title}</div>
      <p className="text-[12px] leading-snug text-koma-fg opacity-75 [overflow-wrap:anywhere]">{children}</p>
      {(primary || secondary) && (
        <div className="mt-1 flex w-full max-w-[260px] flex-col gap-1.5">
          {primary && button(primary, 'primary', 'p')}
          {secondary && button(secondary, 'secondary', 's')}
        </div>
      )}
      {hint && <div className="text-[11px] leading-snug text-koma-fg opacity-75">{hint}</div>}
      {footer}
    </div>
  )
}

export function NoWorkspaceState() {
  const openSettingsTab = useKoma((s) => s.openSettingsTab)
  return (
    <EmptyState
      icon={FolderOpen}
      title="No project folder"
      primary={{ label: 'Open Settings', icon: <Settings size={14} />, onClick: () => openSettingsTab() }}
    >
      Koma works inside a folder on your computer. Add one under Working directories in Settings (Session section).
    </EmptyState>
  )
}

export function NoProjectState({ feature }: { feature: string }) {
  return (
    <EmptyState icon={FolderOpen} title="No project open">
      Open a project from the start screen to use {feature}.
    </EmptyState>
  )
}

// Compact rounded scope badge (replaces literal "[glob]"/"[local]" text) —
// palette-tinted: accent for global (shared across sessions), dim/neutral for
// local (this session only).
export function ScopePill({ scope }: { scope: 'global' | 'local' }) {
  return (
    <span
      title={scope}
      className={`flex-none rounded-full px-1.5 py-0.5 text-[9px] font-medium uppercase tracking-wide ${
        scope === 'global'
          ? 'bg-koma-accent/15 text-koma-accent'
          : 'bg-koma-fg/10 text-koma-fg opacity-60'
      }`}
    >
      {scope === 'global' ? 'glob' : 'local'}
    </span>
  )
}

// `text` is the visible word next to the plus; it must be the start of `label`
// (label-in-name). The word hides below 200px via the `@container` that
// AccordionSection puts on its header, so only use `text` in its action slot.
export function AddBtn({
  onClick,
  label,
  text,
  tourId,
}: {
  onClick: () => void
  label: string
  text?: string
  /** Stable driver.js / tutorial target (always discoverable; not hover-gated). */
  tourId?: string
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      aria-label={label}
      data-tour={tourId}
      className={`flex h-6 flex-none items-center justify-center gap-1 rounded text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100 focus-visible:outline focus-visible:outline-1 focus-visible:outline-koma-accent ${
        text ? 'px-1.5 text-[11px] font-medium normal-case tracking-normal' : 'w-6'
      }`}
    >
      <Plus size={14} />
      {text ? <span className="hidden @[200px]:inline">{text}</span> : null}
    </button>
  )
}

export function DetailHeader({
  onBack,
  title,
  backTourId = 'connector-back',
  tourId = 'connector-detail-header',
}: {
  onBack: () => void
  title: string
  backTourId?: string
  tourId?: string
}) {
  return (
    <div
      className="flex h-8 flex-none items-center gap-1 border-b border-koma-border px-2"
      data-tour={tourId}
    >
      <button
        onClick={onBack}
        aria-label="Back"
        data-tour={backTourId}
        className="flex h-6 w-6 items-center justify-center rounded text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
      >
        <ChevronLeft size={16} />
      </button>
      <span className="text-[12px] font-semibold text-koma-fg">{title}</span>
    </div>
  )
}

export function FormActions({ onCancel, onSave, saveDisabled, saveLabel = 'Save' }: { onCancel: () => void; onSave: () => void; saveDisabled?: boolean; saveLabel?: string }) {
  return (
    <div
      className="flex flex-none items-center justify-end gap-2 border-t border-koma-border px-3 py-2"
      data-tour="form-actions"
    >
      <button
        onClick={onCancel}
        data-tour="form-cancel"
        className="rounded px-2.5 py-1 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
      >
        Cancel
      </button>
      <button
        onClick={onSave}
        disabled={saveDisabled}
        data-tour="form-save"
        className="rounded border border-koma-border px-2.5 py-1 text-[12px] text-koma-fg transition-colors enabled:hover:bg-koma-hover disabled:opacity-40"
      >
        {saveLabel}
      </button>
    </div>
  )
}

export function InlineNameInput({
  initial,
  placeholder,
  onSubmit,
  onCancel,
}: {
  initial: string
  placeholder: string
  onSubmit: (value: string) => void
  onCancel: () => void
}) {
  const [value, setValue] = useState(initial)
  const inputRef = useRef<HTMLInputElement>(null)
  useEffect(() => {
    inputRef.current?.focus()
    inputRef.current?.select()
  }, [])
  const commit = () => {
    const v = value.trim()
    if (!v) {
      onCancel()
      return
    }
    onSubmit(v)
  }
  return (
    <form
      className="flex min-w-0 flex-1 items-center gap-1"
      onSubmit={(e: FormEvent) => {
        e.preventDefault()
        commit()
      }}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
    >
      <input
        ref={inputRef}
        value={value}
        placeholder={placeholder}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e: KeyboardEvent<HTMLInputElement>) => {
          if (e.key === 'Escape') {
            e.preventDefault()
            onCancel()
          }
        }}
        onBlur={() => {
          if (!value.trim()) onCancel()
        }}
        className="h-5 min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-1.5 text-[12px] text-koma-fg outline-none focus:border-koma-fg/40"
      />
      <IconBtn label="Confirm" tone="emerald" onClick={commit}>
        <Check size={12} />
      </IconBtn>
      <IconBtn label="Cancel" tone="red" onClick={onCancel}>
        <X size={12} />
      </IconBtn>
    </form>
  )
}

export function WelcomeLink({ which }: { which: 'diagram' | 'design' }) {
  const open = useKoma((s) => s.openWelcomeTab)
  return (
    <button type="button" onClick={() => open(which)} className="self-center px-5 py-1 text-[12px] text-koma-accent hover:underline">
      See how it works
    </button>
  )
}

const TIP_W = 240
const TIP_GAP = 6
const TIP_MARGIN = 8

/** Small (?) button after a label; click toggles a popover with the help text. */
export function HelpTip({ label, children }: { label: string; children: ReactNode }) {
  const [open, setOpen] = useState(false)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)
  const btn = useRef<HTMLButtonElement>(null)
  const pop = useRef<HTMLDivElement>(null)
  const popId = useId()

  useLayoutEffect(() => {
    if (!open) {
      setPos(null)
      return
    }
    const place = () => {
      const b = btn.current?.getBoundingClientRect()
      const p = pop.current
      if (!b || !p) return
      const vw = document.documentElement.clientWidth
      const vh = window.innerHeight
      const w = Math.min(TIP_W, vw - TIP_MARGIN * 2)
      const h = p.offsetHeight
      const left = Math.max(TIP_MARGIN, Math.min(b.left, vw - w - TIP_MARGIN))
      const below = b.bottom + TIP_GAP
      const top = below + h > vh - TIP_MARGIN && b.top - TIP_GAP - h >= TIP_MARGIN ? b.top - TIP_GAP - h : below
      setPos({ left, top })
    }
    place()
    window.addEventListener('resize', place)
    window.addEventListener('scroll', place, true)
    return () => {
      window.removeEventListener('resize', place)
      window.removeEventListener('scroll', place, true)
    }
  }, [open])

  useEffect(() => {
    if (!open) return
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node
      if (btn.current?.contains(t) || pop.current?.contains(t)) return
      setOpen(false)
    }
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.stopPropagation()
      setOpen(false)
      btn.current?.focus()
    }
    document.addEventListener('mousedown', onDown)
    document.addEventListener('keydown', onKey, true)
    return () => {
      document.removeEventListener('mousedown', onDown)
      document.removeEventListener('keydown', onKey, true)
    }
  }, [open])

  return (
    <>
      <button
        ref={btn}
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-label={`Help: ${label}`}
        aria-expanded={open}
        aria-controls={open ? popId : undefined}
        className="inline-flex h-4 w-4 flex-none items-center justify-center rounded-full text-koma-fg opacity-50 transition hover:opacity-100 focus-visible:opacity-100 focus-visible:outline focus-visible:outline-1 focus-visible:outline-offset-1 focus-visible:outline-koma-grip"
      >
        <CircleHelp size={13} strokeWidth={2} aria-hidden="true" />
      </button>
      {open && (
        <div
          ref={pop}
          id={popId}
          role="dialog"
          aria-label={label}
          style={{ position: 'fixed', left: pos?.left ?? 0, top: pos?.top ?? 0, maxWidth: TIP_W, visibility: pos ? 'visible' : 'hidden' }}
          className="z-50 w-max overflow-hidden whitespace-normal break-words rounded-md border border-koma-border bg-koma-panel px-2.5 py-2 text-[12px] font-normal normal-case leading-snug tracking-normal text-koma-fg shadow-lg"
        >
          {children}
        </div>
      )}
    </>
  )
}
