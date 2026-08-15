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

const RST = '\x1b[0m'
const ACC = '\x1b[32m'     // accent green #39ff14
const FG  = '\x1b[37m'     // fg white #e6e6e6
const DIM = '\x1b[90m'     // dim #adadad
const WARN = '\x1b[33m'    // warn amber #ffb43c
const INFO = '\x1b[36m'    // info #50c8ff
const SEL_FG = '\x1b[30m'  // selection foreground black
const SEL_BG = '\x1b[42m'  // selection background accent
const INVERSE = SEL_FG + SEL_BG + '\x1b[1m'
const SUCCESS = '\x1b[92m' // bright green #00c853

// ─── helpers ──────────────────────────────────────────────────────────

/** Strip ANSI escape codes from a string. */
function stripAnsi(s: string): string {
  return s.replace(/\x1b\[[0-9;]*m/g, '')
}

/**
 * Truncate a line to `w` visible characters, preserving ANSI state.
 * Appends a reset if the line is cut mid-sequence.
 */
function trunc(line: string, w: number): string {
  let vis = 0
  let out = ''
  const re = /\x1b\[[0-9;]*m/g
  let last = 0
  let m: RegExpExecArray | null
  while ((m = re.exec(line)) !== null) {
    const text = line.slice(last, m.index)
    for (const ch of text) {
      if (vis >= w) return out + RST
      out += ch
      vis++
    }
    out += m[0]
    last = re.lastIndex
  }
  const tail = line.slice(last)
  for (const ch of tail) {
    if (vis >= w) return out + RST
    out += ch
    vis++
  }
  return out
}

/** Pad `text` (with ANSI) on the right to `w` visible chars. */
function padRight(text: string, w: number): string {
  const vis = stripAnsi(text).length
  if (vis >= w) return text
  return text + ' '.repeat(w - vis)
}

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
  isNew?: boolean
}

const COOKING: CookingSession[] = [
  { name: 'my-project',  status: 'working', marker: '\u25cf', dir: 'src/proj',   selected: true },
  { name: 'api-server',  status: 'ready',   marker: '\u25cb', dir: 'api-server', selected: false },
  { name: 'new-session', status: '',         marker: '>',      dir: '',            selected: false, isNew: true },
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
  // Row 0 (or 11): border
  const borderStyle = focused ? ACC : DIM
  lines.push(borderStyle + borderLine('cooking (3)', W) + RST)

  // Session rows
  for (const s of COOKING) {
    if (s.isNew) {
      // NewSession kind: accent-styled marker + name
      lines.push(lineW('  ' + ACC + s.marker + ' ' + s.name + RST, W))
    } else if (s.selected && focused) {
      // Selected row: INVERSE highlight (only when cooking pane is focused)
      const vis = '  ' + s.marker + ' ' + s.status + '  ' + s.name + '  ' + s.dir
      lines.push(INVERSE + vis.padEnd(W) + RST)
    } else {
      // Normal row
      const marker = s.status === 'working'
        ? ACC + s.marker + ' ' + s.status + RST
        : DIM + s.marker + ' ' + s.status + RST
      lines.push(lineW('  ' + marker + '  ' + FG + s.name + RST + DIM + '  ' + s.dir + RST, W))
    }
  }

  // Pad remaining rows to COOKING_ROWS (11)
  while (lines.length < COOKING_ROWS) lines.push(lineW('', W))
}

/** Render the history pane border + search + entries into `lines`. */
function buildHistoryPane(lines: string[], focused: boolean, query?: string) {
  // Row 11: border
  const borderStyle = focused ? ACC : DIM
  lines.push(borderStyle + borderLine('history (5)', W) + RST)

  // Row 12: search line
  const cursor = query
    ? FG + query + '\u{2588}' + RST
    : FG + '\u{2588}' + RST
  lines.push(lineW(DIM + '  \u25b8 ' + RST + cursor, W))

  // Rows 13-21: history entries (up to 9 visible rows)
  const entries = query
    ? HISTORY.filter(h => h.name.toLowerCase().includes(query.toLowerCase()))
    : HISTORY

  for (const h of entries) {
    if (focused) {
      // Focused pane: plain dim entries (first one auto-selected visually is up to caller)
      lines.push(lineW('    ' + DIM + h.name + '  ' + h.dir + '  ' + h.age + RST, W))
    } else {
      // Unfocused pane: dim entries
      lines.push(lineW('    ' + DIM + h.name + '  ' + h.dir + '  ' + h.age + RST, W))
    }
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
  lines.push(INVERSE + (' ' + killMsg).padEnd(W) + RST)

  return lines.slice(0, rows).join('\n')
}

// ─── Tutorial Steps ───────────────────────────────────────────────────

import type { TutorialStep } from './first-run-tutorial'

/** Build the tutorial steps for a given terminal row count. */
export function getResumeSteps(rows = 24): TutorialStep[] {
  return [
    {
      title: 'Session Hub',
      narration:
        'Type /resume to open the session hub — a full-screen view of all your active and past sessions. ' +
        'The top half lists live "cooking" sessions with their status, while the bottom half shows session history.',
      points: [
        'Tab switches focus between the cooking (live) and history panes',
        'The focused pane gets an accent-colored header rule',
        'The current session is shown in italic+underline',
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
