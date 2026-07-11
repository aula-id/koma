import type { ReactNode } from 'react'
import { FileText, Plus, Minus, Undo2, Check, X } from 'lucide-react'
import type { GitFileEntry } from '../store/koma'

// Shared presentational atoms for the two "Source Control"-flavoured
// surfaces: the sidebar GitPanel and the graph tab's GraphChanges accordion
// (GK3). Split out of GitPanel.tsx so GraphChanges can reuse the exact same
// stage/unstage/discard idiom (badges, row actions, inline discard confirm)
// without duplicating it or risking a behavior change to GitPanel itself.

// git-porcelain status char -> badge tone. A/? = new (good), M = touched
// (accent), D = removed (error), R/C = a rename/copy (warn), U =
// unmerged/conflict (error, needs attention).
export const STATUS_TONE: Record<string, string> = {
  A: 'text-koma-success',
  '?': 'text-koma-success',
  M: 'text-koma-accent',
  D: 'text-koma-error',
  R: 'text-koma-warn',
  C: 'text-koma-warn',
  U: 'text-koma-error',
}

export function baseName(path: string): string {
  const parts = path.split('/')
  return parts[parts.length - 1] || path
}

export function dirName(path: string): string {
  const parts = path.split('/')
  parts.pop()
  return parts.join('/')
}

// A subtle row-hover action button (stage/unstage/discard) — invisible until
// the row is hovered/focused, mirroring VSCode's Source Control row actions.
// Always stops propagation so clicking it never also fires the row's own
// onClick (which opens the diff tab).
export function RowAction({
  title,
  onClick,
  children,
}: {
  title: string
  onClick: () => void
  children: ReactNode
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      onClick={(e) => {
        e.stopPropagation()
        onClick()
      }}
      className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-0 transition-opacity hover:bg-koma-hover group-hover:opacity-70 hover:!opacity-100 focus-visible:opacity-100"
    >
      {children}
    </button>
  )
}

// Small header-row action (Stage All / Unstage All / Discard All Changes) —
// shown on the AccordionSection header's hover-revealed `action` slot.
export function HeaderAction({
  title,
  onClick,
  children,
}: {
  title: string
  onClick: () => void
  children: ReactNode
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      onClick={(e) => {
        e.stopPropagation()
        onClick()
      }}
      className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
    >
      {children}
    </button>
  )
}

// Full-row inline confirmation, REPLACING a row's normal content — the same
// idiom SessionRowConfirmStrip uses for kill/delete.
export function DiscardConfirmRow({
  label,
  onConfirm,
  onCancel,
}: {
  label: string
  onConfirm: () => void
  onCancel: () => void
}) {
  return (
    <div
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
      className="flex min-h-[30px] items-center justify-between gap-2 bg-koma-error/10 px-3 py-1 text-[12px] font-medium text-koma-error"
    >
      <span className="min-w-0 flex-1 truncate">{label}</span>
      <span className="flex flex-none items-center gap-1">
        <button
          type="button"
          autoFocus
          onClick={onConfirm}
          aria-label="Confirm discard"
          className="flex flex-none items-center gap-1 rounded px-2 py-0.5 font-semibold opacity-90 hover:bg-koma-hover hover:opacity-100"
        >
          <Check size={12} className="flex-none" />
          discard
        </button>
        <button
          type="button"
          onClick={onCancel}
          aria-label="Cancel"
          className="flex flex-none items-center gap-1 rounded px-2 py-0.5 text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
        >
          <X size={12} className="flex-none" />
          cancel
        </button>
      </span>
    </div>
  )
}

// One GIT file row: basename (+ dim parent dir), a rename shows
// `origPath -> path` instead, and a trailing status-char badge. Click opens
// the staged/unstaged Monaco diff for this path. `onStage`/`onUnstage`/
// `onDiscard` (each optional — a row only gets the actions that apply to its
// list) render subtle hover buttons that stopPropagation so they never also
// open the diff.
export function FileRow({
  entry,
  onClick,
  onStage,
  onUnstage,
  onDiscard,
}: {
  entry: GitFileEntry
  onClick: () => void
  onStage?: () => void
  onUnstage?: () => void
  onDiscard?: () => void
}) {
  const tone = STATUS_TONE[entry.status] ?? 'text-koma-dim'
  const dir = dirName(entry.path)
  return (
    <div
      title={entry.origPath ? `${entry.origPath} -> ${entry.path}` : entry.path}
      onClick={onClick}
      className="group flex min-h-[30px] cursor-pointer items-center gap-1.5 px-3 py-1 hover:bg-koma-hover"
    >
      <FileText size={13} className="flex-none text-koma-fg opacity-45" />
      <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-koma-fg">
        {entry.origPath ? (
          <>
            <span className="opacity-60">{baseName(entry.origPath)}</span>
            <span className="opacity-40"> {'->'} </span>
            {baseName(entry.path)}
          </>
        ) : (
          <>
            {baseName(entry.path)}
            {dir && <span className="ml-1.5 text-koma-dim opacity-45">{dir}</span>}
          </>
        )}
      </span>
      {onDiscard && (
        <RowAction title="Discard changes" onClick={onDiscard}>
          <Undo2 size={13} />
        </RowAction>
      )}
      {onStage && (
        <RowAction title="Stage changes" onClick={onStage}>
          <Plus size={13} />
        </RowAction>
      )}
      {onUnstage && (
        <RowAction title="Unstage changes" onClick={onUnstage}>
          <Minus size={13} />
        </RowAction>
      )}
      <span className={`flex-none font-mono text-[11px] font-semibold ${tone}`}>{entry.status}</span>
    </div>
  )
}
