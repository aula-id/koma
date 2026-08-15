/**
 * Tutorial screens for /usage — cost and token dashboard.
 *
 * Step 1: Global dashboard — full-screen view with KPIs, hourly heatmap,
 *         top models table, and role split bar chart.
 * Step 2: Session view — after pressing Tab, showing session-specific
 *         totals, models used, and hourly heatmap.
 *
 * Layouts match src-agent/src/view/usage/mod.rs exactly.
 *
 * Full-screen dimensions (80-col terminal):
 *   Row 0:  header with tab/range/metric indicators
 *   Row 1:  dim rule
 *   Rows 2-22: content (21 rows)
 *   Row 23: footer bar
 *
 * Two-column layout (rows 7-18):
 *   Left column:  36 chars — hourly cost heatmap
 *   Gap:           2 chars
 *   Right column: 42 chars — top models table
 *
 * Colours use the same ANSI mappings as theme.rs dark():
 *   \x1b[32m  accent  (#39ff14)
 *   \x1b[37m  fg      (#e6e6e6)
 *   \x1b[90m  dim     (#adadad)
 *   \x1b[33m  warn    (#ffb43c)
 *   \x1b[36m  info    (#50c8ff)
 *   \x1b[92m  success (#00c853)
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
const BOLD_ACC = '\x1b[1;32m'

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

/** Pad a string to `w` visible characters (right-pad with spaces). */
function padRight(text: string, w: number): string {
  const vis = stripAnsi(text).length
  return text + ' '.repeat(Math.max(0, w - vis))
}

/** Repeat a character `w` times. */
function bar(ch: string, w: number): string {
  return ch.repeat(w)
}

// ─── Screen: Global Dashboard ────────────────────────────────────────
// Full-screen dashboard (80×24):
//   Row 0:      header with tab/range/metric
//   Row 1:      dim rule
//   Row 2:      KPI section header
//   Rows 3-5:   6 KPI values (2 per row)
//   Row 6:      blank
//   Row 7:      mid section headers (left: COST HOURLY, right: TOP MODELS)
//   Rows 8-18:  heatmap hours 07-17 (left 36) + model table (right 42)
//   Row 19:     legend (left) + blank (right)
//   Row 20:     blank
//   Row 21:     ROLE SPLIT section header
//   Row 22:     main + sub bar charts
//   Row 23:     footer bar

