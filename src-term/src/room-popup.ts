import { suggest, type Suggestion } from "./room-suggest.ts"

/** Where the highlight is, tied to the draft it was made for. A new draft starts over. */
export type PopupState = { draft: string; at: number; dismissed: boolean }

export const CLOSED: PopupState = { draft: "", at: 0, dismissed: false }

export type PopupView = { rows: Suggestion[]; title: string; selected: number; open: boolean }

/** What the popup shows for a draft. Editing resets the highlight and reopens a dismissed popup. */
export function popupView(draft: string, state: PopupState): PopupView {
  const { rows, title } = suggest(draft)
  const fresh = state.draft === draft
  const dismissed = fresh && state.dismissed
  const at = fresh ? state.at : 0
  return { rows, title, selected: Math.min(at, Math.max(0, rows.length - 1)), open: rows.length > 0 && !dismissed }
}

/** Up and down stop at the ends, like the coding chat's list. */
export function moved(state: PopupState, draft: string, view: PopupView, direction: -1 | 1): PopupState {
  const next = Math.max(0, Math.min(view.rows.length - 1, view.selected + direction))
  return { draft, at: next, dismissed: false }
}

export function dismissed(draft: string, view: PopupView): PopupState {
  return { draft, at: view.selected, dismissed: true }
}

/** Tab completes the highlighted row and adds a space, which closes the popup. */
export function completion(row: Suggestion): string {
  return row.insert.startsWith("/stop ") || row.insert.startsWith("/leave ") ? row.insert : `${row.label} `
}

export type Chosen = { run: string } | { insert: string }

/** Enter on a command runs it; Enter on a person writes the name and waits for the message. */
export function chosen(row: Suggestion): Chosen {
  // /stop needs to be told whom. Choosing a name under it runs it; the bare command waits for one.
  if (row.label === "/stop") return { insert: "/stop " }
  if (row.insert.startsWith("/stop ")) return { run: row.insert }
  // /leave has two ways out: switch back to the chat, or stop the room as well.
  if (row.label === "/leave") return { insert: "/leave " }
  if (row.insert.startsWith("/leave ")) return { run: row.insert }
  return row.label.startsWith("/") ? { run: row.label } : { insert: completion(row) }
}
