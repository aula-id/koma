import { useCallback, useState, type MouseEvent as ReactMouseEvent } from 'react'

// Multi-select for Cooking / History session rows (issue #126).
//
// Interaction model (per user spec):
//   - plain click          → select only this row (highlight), do NOT open
//   - Ctrl/Cmd+click       → toggle this row in the selection set
//   - Shift+click          → range-select within the same list kind from anchor
//   - double-click / Enter → open/resume (handled by the row, not this hook)
//
// Keys are scoped by list kind so a cooking id and a history id never collide
// in the set, and bulk Kill vs Delete can partition cleanly.

export type SessionListKind = 'session' | 'history'

export type SessionSelKey = `${SessionListKind}:${string}`

export function selKey(kind: SessionListKind, id: string): SessionSelKey {
  return `${kind}:${id}`
}

export function parseSelKey(key: SessionSelKey): { kind: SessionListKind; id: string } {
  const i = key.indexOf(':')
  return { kind: key.slice(0, i) as SessionListKind, id: key.slice(i + 1) }
}

export function useSessionMultiSelect() {
  const [selected, setSelected] = useState<Set<SessionSelKey>>(() => new Set())
  // Anchor for Shift+click range — last plain / ctrl click within a kind.
  const [anchor, setAnchor] = useState<{ kind: SessionListKind; id: string } | null>(null)

  const clear = useCallback(() => {
    setSelected(new Set())
    setAnchor(null)
  }, [])

  const isSelected = useCallback(
    (kind: SessionListKind, id: string) => selected.has(selKey(kind, id)),
    [selected],
  )

  /** Ordered ids currently selected for one list kind. */
  const selectedIds = useCallback(
    (kind: SessionListKind): string[] => {
      const out: string[] = []
      for (const k of selected) {
        const p = parseSelKey(k)
        if (p.kind === kind) out.push(p.id)
      }
      return out
    },
    [selected],
  )

  /**
   * Handle a row click for selection. Returns true if the click was consumed
   * as a selection change (caller should NOT open the session).
   *
   * `orderedIds` = visible row ids in paint order for this kind (filtered list).
   */
  const onRowClick = useCallback(
    (
      e: ReactMouseEvent,
      kind: SessionListKind,
      id: string,
      orderedIds: string[],
    ): boolean => {
      // Ignore pure right-clicks (context menu is #127).
      if (e.button !== 0) return false

      const key = selKey(kind, id)
      const mod = e.metaKey || e.ctrlKey

      if (e.shiftKey && anchor && anchor.kind === kind) {
        e.preventDefault()
        const a = orderedIds.indexOf(anchor.id)
        const b = orderedIds.indexOf(id)
        if (a >= 0 && b >= 0) {
          const [lo, hi] = a < b ? [a, b] : [b, a]
          setSelected((prev) => {
            const next = new Set(prev)
            // Range replace within this kind; keep other kind's selection.
            for (const k of prev) {
              if (parseSelKey(k).kind === kind) next.delete(k)
            }
            for (let i = lo; i <= hi; i++) {
              const rid = orderedIds[i]
              if (rid) next.add(selKey(kind, rid))
            }
            return next
          })
        } else {
          // Anchor not visible (filter changed) — fall back to single select.
          setSelected(new Set([key]))
          setAnchor({ kind, id })
        }
        return true
      }

      if (mod) {
        e.preventDefault()
        setSelected((prev) => {
          const next = new Set(prev)
          if (next.has(key)) next.delete(key)
          else next.add(key)
          return next
        })
        setAnchor({ kind, id })
        return true
      }

      // Plain click: select only this row (highlight). Does not open.
      e.preventDefault()
      setSelected(new Set([key]))
      setAnchor({ kind, id })
      return true
    },
    [anchor],
  )

  const count = selected.size
  const cookingCount = selectedIds('session').length
  const historyCount = selectedIds('history').length

  return {
    selected,
    count,
    cookingCount,
    historyCount,
    isSelected,
    selectedIds,
    onRowClick,
    clear,
    /** Disarm helper for parents that also track a single armed kill/delete row. */
    hasSelection: count > 0,
  }
}