function screenGlobalDashboard(rows = 24): string {
  const W = 80
  const LEFT_COL = 36   // heatmap column width
  const RIGHT_COL = 42  // model table column width
  const GAP = 2         // gap between columns (= 80 - 36 - 42)
  const lines: string[] = []

  // ── Row 0: Header ───────────────────────────────────────────────
  const hdr = [
    '  ',
    BOLD_ACC + 'koma / usage ' + RST,
    DIM + '[tab: global] ' + RST,
    INVERSE + ' 1:today ' + RST,
    DIM + ' 2:week ' + RST,
    DIM + ' 3:year ' + RST,
    DIM + ' [m: cost]' + RST,
  ].join('')
  lines.push(hdr)

  // ── Row 1: Dim rule ─────────────────────────────────────────────
  lines.push(DIM + bar('\u2500', W) + RST)

  // ── Row 2: KPI section header ───────────────────────────────────
  lines.push('  ' + BOLD_ACC + 'KPI' + RST + ' ' + DIM + bar('\u2500', W - 7) + RST)

  // ── Rows 3-5: 6 KPI values (2 per row) ─────────────────────────
  const kpiPairs = [
    [
      { label: 'total',    value: '$1.47' },
      { label: 'in',       value: '1.2M' },
    ],
    [
      { label: 'cached',   value: '340.2k' },
      { label: 'out',      value: '89.3k' },
    ],
    [
      { label: 'calls',    value: '127' },
      { label: 'avg/call', value: '$0.0116' },
    ],
  ]
  for (const pair of kpiPairs) {
    const left = '    ' + DIM + pair[0].label.padEnd(10) + RST + ' ' + FG + pair[0].value + RST
    const right = DIM + pair[1].label.padEnd(10) + RST + ' ' + FG + pair[1].value + RST
    lines.push(padRight(left, 40) + padRight(right, 40))
  }

  // ── Row 6: Blank ────────────────────────────────────────────────
  lines.push('')

  // ── Row 7: Mid section headers (left + right) ──────────────────
  const leftHdr = '  ' + BOLD_ACC + 'COST (HOURLY)' + RST + ' ' + DIM + bar('\u2500', 20) + RST
  const rightHdr = '  ' + BOLD_ACC + 'TOP MODELS' + RST + ' ' + DIM + bar('\u2500', 28) + RST
  lines.push(padRight(leftHdr, LEFT_COL) + ' '.repeat(GAP) + padRight(rightHdr, RIGHT_COL))

  // ── Rows 8-18: Heatmap (left) + Model table (right) ───────────
  //
  // Heatmap: 11 hour rows (07-17)
  // Model table: 1 header + 3×2 data = 7 rows + 4 blank

  // Hourly cost data (proportional to $1.47 total)
  const hourCosts = [
    0,    0,    0,    0,    0,    0,     // 00-05
    0.01, 0.06, 0.12, 0.18, 0.15, 0.10,  // 06-11
    0.05, 0.11, 0.20, 0.17, 0.12, 0.07,  // 12-17
    0.03, 0.01, 0,    0,    0,    0,      // 18-23
  ]
  const maxCost = Math.max(...hourCosts)
  const maxBarLen = 27

  // Top models data
  const models = [
    { name: 'claude-3.5-sonnet', cost: '$0.89', calls: '67', pct: '61%', barLen: 24 },
    { name: 'gpt-4o',            cost: '$0.38', calls: '42', pct: '26%', barLen: 10 },
    { name: 'gpt-4o-mini',       cost: '$0.20', calls: '18', pct: '14%', barLen: 6 },
  ]

  // Build right column content (11 rows to match heatmap)
  const rightContent: string[] = []
  // Row 0: column headers
  rightContent.push(
    '    ' + DIM +
      'model'.padEnd(17) +
      'cost'.padStart(5) +
      '  ' +
      'calls'.padStart(5) +
      '  ' +
      '%'.padStart(4) +
      RST,
  )
  // Rows 1-6: model data + bar (3 models × 2 rows)
  for (const m of models) {
    rightContent.push(
      '    ' +
        FG + m.name.padEnd(17) + RST +
        FG + m.cost.padStart(5) + RST +
        '  ' +
        DIM + m.calls.padStart(5) + '  ' +
        m.pct.padStart(4) + RST,
    )
    rightContent.push('    ' + ACC + bar('\u2588', m.barLen) + RST)
  }
  // Pad to 11 rows
  while (rightContent.length < 11) rightContent.push('')

  // Rows 8-18: heatmap hours 07-17
  for (let hi = 0; hi < 11; hi++) {
    const h = hi + 7 // hours 07-17
    // Left column: hour label + proportional bar
    const hourStr = DIM + String(h).padStart(2, '0') + ' ' + RST
    const bLen = maxCost > 0 ? Math.round(hourCosts[h] * maxBarLen / maxCost) : 0
    let color = DIM
    if (hourCosts[h] >= 0.05) color = ACC
    if (h === 14) color = BOLD_ACC  // current hour highlighted
    const barStr = bLen > 0 ? color + bar('\u2588', bLen) + RST : ''

    const left = hourStr + barStr
    const right = rightContent[hi]

    lines.push(padRight(left, LEFT_COL) + ' '.repeat(GAP) + padRight(right, RIGHT_COL))
  }

  // ── Row 19: legend (left) + blank (right) ──────────────────────
  const legend = '    ' + DIM + 'intensity: \u2591 low  \u2592 med  \u2593 high  \u2588 peak' + RST
  lines.push(padRight(legend, LEFT_COL) + ' '.repeat(GAP) + padRight('', RIGHT_COL))

  // ── Row 20: Blank ───────────────────────────────────────────────
  lines.push('')

  // ── Row 21: ROLE SPLIT section header ───────────────────────────
  lines.push(
    '  ' + BOLD_ACC + 'ROLE SPLIT' + RST + ' ' + DIM + bar('\u2500', W - 16) + RST,
  )

  // ── Row 22: main + sub bar charts ───────────────────────────────
  const mainLabel = '  main  '
  const mainBar = ACC + bar('\u2588', 28) + RST
  const mainStats = '  79% 38c'
  const sep = '    '
  const subLabel = DIM + 'sub ' + RST
  const subBar = ACC + bar('\u2588', 8) + RST
  const subStats = DIM + ' 21%  9c' + RST
  lines.push(
    padRight(mainLabel + mainBar + mainStats, 40) +
    padRight(sep + subLabel + subBar + subStats, 40),
  )

  // ── Row 23: Footer (inverse bar) ────────────────────────────────
  const footerText = ' [Tab] view  [1\u20133] range  [m] metric  [Esc] exit '
  lines.push(INVERSE + padRight(footerText, W) + RST)

  return lines.slice(0, rows).map((l) => padRight(trunc(l, W), W)).join('\n')
}

