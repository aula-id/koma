/**
 * Tutorial screens for /todo — the task list overlay.
 *
 * Layout matches src-agent/src/view/todo/mod.rs exactly: bordered overlay
 * anchored above composer, two-pane split with LIST_W=24 left pane
 * (Block::borders(RIGHT)) and right pane (Margin{horizontal:1}).
 *
 * Step 1: in-progress item selected
 * Step 2: pending item selected
 */

import { RST, ACC, FG, DIM, INVERSE, trunc, padRight, bar, line80, chatHeader, chatInput, commandEntryScreen } from './chat-chrome'

// ─── Todo overlay builder ─────────────────────────────────────────────
// Matches Rust: bordered overlay, LIST_W=24 left pane (RIGHT border),
// right pane with Margin{horizontal:1}, " todo (N/M) " title.

interface TodoItem {
  symbol: string
  label: string
  status: string
  priority: string
  content: string
  completed: boolean
}

function truncatePlain(s: string, max: number): string {
  if (s.length <= max) return s
  return s.slice(0, max - 1) + '\u2026'
}

function buildTodoOverlay(items: TodoItem[], selectedIdx: number, rows: number): string {
  const W = 80
  const OVERLAY_H = 16 // total height including borders
  const done = items.filter(i => i.completed).length
  const innerW = W - 2 // 78 (inside outer borders)

  // Two-pane split: [LIST_W=24, Min(0)]
  const LIST_W = 24
  const LIST_TEXT_W = 23 // 24 - 1 (RIGHT border)
  const RIGHT_W = innerW - LIST_W // 54
  const RIGHT_TEXT_W = RIGHT_W - 2 // 52 (Margin{horizontal:1})

  // ── Top border ──
  const title = ` todo (${done}/${items.length}) `
  const overlayLines: string[] = []
  overlayLines.push(DIM + '\u250c' + title + bar('\u2500', innerW - title.length) + '\u2510' + RST)

  // ── Build left pane rows (independent of right pane) ──
  const leftRows: string[] = []
  for (let i = 0; i < items.length; i++) {
    const item = items[i]
    const sel = i === selectedIdx
    const labelTrunc = truncatePlain(item.label, LIST_TEXT_W - 2) // 21 chars max
    const text = item.symbol + ' ' + labelTrunc

    if (sel) {
      leftRows.push(INVERSE + padRight(text, LIST_TEXT_W) + RST)
    } else {
      // Symbol always dim; label dim if completed/cancelled, fg otherwise
      const labelDim = item.completed || item.status === 'cancelled'
      leftRows.push(
        padRight(
          DIM + item.symbol + ' ' + RST +
          (labelDim ? DIM : FG) + labelTrunc + RST,
          LIST_TEXT_W
        )
      )
    }
  }

  // ── Build right pane rows (detail for selected item) ──
  const sel = items[selectedIdx]
  const rightRows: string[] = []

  // Line 1: status + priority
  const sc = sel.status === 'in-progress' ? ACC : sel.completed ? DIM : FG
  rightRows.push(
    FG + 'status: ' + sc + sel.status + FG + '   \u00b7   priority: ' + sel.priority + RST
  )

  // Line 2: status-dependent hint
  const hints: Record<string, [string, string]> = {
    'in-progress': ['(currently being worked on)', ACC],
    completed:     ['(completed)', DIM],
    cancelled:     ['(cancelled)', DIM],
    pending:       ['(awaiting model or user action)', DIM],
  }
  const hint = hints[sel.status] || hints.pending
  rightRows.push(hint[1] + hint[0] + RST)

  // Line 3: blank
  rightRows.push('')

  // Line 4: "content:" header
  rightRows.push(DIM + 'content:' + RST)

  // Lines 5+: word-wrapped content
  const words = sel.content.split(' ')
  let contentLine = ''
  for (const word of words) {
    const test = contentLine ? contentLine + ' ' + word : word
    if (contentLine && test.length > RIGHT_TEXT_W) {
      rightRows.push(FG + contentLine + RST)
      contentLine = word
    } else {
      contentLine = test
    }
  }
  if (contentLine) rightRows.push(FG + contentLine + RST)

  // ── Pad both panes to content height ──
  const contentH = OVERLAY_H - 2 // 10 rows of content
  while (leftRows.length < contentH) leftRows.push(' '.repeat(LIST_TEXT_W))
  while (rightRows.length < contentH) rightRows.push('')

  // ── Combine panes row by row ──
  // Each row: │(1) + left(23) + │(1) + rightPane(54) + │(1) = 80
  // rightPane = Margin{h:1}: ' ' + text padded to 52 + ' '
  for (let i = 0; i < contentH; i++) {
    const rightContent = padRight(' ' + rightRows[i], RIGHT_W)
    overlayLines.push(
      DIM + '\u2502' + RST
      + leftRows[i]
      + DIM + '\u2502' + RST
      + rightContent
      + DIM + '\u2502' + RST
    )
  }

  // ── Bottom border ──
  overlayLines.push(DIM + '\u2514' + bar('\u2500', innerW) + '\u2518' + RST)

  // ── Compose full screen ──
  const lines: string[] = []
  lines.push(...chatHeader())

  const inputBar = chatInput('/todo')
  const targetStart = rows - inputBar.length - overlayLines.length
  while (lines.length < targetStart) lines.push('')
  lines.push(...overlayLines)
  lines.push(...inputBar)

  while (lines.length < rows) lines.push('')
  return lines.slice(0, rows).map(l => line80(l)).join('\n')
}

