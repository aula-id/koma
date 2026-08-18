/**
 * Tutorial screens for `/bash` — the background jobs panel.
 *
 * The bash panel is a bordered popup anchored above the input box,
 * drawn on top of the chat transcript. Two-pane layout inside the box:
 *   LEFT  — narrow list (18 cols) of jobs with id, status, elapsed time
 *   RIGHT — wide detail pane with command, status line, output tail
 *
 * Layouts match src-agent/src/view/bash/mod.rs exactly.
 *
 * Overlay dimensions (80-col terminal, 18-row box):
 *   Outer box: 80 chars including │ borders
 *   Left pane: 18 cols with │ RIGHT border (17 inner text cols)
 *   Right pane: 60 cols after │ divider + Margin(1) left = 59 inner text cols
 *
 * Chat chrome fills rows 0-5 (6 rows), overlay fills rows 6-23 (18 rows).
 *
 * Colours use the same ANSI mappings as theme.rs dark():
 *   \x1b[32m  accent  (#39ff14)
 *   \x1b[37m  fg      (#e6e6e6)
 *   \x1b[90m  dim     (#adadad)
 *   \x1b[30m  sel_fg  (black)
 *   \x1b[42m  sel_bg  (accent)
 */

import { RST, ACC, FG, DIM, INVERSE, trunc, padRight, bar, chatInput, commandEntryScreen } from './chat-chrome'

// ─── Bash job data ────────────────────────────────────────────────────

interface BashJob {
  id: number
  command: string
  status: string     // "running" | "done" | "error" | "killed"
  running: boolean
  elapsedSecs: number
  elapsed: string
  outputTail: string
}

const jobs: BashJob[] = [
  {
    id: 1,
    command: 'cargo test --release 2>&1',
    status: 'running',
    running: true,
    elapsedSecs: 3,
    elapsed: '3s',
    outputTail:
      'test test_pool_checkout ... ok\n' +
      'test test_pool_release ... ok\n' +
      'test test_concurrent_checkout ... running',
  },
  {
    id: 2,
    command: 'npm run build',
    status: 'done',
    running: false,
    elapsedSecs: 8,
    elapsed: '8s',
    outputTail:
      '> project@1.0.0 build\n' +
      '> tsc && vite build\n' +
      'built in 7.2s',
  },
  {
    id: 3,
    command: 'pytest tests/ -v',
    status: 'done',
    running: false,
    elapsedSecs: 12,
    elapsed: '12s',
    outputTail:
      'tests/test_auth.py::test_validate PASSED\n' +
      'tests/test_pool.py::test_checkout PASSED\n' +
      '18 passed in 11.4s',
  },
]

// ─── Screen: Background Jobs Panel ────────────────────────────────────
// Bordered popup above input, two-pane: left list + right detail.
//
// Layout per Rust source (bash/mod.rs):
//   Block::bordered() on outer rect  →  outer │ │ borders
//   cols = split horizontally: [18, Min(0)]
//   list_block = Block::new().borders(RIGHT) on cols[0]  →  right │ divider
//   list_inner = list_block.inner(cols[0])  →  17 text cols
//   right = cols[1].inner(Margin{horizontal:1})  →  59 text cols
//   Right pane: detail_lines() renders independently per selected job
//
// Left pane (job_row):
//   Selected:   "› " (2 chars, inverse) + label (pad label_w) + " " + elapsed (dim)
//   Non-selected: "  " (2 chars, dim) + label (pad label_w, accent if running else dim) + " " + elapsed (dim)
//   label_w = width - 2 - (elapsed_vis + 1)
//   label = "bash-{id}  {status}"

