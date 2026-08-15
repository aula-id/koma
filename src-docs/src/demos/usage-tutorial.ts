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

import { RST, ACC, FG, DIM, INVERSE, BOLD_ACC, trunc, padRight, bar, commandEntryScreen } from './chat-chrome'

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
// After pressing Tab — session totals, models used, and hourly heatmap for
// one session. The lower-hour slice reflects how the live 24-row viewport
// clips the 24 hourly bars from the top.
//
// Layout (80×24):
//   Row 0:      header [tab: session]
//   Row 1:      dim rule
//   Row 2:      SESSION TOTALS section header
//   Rows 3-7:   in, cached, out, cost, calls
//   Row 8:      blank
//   Row 9:      MODELS USED (42) + gap (2) + HOURLY HEATMAP (36)
//   Rows 10-22: model rows + clipped hourly bars and legend
//   Row 23:     footer

function screenSessionView(rows = 24): string {
  const W = 80
  const MODELS_W = 42
  const HEATMAP_W = 36
  const GAP = 2
  const lines: string[] = []

  // The session header deliberately omits global-only range and metric controls.
  lines.push(' ' + BOLD_ACC + 'koma / usage  ' + RST + DIM + '[tab: session]' + RST)
  lines.push(DIM + bar('─', W) + RST)

  // The live view has five session totals and no activity-derived metrics.
  lines.push(BOLD_ACC + 'SESSION TOTALS' + RST + ' ' + DIM + bar('─', W - 15) + RST)
  const totals = [
    { label: 'in', value: '340K' },
    { label: 'cached', value: '42K' },
    { label: 'out', value: '128K' },
    { label: 'cost', value: '$3.42' },
    { label: 'calls', value: '13' },
  ]
  for (const total of totals) {
    lines.push(' ' + DIM + total.label.padEnd(10) + RST + FG + total.value + RST)
  }
  lines.push('')

  // At 80 columns draw_session() allocates 42 model columns, a two-cell gap,
  // and 36 heatmap columns. Both section labels occupy the same row.
  const modelsHeading = BOLD_ACC + 'MODELS USED' + RST + ' ' + DIM + bar('─', MODELS_W - 12) + RST
  const heatmapHeading = BOLD_ACC + 'HOURLY HEATMAP' + RST + ' ' + DIM + bar('─', HEATMAP_W - 15) + RST
  lines.push(padRight(modelsHeading, MODELS_W) + ' '.repeat(GAP) + padRight(heatmapHeading, HEATMAP_W))

  const modelRows = [
    DIM + 'model'.padEnd(11) + '  ' + 'cost'.padStart(7) + '  ' + 'tokens'.padStart(6) + '  ' + 'calls'.padStart(5) + RST,
    FG + 'claude-3.5'.padEnd(11) + RST + '  ' + FG + '$2.18'.padStart(7) + RST + '  ' + DIM + '320K'.padStart(6) + RST + '  ' + DIM + '8'.padStart(5) + RST + ' ' + ACC + '██████' + RST,
    FG + 'gpt-4o-mini'.padEnd(11) + RST + '  ' + FG + '$1.24'.padStart(7) + RST + '  ' + DIM + '148K'.padStart(6) + RST + '  ' + DIM + '5'.padStart(5) + RST + ' ' + ACC + '██▍' + RST,
  ]

  // The renderer creates up to 24 hourly rows plus a legend, but a 24-row
  // terminal clips the middle panel. Show the fitting lower-hour portion,
  // rather than inventing a multi-day grid.
  const hourly = [
    { hour: '12', fill: 4 }, { hour: '13', fill: 8 }, { hour: '14', fill: 16 },
    { hour: '15', fill: 22 }, { hour: '16', fill: 18 }, { hour: '17', fill: 12 },
    { hour: '18', fill: 8 }, { hour: '19', fill: 5 }, { hour: '20', fill: 3 },
    { hour: '21', fill: 1 }, { hour: '22', fill: 0 }, { hour: '23', fill: 0 },
  ]
  const heatmapRows = hourly.map(({ hour, fill }) =>
    DIM + hour + RST + ACC + bar('█', fill) + DIM + bar('█', 34 - fill) + RST,
  )

  // Thirteen rows remain before the footer: table header/data on the left,
  // twelve hourly bars and the legend on the right.
  for (let i = 0; i < 13; i++) {
    const left = modelRows[i] ?? ''
    const right = i < heatmapRows.length
      ? heatmapRows[i]
      : DIM + '     cheap ' + RST + ACC + '█████' + RST + DIM + ' expensive' + RST
    lines.push(padRight(left, MODELS_W) + ' '.repeat(GAP) + padRight(right, HEATMAP_W))
  }

  // The real footer is dim text on the normal background, with a one-cell margin.
  lines.push(' ' + DIM + '[Tab] view  [1-3] range  [m] metric  [Esc] exit' + RST)

  return lines.slice(0, rows).map((l) => padRight(trunc(l, W), W)).join('\n')
}

// ─── Tutorial Steps ───────────────────────────────────────────────────

import type { TutorialStep } from './first-run-tutorial'

export function getUsageSteps(rows = 24): TutorialStep[] {
  return [
    {
      title: 'Type /usage',
      narration: 'From normal chat, type /usage in the composer and press Enter to open the cost and token dashboard.',
      screen: commandEntryScreen(rows, '/usage'),
    },
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
        'Press Tab to switch to session view. This scopes token totals, cost, calls, ' +
        'model usage, and the hourly spend heatmap to the current session.',
      points: [
        'Session totals show input, cached, output, cost, and call count',
        'Model rows include cost, token volume, calls, and an inline token bar',
        'The 24-row terminal shows the lower hours and heatmap legend that fit',
        'Press Tab again to return to the global dashboard',
      ],
      screen: screenSessionView(rows),
    },
  ]
}
