import { useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import { AlertTriangle, Trash2 } from 'lucide-react'

type Props = {
  /** Display name of the extension being uninstalled. */
  name: string
  /** The extension's declared data directory, when it has one — named in the copy. */
  workspaceDir?: string | null
  onConfirm: () => void
  onCancel: () => void
}

// The ONE destructive-confirm both the Store tab and the Installed-Extension tab use before
// firing an uninstall — never `window.confirm` (a silent no-op inside wry, see the git-panel
// discard idiom). A centered portal dialog (so it needs no per-button anchor) that dismisses
// on Esc or a backdrop click, reusing the same createPortal + keydown pattern as
// DirtyCloseConfirm / RebaseDropConfirm. The uninstall it gates is an irreversible NUKE —
// files, agents, MCP servers, and the extension's own data directory — so the copy spells
// that out and names the data dir when one is declared.
export function UninstallExtensionConfirm({ name, workspaceDir, onConfirm, onCancel }: Props) {
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onCancel])

  const dir = workspaceDir?.trim()
  const dataDir = dir ? ` (${dir})` : ''

  return createPortal(
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center bg-black/40 p-4"
      onMouseDown={onCancel}
    >
      <div
        ref={ref}
        onMouseDown={(e) => e.stopPropagation()}
        className="flex w-full max-w-sm flex-col gap-3 rounded-md border border-koma-border bg-koma-panel p-4 shadow-lg"
      >
        <div className="flex items-start gap-2">
          <AlertTriangle size={16} className="mt-0.5 flex-none text-koma-error" />
          <div className="min-w-0 text-[12px] leading-relaxed text-koma-fg">
            Uninstall <span className="font-semibold">{name}</span>? This removes its files,
            agents, MCP servers, and its data directory{dataDir}. This cannot be undone.
          </div>
        </div>
        <div className="flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="rounded px-3 py-1 text-[11px] text-koma-fg opacity-70 transition hover:bg-koma-hover hover:opacity-100"
          >
            Cancel
          </button>
          <button
            type="button"
            autoFocus
            onClick={onConfirm}
            className="flex items-center gap-1.5 rounded bg-koma-error/15 px-3 py-1 text-[11px] font-semibold text-koma-error transition hover:bg-koma-error/25"
          >
            <Trash2 size={12} /> Uninstall
          </button>
        </div>
      </div>
    </div>,
    document.body,
  )
}