function buildBashScreen(
  rows: number,
  selectedIdx: number,
): string {
  const W = 80
  const LIST_TEXT_W = 17     // left pane inner text width (list_inner.width)
  const RIGHT_TEXT_W = 59    // right pane inner text width after Margin(1)
  const CONTENT_ROWS = 14   // box content rows (16-row box minus borders)
  const CHAT_ROWS = 6       // rows 0-5: chat chrome

  const lines: string[] = []
  const inputBar = chatInput('/bash')

  // ── Chat chrome (rows 0-11) ──
  // Row 0: header with version + mode indicator (80 visible chars)
  const brandVis = 'koma 0.3.16'  // 11 chars
  const modeVis = '\u25cf auto'    // 6 chars
  const gap = Math.max(1, W - 2 - brandVis.length - modeVis.length)
  lines.push(padRight('  ' + DIM + 'koma' + RST + ' ' + ACC + '0.3.16' + RST, 2 + brandVis.length) +
    ' '.repeat(gap) + ACC + modeVis + RST)
  // Row 1: dim rule
  lines.push(DIM + bar('\u2500', W) + RST)
  // Row 2: user message
  lines.push(padRight('  ' + FG + '\u25cf run the tests and build the frontend' + RST, W))
  // Row 3: tool call 1
  lines.push(padRight('    \u2699 ' + ACC + 'cargo test --release 2>&1' + RST, W))
  // Row 4: background job 1
  lines.push(padRight('    ' + DIM + 'background job started: bash-1' + RST, W))
  // Row 5: tool call 2
  lines.push(padRight('    \u2699 ' + ACC + 'npm run build' + RST, W))
  // Row 6: background job 2
  lines.push(padRight('    ' + DIM + 'background job started: bash-2' + RST, W))
  // Row 7: tool call 3
  lines.push(padRight('    \u2699 ' + ACC + 'pytest tests/ -v' + RST, W))
  // Row 8: background job 3
  lines.push(padRight('    ' + DIM + 'background job started: bash-3' + RST, W))
  // Row 9: assistant
  lines.push(padRight('  ' + FG + '\u25cf 3 background jobs running. Open /bash to monitor.' + RST, W))
  // Row 10: completion notice
  lines.push(padRight('    ' + DIM + '\u2713 bash-3 completed (12s)' + RST, W))
  // Row 11: assistant
  lines.push(padRight('  ' + FG + '\u25cf Pytest done \u2014 18 tests passed. Waiting on cargo + npm.' + RST, W))

  // The overlay covers older transcript rows while preserving the composer.
  lines.length = Math.min(lines.length, rows - inputBar.length - (CONTENT_ROWS + 2))

  // ── Overlay: title border (row 12) ──
  const title = ' bash '
  lines.push(
    DIM + '\u250c' + title + bar('\u2500', W - 2 - title.length) + '\u2510' + RST,
  )

  // ── Overlay: content rows (rows 13-22) ──
  //
  // LEFT PANE — one row per job:
  //   See job_row() in bash/mod.rs: marker (2) + label (pad) + " " + elapsed
  //   label_w = LIST_TEXT_W - 2 - (elapsed.length + 1)
  //   label = "bash-{id}  {status}"
  //
  // RIGHT PANE — independent of left-pane row index:
  //   detail_lines() from bash/mod.rs:
  //     line 0: "$ " (dim) + command (accent)
  //     line 1: "status: " (dim) + status (accent/fg) + "   ·   " + elapsed (dim)
  //     line 2: blank
  //     lines 3+: output lines (fg) or "(no output yet)" (dim)

  // Pre-build right pane lines for the selected job
  const j = jobs[selectedIdx]
  const rightLines: string[] = []
  // Line 0: $ command
  rightLines.push(DIM + '$ ' + RST + ACC + j.command + RST)
  // Line 1: status: ...   ·   Ns
  const statusStyle = j.running ? ACC : FG
  rightLines.push(
    DIM + 'status: ' + RST + statusStyle + j.status + RST +
    DIM + '   \u00b7   ' + j.elapsed + RST,
  )
  // Line 2: blank
  rightLines.push('')
  // Lines 3+: output tail
  const outLines = j.outputTail.split('\n')
  if (outLines.length === 0 || (outLines.length === 1 && outLines[0].trim() === '')) {
    rightLines.push(DIM + '(no output yet)' + RST)
  } else {
    for (const ol of outLines) {
      rightLines.push(FG + ol + RST)
    }
  }
  // Pad right lines to CONTENT_ROWS
  while (rightLines.length < CONTENT_ROWS) rightLines.push('')

  for (let i = 0; i < CONTENT_ROWS; i++) {
    // Left pane
    let leftText: string
    if (i < jobs.length) {
      const job = jobs[i]
      const labelW = LIST_TEXT_W - 2 - (job.elapsed.length + 1)
      const label = `bash-${job.id}  ${job.status}`
      const paddedLabel = padRight(trunc(label, labelW), labelW)

      if (i === selectedIdx) {
        // Selected: "› " inverse + label inverse + " " + elapsed dim
        leftText =
          INVERSE + '\u203a ' + paddedLabel + ' ' + RST +
          DIM + job.elapsed + RST
      } else {
        // Non-selected: "  " dim + label (accent if running, dim) + " " + elapsed dim
        const nameStyle = job.running ? ACC : DIM
        leftText =
          DIM + '  ' + RST +
          nameStyle + paddedLabel + ' ' + RST +
          DIM + job.elapsed + RST
      }
    } else {
      leftText = ' '.repeat(LIST_TEXT_W)
    }

    // Right pane — padded to 59 visible chars
    const rightText = padRight(rightLines[i], RIGHT_TEXT_W)

    // Full row: │ left │ right │  (padded to 80 visible chars)
    lines.push(
      DIM + '\u2502' + RST +
      leftText +
      DIM + '\u2502' + RST +
      rightText +
      DIM + '\u2502' + RST,
    )
  }

  // ── Overlay: bottom border ──
  lines.push(DIM + '\u2514' + bar('\u2500', W - 2) + '\u2518' + RST)
  lines.push(...inputBar)

  // Ensure exactly `rows` lines, each padded/truncated to exactly 80 visible chars
  while (lines.length < rows) lines.push('')
  return lines.slice(0, rows).map(l => padRight(l, W)).join('\n')
}

