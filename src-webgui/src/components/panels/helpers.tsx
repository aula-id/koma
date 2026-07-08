import type { ReactNode } from 'react'
import { Plus, Pencil, Trash2, Check, X, ChevronLeft } from 'lucide-react'

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
      onClick={onClick}
      aria-label={label}
      title={label}
      className={`flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg transition ${
        faded ? 'opacity-0 group-hover:opacity-60' : 'opacity-70'
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
      <button onClick={onEdit} disabled={!onEdit} className="min-w-0 flex-1 text-left disabled:cursor-default">
        <div className="truncate text-[13px] text-koma-fg">{title}</div>
        {subtitle && <div className="truncate text-[11px] text-koma-fg opacity-45">{subtitle}</div>}
      </button>
      {right && <div className="flex-none">{right}</div>}
      {onEdit && (
        <IconBtn label="Edit" faded onClick={onEdit}>
          <Pencil size={13} />
        </IconBtn>
      )}
      <IconBtn label="Delete" tone="red" faded onClick={onArm}>
        <Trash2 size={13} />
      </IconBtn>
    </div>
  )
}

export function Empty({ children }: { children: string }) {
  return <div className="px-5 py-1.5 text-[12px] text-koma-fg opacity-35">{children}</div>
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

export function AddBtn({ onClick, label }: { onClick: () => void; label: string }) {
  return (
    <button
      onClick={onClick}
      title={label}
      aria-label={label}
      className="flex h-5 w-5 items-center justify-center rounded text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
    >
      <Plus size={14} />
    </button>
  )
}

export function DetailHeader({ onBack, title }: { onBack: () => void; title: string }) {
  return (
    <div className="flex h-8 flex-none items-center gap-1 border-b border-koma-border px-2">
      <button
        onClick={onBack}
        aria-label="Back"
        className="flex h-6 w-6 items-center justify-center rounded text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
      >
        <ChevronLeft size={16} />
      </button>
      <span className="text-[12px] font-semibold text-koma-fg">{title}</span>
    </div>
  )
}

export function FormActions({ onCancel, onSave, saveDisabled }: { onCancel: () => void; onSave: () => void; saveDisabled?: boolean }) {
  return (
    <div className="flex flex-none items-center justify-end gap-2 border-t border-koma-border px-3 py-2">
      <button
        onClick={onCancel}
        className="rounded px-2.5 py-1 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
      >
        Cancel
      </button>
      <button
        onClick={onSave}
        disabled={saveDisabled}
        className="rounded border border-koma-border px-2.5 py-1 text-[12px] text-koma-fg transition-colors enabled:hover:bg-koma-hover disabled:opacity-40"
      >
        Save
      </button>
    </div>
  )
}