// ─── Screen: Session View ────────────────────────────────────────────
// After pressing Tab — session-scoped KPI, models used, and hourly
// heatmap for a single session.
//
// Layout (80×24):
//   Row 0:      header [tab: session]
//   Row 1:      dim rule
//   Row 2:      SESSION KPI section header
//   Rows 3-8:   6 KPI values (two-column: left cost, right activity)
//   Row 9:      blank
//   Row 10:     MODELS USED section header
//   Row 11:     column headers
//   Rows 12-15: 2 models × 2 rows (data + bar)
//   Row 16:     blank
//   Row 17:     HOURLY HEATMAP section header
//   Rows 18-21: 4 heatmap rows (Thu-Sun)
//   Row 22:     legend
//   Row 23:     footer

function screenSessionView(rows = 24): string {
  const W = 80
  const lines: string[] = []

  // ── Row 0: Header ───────────────────────────────────────────────
  const hdr = [
    '  ',
    BOLD_ACC + 'koma / usage ' + RST,
    DIM + '[tab: session] ' + RST,
    INVERSE + ' 1:today ' + RST,
    DIM + ' 2:week ' + RST,
    DIM + ' 3:year ' + RST,
    DIM + ' [m: cost]' + RST,
  ].join('')
  lines.push(hdr)

  // ── Row 1: Dim rule ─────────────────────────────────────────────
  lines.push(DIM + bar('\u2500', W) + RST)

  // ── Row 2: SESSION KPI section header ───────────────────────────
  lines.push(
    '  ' + BOLD_ACC + 'SESSION KPI' + RST + ' ' + DIM + bar('\u2500', W - 17) + RST,
  )

  // ── Rows 3-8: KPI data (two-column: left cost metrics, right activity) ──
  const leftKpis = [
    { label: 'total',    value: '$3.42' },
    { label: 'in',       value: '340K' },
    { label: 'out',      value: '128K' },
    { label: 'cached',   value: '42K' },
    { label: 'calls',    value: '13' },
    { label: 'avg/call', value: '$0.26' },
  ]
  const rightKpis = [
    { label: 'duration', value: '2h 14m' },
    { label: 'messages', value: '18' },
    { label: 'tools',    value: '7' },
    { label: 'edits',    value: '3' },
    { label: 'files',    value: '5' },
  ]

  for (let i = 0; i < leftKpis.length; i++) {
    const left = leftKpis[i]
    const leftLabel = DIM + left.label.padEnd(10) + RST
    const leftValue = FG + left.value + RST
    let line = '    ' + leftLabel + ' ' + leftValue

    if (i < rightKpis.length) {
      const right = rightKpis[i]
      const rightLabel = DIM + right.label.padEnd(11) + RST
      const rightValue = FG + right.value + RST
      line += '        ' + rightLabel + ' ' + rightValue
    }

    lines.push(line)
  }

  // ── Row 9: Blank ────────────────────────────────────────────────
  lines.push('')

  // ── Row 10: MODELS USED section header ──────────────────────────
  lines.push(
    '  ' +
      BOLD_ACC + 'MODELS USED' + RST +
      ' ' + DIM + bar('\u2500', W - 17) + RST,
  )

  // ── Row 11: Column headers ──────────────────────────────────────
  lines.push(
    '    ' +
      DIM +
      'model'.padEnd(17) +
      'cost'.padStart(5) +
      '  ' +
      'calls'.padStart(5) +
      '  ' +
      '%'.padStart(4) +
      RST,
  )

  // ── Rows 12-15: Model data ─────────────────────────────────────
  const models = [
    {
      name: 'claude-3.5-sonnet',
      cost: '$2.18',
      calls: '8',
      pct: '64%',
      barLen: 26,
    },
    {
      name: 'gpt-4o-mini',
      cost: '$1.24',
      calls: '5',
      pct: '36%',
      barLen: 14,
    },
  ]

  for (const m of models) {
    // Data row
    const dataRow =
      '    ' +
        FG + m.name.padEnd(17) + RST +
        FG + m.cost.padStart(5) + RST +
        '  ' +
        DIM + m.calls.padStart(5) + '  ' +
        m.pct.padStart(4) + RST
    lines.push(dataRow)
    // Bar chart row
    lines.push('    ' + ACC + bar('\u2588', m.barLen) + RST)
  }

  // ── Row 16: Blank ───────────────────────────────────────────────
  lines.push('')

  // ── Row 17: HOURLY HEATMAP section header ───────────────────────
  lines.push(
    '  ' +
      BOLD_ACC + 'HOURLY HEATMAP' + RST +
      ' ' + DIM + bar('\u2500', W - 20) + RST,
  )

  // ── Rows 18-21: Heatmap (4 rows for session window) ────────────
  const heatmapData: number[][] = [
    //0  1  2  3  4  5  6  7  8  9 10 11 12 13 14 15 16 17 18 19 20 21 22 23
    [0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 2, 1, 0, 1, 3, 2, 1, 0, 0, 0, 0, 0, 0, 0], // Thu
    [0, 0, 0, 0, 0, 0, 0, 1, 2, 2, 1, 1, 0, 1, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0], // Fri
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 2, 1, 2, 3, 2, 1, 0, 0, 0, 0, 0, 0, 0], // Sat
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0], // Sun
  ]
  const dayLabels = ['Thu', 'Fri', 'Sat', 'Sun']

  const BLOCK = ['\u2591', '\u2592', '\u2593', '\u2588']

  for (let i = 0; i < heatmapData.length; i++) {
    const dayLabel = DIM + dayLabels[i] + ' ' + RST
    let heatRow = ''
    for (const v of heatmapData[i]) {
      const ch = BLOCK[Math.min(v, 3)]
      const color = v >= 2 ? ACC : DIM
      heatRow += color + ch + RST
    }
    lines.push('    ' + dayLabel + heatRow)
  }

  // ── Row 22: Legend ──────────────────────────────────────────────
  lines.push('    ' + DIM + '\u2591 low  \u2592 med  \u2593 high  \u2588 peak' + RST)

  // ── Row 23: Footer (inverse bar) ────────────────────────────────
  const footerText = ' [Tab] global  [1\u20133] range  [m] metric  [Esc] exit '
  lines.push(INVERSE + padRight(footerText, W) + RST)

  return lines.slice(0, rows).map((l) => padRight(trunc(l, W), W)).join('\n')
}

