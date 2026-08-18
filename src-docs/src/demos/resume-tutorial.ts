/**
 * Tutorial screens for `/resume` — the session hub.
 *
 * The session hub is a FULL-SCREEN mode split into two horizontal halves:
 *   TOP    — cooking (live sessions) with name + status markers
 *   BOTTOM — history (past sessions) with live search + relative age
 *
 * The FOCUSED pane's header rule is accented; the unfocused pane is dim.
 * A one-line keybinding hint sits at the bottom.
 *
 * Layouts match src-agent/src/view/session_hub.rs exactly at 80×24:
 *
 *   Rows 0-10:   Cooking pane (11 rows)
 *     Row 0:    TOP border  ╭ cooking (3) ───...──╮  (accent, focused)
 *     Rows 1-10: Content (10 rows, margin-h 1 = text at col 2)
 *   Rows 11-22: History pane (12 rows)
 *     Row 11:   TOP border  ╭ history (5) ───...──╮  (dim, unfocused)
 *     Row 12:   Search line "› █" (dim)
 *     Rows 13-21: History entries (9 visible rows)
 *     Row 22:   empty
 *   Row 23:     Footer hint (dim)
 */

import { RST, ACC, FG, DIM, INVERSE, trunc, padRight, commandEntryScreen } from './chat-chrome'

/** Build a box-drawing top border: ╭ <title> ───...──╮  (exactly `w` visible chars). */
function borderLine(title: string, w: number): string {
  // ╭(1) + space(1) + title + space(1) + ─×N + ╮(1) = w
  const dashes = w - title.length - 4
  return '╭ ' + title + ' ' + '─'.repeat(dashes) + '╮'
}

/** Pad a line to exactly `w` visible chars, truncating if needed. */
function lineW(content: string, w: number): string {
  return padRight(trunc(content, w), w)
}

// ─── Data ─────────────────────────────────────────────────────────────

const W = 80
const COOKING_ROWS = 11   // rows 0-10
const HISTORY_ROWS = 12   // rows 11-22

interface CookingSession {
  name: string
  status: string
  marker: string
  dir: string
  selected: boolean
}

const COOKING: CookingSession[] = [
  { name: 'my-project',  status: 'working', marker: '\u25cf', dir: 'src/proj',   selected: true },
  { name: 'api-server',  status: 'ready',   marker: '\u25cb', dir: 'api-server', selected: false },
]

interface HistoryEntry {
  name: string
  dir: string
  age: string
}

const HISTORY: HistoryEntry[] = [
  { name: 'fix-auth-bug',      dir: 'src/proj',   age: '5s ago' },
  { name: 'add-rate-limiter',  dir: 'api-server', age: '2m ago' },
  { name: 'refactor-db-layer', dir: 'src/proj',   age: '1h ago' },
  { name: 'update-deps',       dir: 'src/proj',   age: '3h ago' },
  { name: 'prototype-ui',      dir: 'frontend',   age: '1d ago' },
]

// ─── Shared pane builders ─────────────────────────────────────────────

/** Render the cooking pane border + sessions into `lines`. */
function buildCookingPane(lines: string[], focused: boolean) {
  const INNER_W = W - 4 // content area (margin 2 each side)

  // Row 0: border
  const borderStyle = focused ? ACC : DIM
  lines.push(borderStyle + borderLine('cooking (3)', W) + RST)

  // [ + new session ] button
  const btnText = ' [+ new session]'
  lines.push(lineW(FG + btnText + RST, W))

  // Session rows — two-column: name LEFT, dir + status RIGHT
  for (const s of COOKING) {
    const rightPart = s.dir + '  ' + s.marker + ' ' + s.status
    const nameW = Math.max(4, INNER_W - rightPart.length - 2)
    const name = s.name.length > nameW ? s.name.slice(0, nameW - 1) + '\u2026' : s.name.padEnd(nameW)
    const sel = s.selected && focused
    if (sel) {
      lines.push(INVERSE + padRight('  ' + name + '  ' + rightPart, W) + RST)
    } else {
      const nameStyle = s.status === 'working' ? ACC : FG
      lines.push(lineW('  ' + nameStyle + name + '  ' + DIM + rightPart + RST, W))
    }
  }

  // Pad remaining rows to COOKING_ROWS (11)
  while (lines.length < COOKING_ROWS) lines.push(lineW('', W))
}