// ─── Screen: Running Job (bash-1 selected) ────────────────────────────

function screenBashRunning(rows = 24): string {
  return buildBashScreen(rows, 0)
}

// ─── Screen: Completed Job (bash-2 selected) ──────────────────────────

function screenBashDone(rows = 24): string {
  return buildBashScreen(rows, 1)
}

// ─── Tutorial Steps ───────────────────────────────────────────────────

import type { TutorialStep } from './first-run-tutorial'

/** Build the tutorial steps for a given terminal row count. */
export function getBashSteps(rows = 24): TutorialStep[] {
  return [
    {
      title: 'Type /bash',
      narration: 'From normal chat, type /bash in the composer and press Enter to open the read-only background jobs panel.',
      screen: commandEntryScreen(rows, '/bash'),
    },
    {
      title: 'Running Job',
      narration:
        'Type /bash to open the background jobs panel \u2014 a bordered popup anchored above the ' +
        'input. The left pane lists all active and completed jobs; the right pane shows the ' +
        'selected job\u2019s command, status, and streaming output.',
      points: [
        'Running jobs show accent color; elapsed time updates live',
        'The right pane displays the command, status line, and output tail',
        'Each job is numbered (bash-1, bash-2) for easy reference',
      ],
      screen: screenBashRunning(rows),
    },
    {
      title: 'Completed Jobs',
      narration:
        'Navigate to a completed job to review its output. The detail pane shows the full command, ' +
        'final status with elapsed time, and the complete output tail. This lets you review results ' +
        'without leaving the chat.',
      points: [
        '"done" jobs show output in accent-free text for easy reading',
        '"error" jobs display the error output for quick debugging',
        'Run multiple background tasks in parallel and review them here',
      ],
      screen: screenBashDone(rows),
    },
  ]
}