// ─── Tutorial Steps ───────────────────────────────────────────────────

import type { TutorialStep } from './first-run-tutorial'

export function getUsageSteps(rows = 24): TutorialStep[] {
  return [
    {
      title: 'Global Dashboard',
      narration:
        'The /usage command opens a full-screen cost and token dashboard. ' +
        'It shows total spend, input/output/cache breakdown, an hourly activity heatmap, ' +
        'top models by cost, and a role split between main and sub-agents.',
      points: [
        'KPI strip at the top shows cumulative totals across all sessions',
        'Hourly heatmap on the left highlights peak usage hours in accent green',
        'Top models table on the right ranks by cost with proportional bar charts',
        'Role split shows main-agent vs sub-agent spend at a glance',
      ],
      screen: screenGlobalDashboard(rows),
    },
    {
      title: 'Session View',
      narration:
        'Press Tab to switch to session view. This scopes every metric to the current ' +
        'session \u2014 showing which models were used, how many tools and edits ran, ' +
        'and the session-specific activity heatmap.',
      points: [
        'Session KPI includes activity metrics (messages, tools, edits, files)',
        'Model list shows only models called during this session',
        'Heatmap covers the session time window rather than the full week',
        'Press Tab again to return to the global dashboard',
      ],
      screen: screenSessionView(rows),
    },
  ]
}