/** Render the history pane border + search + entries into `lines`. */
function buildHistoryPane(lines: string[], focused: boolean, query?: string) {
  const INNER_W = W - 4

  // Border
  const borderStyle = focused ? ACC : DIM
  lines.push(borderStyle + borderLine('history (5)', W) + RST)

  // Search line
  const cursor = query
    ? FG + query + '\u{2588}' + RST
    : FG + '\u{2588}' + RST
  lines.push(lineW(DIM + '  \u25b8 ' + RST + cursor, W))

  // History entries — two-column: name LEFT, dir + age RIGHT
  const entries = query
    ? HISTORY.filter(h => h.name.toLowerCase().includes(query.toLowerCase()))
    : HISTORY

  for (const h of entries) {
    const rightPart = h.dir + '  ' + h.age
    const nameW = Math.max(4, INNER_W - rightPart.length - 2)
    const name = h.name.length > nameW ? h.name.slice(0, nameW - 1) + '\u2026' : h.name.padEnd(nameW)
    lines.push(lineW('  ' + DIM + name + '  ' + DIM + rightPart + RST, W))
  }

  // Pad to COOKING_ROWS + HISTORY_ROWS (23)
  const historyContentEnd = COOKING_ROWS + HISTORY_ROWS
  while (lines.length < historyContentEnd) lines.push(lineW('', W))
}

// ─── Screen: Session Hub — Cooking Focused ────────────────────────────

function screenHubCooking(rows = 24): string {
  const lines: string[] = []

  buildCookingPane(lines, true)
  buildHistoryPane(lines, false)

  // Row 23: footer hint
  const footer = 'Tab switch \u00b7 \u2191\u2193 select \u00b7 Enter open \u00b7 Ctrl+X kill \u00b7 type to search \u00b7 Esc close'
  lines.push(lineW(DIM + footer + RST, W))

  return lines.slice(0, rows).join('\n')
}

// ─── Screen: Session Hub — Kill Confirmation ──────────────────────────

function screenHubKillConfirm(rows = 24): string {
  const lines: string[] = []

  buildCookingPane(lines, true)
  buildHistoryPane(lines, false)

  // Row 23: INVERSE kill confirmation bar
  const killMsg = "Stop running session 'my-project'?  Ctrl+X/Enter confirm \u00b7 Esc cancel"
  lines.push(INVERSE + padRight(' ' + killMsg, W) + RST)

  return lines.slice(0, rows).join('\n')
}

// ─── Tutorial Steps ───────────────────────────────────────────────────

import type { TutorialStep } from './first-run-tutorial'

/** Build the tutorial steps for a given terminal row count. */
export function getResumeSteps(rows = 24): TutorialStep[] {
  return [
    {
      title: 'Type /resume',
      narration: 'From normal chat, type /resume in the composer and press Enter to open the standalone Session Hub.',
      screen: commandEntryScreen(rows, '/resume'),
    },
    {
      title: 'Session Hub',
      narration:
        'The standalone Session Hub lists live and past sessions. Choosing a live cooking session swaps the daemon client foreground; choosing a history session loads it into a new appended tab. ' +
        'The top half lists live sessions with their status, while the bottom half shows session history.',
      points: [
        'Tab switches focus between the cooking (live) and history panes',
        'The focused pane gets an accent-colored header rule',
        'The foreground session is tagged (current)',
      ],
      screen: screenHubCooking(rows),
    },
    {
      title: 'Kill Confirmation',
      narration:
        'Select a running cooking session and press Ctrl+X to kill it. A confirmation bar appears at the bottom ' +
        'asking you to confirm — press Ctrl+X or Enter to confirm, or Esc to cancel.',
      points: [
        'Ctrl+X on a cooking session triggers kill confirmation',
        'The footer turns into an INVERSE confirmation prompt',
        'Press Esc to cancel and return to the session hub',
      ],
      screen: screenHubKillConfirm(rows),
    },
  ]
}
