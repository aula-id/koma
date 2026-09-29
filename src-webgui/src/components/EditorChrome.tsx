import type { ReactNode } from 'react'
import { Code2, Eye, History, Redo2, RotateCcw, Save, Undo2 } from 'lucide-react'

export function EditorChrome({
  path,
  status,
  canSave,
  canRevert,
  saving,
  onSave,
  onRevert,
  preview = false,
  onTogglePreview,
  onHistory,
  icon,
  canUndo,
  canRedo,
  onUndo,
  onRedo,
  trailing,
}: {
  path: string
  status: string
  canSave?: boolean
  canRevert?: boolean
  saving?: boolean
  onSave?: () => void
  onRevert?: () => void
  preview?: boolean
  onTogglePreview?: () => void
  onHistory?: () => void
  icon?: ReactNode
  canUndo?: boolean
  canRedo?: boolean
  onUndo?: () => void
  onRedo?: () => void
  trailing?: ReactNode
}) {
  // Density via container query — no RO/setState. Narrow split panes hide the
  // full path (title still has it) and drop the status text so Save/Revert stay.
  return (
    <div className="@container/pathbar flex h-8 min-w-0 flex-none items-center gap-2 border-b border-koma-border bg-koma-panel px-3 text-[12px] @max-xs/pathbar:gap-1.5 @max-xs/pathbar:px-2 @max-[12rem]/pathbar:px-1.5">
      {icon ?? (preview ? <Eye size={13} className="flex-none text-koma-dim" /> : <Code2 size={13} className="flex-none text-koma-dim" />)}
      <span
        className="min-w-0 flex-1 truncate font-mono text-koma-fg @max-[12rem]/pathbar:hidden"
        title={path}
      >
        {path}
      </span>
      <span className="min-w-0 flex-none truncate text-[11px] text-koma-dim @max-xs/pathbar:max-w-[5rem] @max-[12rem]/pathbar:hidden">
        {status}
      </span>
      {onTogglePreview && (
        <button
          type="button"
          onClick={onTogglePreview}
          title={preview ? 'Open Source' : 'Open Markdown Preview'}
          aria-label={preview ? 'Open Source' : 'Open Markdown Preview'}
          className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg disabled:opacity-30"
        >
          {preview ? <Code2 size={13} /> : <Eye size={13} />}
        </button>
      )}
      {onHistory && <button type="button" onClick={onHistory} title="Local History" aria-label="Local History" className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg"><History size={13}/></button>}
      {onUndo && <button type="button" onClick={onUndo} disabled={!canUndo || saving} title="Undo" aria-label="Undo" className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg disabled:opacity-30"><Undo2 size={13}/></button>}
      {onRedo && <button type="button" onClick={onRedo} disabled={!canRedo || saving} title="Redo" aria-label="Redo" className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg disabled:opacity-30"><Redo2 size={13}/></button>}
      {onRevert && <button
        type="button"
        onClick={onRevert}
        disabled={!canRevert || saving}
        title="Revert"
        className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg disabled:opacity-30"
      >
        <RotateCcw size={13} />
      </button>}
      {onSave && <button
        type="button"
        onClick={onSave}
        disabled={!canSave}
        title="Save"
        className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg disabled:opacity-30"
      >
        <Save size={13} />
      </button>}
      {trailing}
    </div>
  )
}