// ─── Tutorial Steps ───────────────────────────────────────────────────

import type { TutorialStep } from './first-run-tutorial'

export function getTodoSteps(rows = 24): TutorialStep[] {
  const items: TodoItem[] = [
    { symbol: '\u25d0', label: 'Implement auth middleware', status: 'in-progress', priority: 'high',   content: 'Add JWT-based authentication middleware to the API gateway. Must support token refresh and role-based access control.', completed: false },
    { symbol: '\u25cb', label: 'Add unit tests',           status: 'pending',    priority: 'medium', content: 'Write comprehensive unit tests for the auth module covering edge cases like expired tokens and invalid signatures.', completed: false },
    { symbol: '\u25cf', label: 'Update README',            status: 'completed',  priority: 'high',   content: 'Update the project README with new setup instructions, API documentation, and contribution guidelines.', completed: true },
    { symbol: '\u25cb', label: 'Write migration script',   status: 'pending',    priority: 'high',   content: 'Create a database migration script to add the users table with proper indexes and constraints.', completed: false },
    { symbol: '\u2298', label: 'Refactor utils',           status: 'cancelled',  priority: 'low',    content: 'Refactor utility functions into a separate crate. Cancelled \u2014 not enough benefit for the effort.', completed: false },
  ]

  const screen1 = buildTodoOverlay(items, 0, rows)
  const screen2 = buildTodoOverlay(items, 1, rows)

  return [
    {
      title: 'Type /todo',
      narration: 'From normal chat, type /todo in the composer and press Enter to open the model-managed task panel.',
      screen: commandEntryScreen(rows, '/todo'),
    },
    {
      title: 'Task Panel',
      narration:
        'Type /todo to view the model-managed, read-only task tracker overlay. It shows tracked tasks with their status symbols and a detail pane for the selected item.',
      points: [
        '\u25d0 in-progress  \u25cb pending  \u25cf completed  \u2298 cancelled',
        'The right pane shows status, priority, and content for the selected task',
        'Completed and cancelled items appear dimmed',
      ],
      screen: screen1,
    },
    {
      title: 'Task Detail',
      narration:
        'Navigate with \u2191\u2193 or k/j to select a different task. The detail pane updates immediately to show the status, priority, and content of the highlighted item.',
      points: [
        'The model manages checklist items; this panel has no add or delete controls',
        'Enter resets an unlocked non-pending item to pending, signalling the model to redo it',
        'Esc closes the panel',
      ],
      screen: screen2,
    },
  ]
}
