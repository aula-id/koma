/**
 * Tutorial screens for `/task` — the sub-agents panel.
 *
 * The sub-agents panel is a bordered popup anchored above the input box,
 * drawn on top of the chat transcript. Two-pane layout inside the box:
 *   LEFT  — narrow list (18 cols) of sub-agents with id, name, status
 *   RIGHT — wide detail pane with status line + transcript
 *
 * Layouts match src-agent/src/view/chat/subagents.rs exactly.
 *
 * Overlay dimensions (80-col terminal, 12-row box):
 *   Outer box: 80 chars including │ borders
 *   Left pane: 18 cols with │ RIGHT border (17 inner text cols)
 *   Right pane: 60 cols after │ divider + Margin(1) left = 59 inner text cols
 *
 * Chat chrome fills rows 0-11 (12 rows), overlay fills rows 12-23 (12 rows).
 *
 * Colours use the same ANSI mappings as theme.rs dark():
 *   \x1b[32m  accent  (#39ff14)
 *   \x1b[37m  fg      (#e6e6e6)
 *   \x1b[90m  dim     (#adadad)
 *   \x1b[30m  sel_fg  (black)
 *   \x1b[42m  sel_bg  (accent)
 */

const RST = '\x1b[0m'
const ACC = '\x1b[32m'     // accent green #39ff14
const FG  = '\x1b[37m'     // fg white #e6e6e6
const DIM = '\x1b[90m'     // dim #adadad
const SEL_FG = '\x1b[30m'  // selection foreground black
const SEL_BG = '\x1b[42m'  // selection background accent
const INVERSE = SEL_FG + SEL_BG + '\x1b[1m'

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
  return text + ' '.repeat(Math.max(0, w - vis))
}

/** Fill a row with `ch` repeated `w` times. */
function bar(ch: string, w: number): string {
  return ch.repeat(w)
}

// ─── Sub-agent data ───────────────────────────────────────────────────

interface SubAgent {
  id: number
  name: string
  tag: string         // short status: "running", "done", "killed", "error"
  label: string
  statusLine: string  // right-pane accent status line
  transcript: string[]
}

const subagents: SubAgent[] = [
  {
    id: 1,
    name: 'explore',
    tag: 'running',
    label: 'find-auth-middleware',
    statusLine: 'running\u2026',
    transcript: [
      'searching src/mw/',
      'found: src/mw/auth.rs',
      'reading auth.rs...',
    ],
  },
  {
    id: 2,
    name: 'code-impl',
    tag: 'done',
    label: 'fix-connection-pool',
    statusLine: 'done \u00b7 fixed the race condition by changing Ordering::Relaxed to Acquire',
    transcript: [
      'Changed to Ordering::Acquire',
      'added empty pool check',
      'verified with 3 test runs',
    ],
  },
]

// ─── Screen: Sub-agents Panel ─────────────────────────────────────────
// Bordered popup above input, two-pane: left list + right detail.
//
// Layout per Rust source (subagents.rs):
//   Block::bordered() on outer rect  →  outer │ │ borders
//   cols = split horizontally: [18, Min(0)]
//   list_block = Block::new().borders(RIGHT) on cols[0]  →  right │ divider
//   list_inner = list_block.inner(cols[0])  →  17 text cols
//   right = cols[1].inner(Margin{horizontal:1})  →  59 text cols

