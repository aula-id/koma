import { useMemo, useState } from 'react'
import { Check, Power, Trash2, X } from 'lucide-react'
import { useKoma } from '../store/koma'

type Props = {
  cookingIds: string[]
  historyIds: string[]
  /** Cooking rows that are the currently-attached session (need detachSession). */
  foregroundCookingIds?: string[]
  onDone: () => void
  onClear: () => void
  className?: string
}

// Compact bulk action strip swapped into an existing session-list header when
// multi-select is non-empty (never inserted as an extra band above the list —
// that shoved rows down on first click). Kill applies only to cooking ids;
// Delete forever only to history. Confirm reuses this same slot (same visual
// language as SessionRowConfirmStrip) so bulk destructive ops never one-click-fire.
//
// Callers own outer padding so height matches the idle header they replace
// (overlay Cooking: `px-3 pb-1 pt-2`; welcome Recent: parent `px-4 pt-4` + `mb-2`).
export function SessionBulkBar({
  cookingIds,
  historyIds,
  foregroundCookingIds = [],
  onDone,
  onClear,
  className = '',
}: Props) {
  const req = useKoma((s) => s.req)
  const markDying = useKoma((s) => s.markDying)
  const detachSession = useKoma((s) => s.detachSession)
  const theme = useKoma((s) => s.config.theme)
  const palettes = useKoma((s) => s.config.palettes)
  const [pending, setPending] = useState<null | 'kill' | 'delete'>(null)

  const errorTint = useMemo(() => {
    const active = palettes.find((p) => p.name === theme)
    return active?.colors?.[9] || 'var(--koma-fg)'
  }, [palettes, theme])

  const nCook = cookingIds.length
  const nHist = historyIds.length
  const total = nCook + nHist
  if (total === 0) return null

  const runKill = () => {
    const fg = new Set(foregroundCookingIds)
    for (const id of cookingIds) {
      req({ r: 'KillSession', id })
      markDying(id, 'kill')
      if (fg.has(id)) detachSession()
    }
    setPending(null)
    onDone()
  }

  const runDelete = () => {
    for (const id of historyIds) {
      req({ r: 'DeleteSession', id })
      markDying(id, 'delete')
    }
    setPending(null)
    onDone()
  }

  if (pending === 'kill') {
    return (
      <div
        className={`flex w-full items-center justify-between text-[12px] font-medium ${className}`}
        style={{ color: errorTint, backgroundColor: `color-mix(in srgb, ${errorTint} 16%, transparent)` }}
        onMouseDown={(e) => e.stopPropagation()}
        onClick={(e) => e.stopPropagation()}
      >
        <span className="min-w-0 flex-1 truncate">
          {nCook === 1 ? 'kill 1 session?' : `kill ${nCook} sessions?`}
        </span>
        <span className="flex flex-none items-center gap-1.5">
          <button
            onClick={runKill}
            autoFocus
            aria-label="Confirm kill"
            className="flex flex-none items-center gap-1 rounded px-3 text-[12px] font-semibold opacity-90 transition-opacity hover:opacity-100"
            style={{ color: errorTint }}
          >
            <Check size={13} className="flex-none" />
            yes
          </button>
          <button
            onClick={() => setPending(null)}
            aria-label="Cancel"
            className="flex flex-none items-center gap-1 rounded px-3 text-[12px] text-koma-fg opacity-70 transition-opacity hover:opacity-100"
          >
            <X size={13} className="flex-none" />
            no
          </button>
        </span>
      </div>
    )
  }

  if (pending === 'delete') {
    return (
      <div
        className={`flex w-full items-center justify-between text-[12px] font-medium ${className}`}
        style={{ color: errorTint, backgroundColor: `color-mix(in srgb, ${errorTint} 16%, transparent)` }}
        onMouseDown={(e) => e.stopPropagation()}
        onClick={(e) => e.stopPropagation()}
      >
        <span className="min-w-0 flex-1 truncate">
          {nHist === 1 ? 'delete 1 forever?' : `delete ${nHist} forever?`}
        </span>
        <span className="flex flex-none items-center gap-1.5">
          <button
            onClick={runDelete}
            autoFocus
            aria-label="Confirm delete"
            className="flex flex-none items-center gap-1 rounded px-3 text-[12px] font-semibold opacity-90 transition-opacity hover:opacity-100"
            style={{ color: errorTint }}
          >
            <Check size={13} className="flex-none" />
            yes
          </button>
          <button
            onClick={() => setPending(null)}
            aria-label="Cancel"
            className="flex flex-none items-center gap-1 rounded px-3 text-[12px] text-koma-fg opacity-70 transition-opacity hover:opacity-100"
          >
            <X size={13} className="flex-none" />
            no
          </button>
        </span>
      </div>
    )
  }

  // Hub palette is narrow (~340px) and must fit Kill+Delete+Clear without
  // eating the summary mid-word. Prefer the shortest mixed form that still
  // shows both counts; full sentence is always on `title` for hover.
  // e.g. visible "2 · 1L · 1H"  title "2 selected · 1 live · 1 history"
  const summary =
    nCook > 0 && nHist > 0
      ? `${total} · ${nCook}L · ${nHist}H`
      : nCook > 0
        ? `${nCook} live`
        : nHist > 0
          ? `${nHist} history`
          : `${total} selected`
  const summaryTitle =
    nCook > 0 && nHist > 0
      ? `${total} selected · ${nCook} live · ${nHist} history`
      : nCook > 0
        ? `${nCook} live selected`
        : nHist > 0
          ? `${nHist} history selected`
          : `${total} selected`

  return (
    <div
      className={`flex w-full items-center gap-2 text-[11px] text-koma-fg ${className}`}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
    >
      <span className="min-w-0 flex-1 truncate opacity-80" title={summaryTitle}>
        {summary}
      </span>
      {nCook > 0 && (
        <button
          type="button"
          onClick={() => setPending('kill')}
          className="flex flex-none items-center gap-1 rounded px-2 py-0.5 opacity-80 transition hover:bg-koma-hover hover:opacity-100"
          title="Kill selected live sessions (stop daemon, keep on disk)"
        >
          <Power size={12} className="flex-none" />
          Kill
        </button>
      )}
      {nHist > 0 && (
        <button
          type="button"
          onClick={() => setPending('delete')}
          className="flex flex-none items-center gap-1 rounded px-2 py-0.5 opacity-80 transition hover:bg-koma-hover hover:opacity-100"
          title="Delete selected history sessions forever"
        >
          <Trash2 size={12} className="flex-none" />
          Delete
        </button>
      )}
      <button
        type="button"
        onClick={onClear}
        className="flex-none rounded px-2 py-0.5 opacity-50 transition hover:bg-koma-hover hover:opacity-100"
        title="Clear selection"
      >
        Clear
      </button>
    </div>
  )
}