function buildTaskScreen(
  rows: number,
  selectedIdx: number,
  opts: {
    mode?: string
  } = {},
): string {
  const W = 80
  const LIST_TEXT_W = 17     // left pane inner text width (list_inner.width)
  const RIGHT_TEXT_W = 59    // right pane inner text width after Margin(1)
  const CONTENT_ROWS = 10   // box content rows (boxH - 2)

  const mode = opts.mode ?? 'auto'

  // ── Input bar (3 rows: rule, input, rule) ──
  const inputBar: string[] = [
    DIM + bar('\u2500', W) + RST,
    padRight('  ' + ACC + '[$] /task\u2588' + RST, W),
    DIM + bar('\u2500', W) + RST,
  ]

  // ── Overlay lines (12 rows: top border + 10 content + bottom border) ──
  const overlayLines: string[] = []
  const title = ' sub-agents  Ctrl+X kill \u00b7 Ctrl+B background '
  overlayLines.push(
    DIM + '\u250c' + title + bar('\u2500', W - 2 - title.length) + '\u2510' + RST,
  )

  // Pre-build right pane lines
  const sel = subagents[selectedIdx]
  const rightLines: string[] = []
  rightLines.push(ACC + sel.statusLine + RST)
  const budget = CONTENT_ROWS - 1
  const tStart = Math.max(0, sel.transcript.length - budget)
  for (const tl of sel.transcript.slice(tStart)) {
    rightLines.push(DIM + tl + RST)
  }
  while (rightLines.length < CONTENT_ROWS) rightLines.push('')

  for (let i = 0; i < CONTENT_ROWS; i++) {
    let leftText: string
    if (i < subagents.length) {
      const sa = subagents[i]
      if (i === selectedIdx) {
        const label = `#${sa.id} ${sa.name} ${sa.tag} ${sa.label}`
        leftText = INVERSE + padRight(trunc(label, LIST_TEXT_W), LIST_TEXT_W) + RST
      } else {
        const idVis = `#${sa.id} `.length
        const restMax = LIST_TEXT_W - idVis
        leftText =
          ACC + `#${sa.id} ` + RST +
          trunc(FG + `${sa.name} ${sa.tag} ${sa.label}` + RST, restMax)
      }
    } else {
      leftText = ' '.repeat(LIST_TEXT_W)
    }
    const rightText = padRight(rightLines[i], RIGHT_TEXT_W)
    overlayLines.push(
      DIM + '\u2502' + RST +
      leftText +
      DIM + '\u2502' + RST +
      rightText +
      DIM + '\u2502' + RST,
    )
  }
  overlayLines.push(DIM + '\u2514' + bar('\u2500', W - 2) + '\u2518' + RST)

  // ── Chat chrome (fits above overlay + input bar) ──
  const lines: string[] = []
  const brandVis = 'koma 0.3.16'
  const modeVis = '\u25cf ' + mode
  const gap = Math.max(1, W - 2 - brandVis.length - modeVis.length)
  lines.push(padRight('  ' + DIM + 'koma' + RST + ' ' + ACC + '0.3.16' + RST, 2 + brandVis.length) +
    ' '.repeat(gap) + ACC + modeVis + RST)
  lines.push(DIM + bar('\u2500', W) + RST)
  lines.push(padRight('  ' + FG + '\u25cf find the auth middleware and fix the connection pool race condition' + RST, W))
  lines.push(padRight('  ' + FG + '\u25cf I\u2019ll investigate both issues. Let me start by exploring the auth middleware.' + RST, W))
  lines.push(padRight('    \u2699 ' + ACC + 'spawn explore \u2192 #1 find-auth-middleware' + RST, W))
  lines.push(padRight('    ' + DIM + '\u2192 #1 explore spawned' + RST, W))
  lines.push(padRight('  ' + FG + '\u25cf Meanwhile, let me examine the connection pool directly.' + RST, W))
  lines.push(padRight('    \u2699 ' + ACC + 'read src/pool.rs:42-89' + RST, W))
  lines.push(padRight('  ' + FG + '\u25cf Found the race: Ordering::Relaxed on the atomic counter.' + RST, W))

  // ── Compose full screen: chat + overlay above input bar ──
  const targetStart = rows - inputBar.length - overlayLines.length
  while (lines.length < targetStart) lines.push('')
  lines.push(...overlayLines)
  lines.push(...inputBar)

  while (lines.length < rows) lines.push('')
  return lines.slice(0, rows).map(l => padRight(trunc(l, W), W)).join('\n')
}

// ─── Screen: Running Agent (#1 explore, selected) ─────────────────────

function screenSubagentsRunning(rows = 24): string {
  return buildTaskScreen(rows, 0, {
    mode: 'auto',
  })
}

// ─── Screen: Completed Agent (#2 code-impl, selected) ────────────────

function screenSubagentsDone(rows = 24): string {
  return buildTaskScreen(rows, 1, {
    mode: 'auto',
  })
}

// ─── Tutorial Steps ───────────────────────────────────────────────────

import type { TutorialStep } from './first-run-tutorial'

/** Build the tutorial steps for a given terminal row count. */
export function getTaskSteps(rows = 24): TutorialStep[] {
  return [
    {
      title: 'Running Agent',
      narration:
        'Type /task to open the sub-agents panel \u2014 a bordered popup anchored above the input. ' +
        'The left pane lists all active sub-agents; the right pane shows the selected agent\u2019s ' +
        'live status and streaming transcript.',
      points: [
        'Each sub-agent shows its id, name, status tag, and label',
        'The selected agent (inverse highlight) streams live transcript updates',
        'Press Ctrl+X to kill a selected sub-agent, Ctrl+B to background it',
      ],
      screen: screenSubagentsRunning(rows),
    },
    {
      title: 'Completed Agents',
      narration:
        'Navigate to a completed sub-agent to review its result. The right pane shows the status ' +
        'line \u2014 a done agent displays its answer summary; a killed or error agent shows the ' +
        'reason. Press Enter on any row to open the full-screen viewer.',
      points: [
        '"done" agents show a summary of what they found or implemented',
        '"error" agents show the failure reason for debugging',
        'Press Enter on any agent to open the full-screen transcript viewer',
      ],
      screen: screenSubagentsDone(rows),
    },
  ]
}
