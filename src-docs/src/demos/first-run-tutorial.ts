/**
 * Tutorial step data for the "first run" flow.
 *
 * Each screen is a COMPLETE ANSI-escape rendering of a real TUI state,
 * written as a single string to xterm.js. Layouts match the actual Rust
 * view code (onboard.rs, loading.rs, chat/mod.rs, input.rs, status.rs,
 * header.rs) — left-aligned within a centred 64-char block for onboard,
 * full-width with horizontal padding for chat.
 *
 * Colours use the same ANSI mappings as theme.rs dark():
 *   \x1b[32m  accent  (#39ff14)
 *   \x1b[37m  fg      (#e6e6e6)
 *   \x1b[90m  dim     (#adadad)
 *   \x1b[33m  warn    (#ffb43c)
 *   \x1b[36m  info    (#50c8ff)
 *   \x1b[32m  success (#00c853) — same ANSI code as accent in dark theme
 */

import { RST, ACC, FG, DIM, WARN, BOLD_ACC, stripAnsi, trunc, padRight, rightAlign, bar } from './chat-chrome'

// ─── Screen: Onboarding Chooser ───────────────────────────────────────
// Matches onboard.rs::draw — 64-char block centered at col 8 in 80-col terminal.

function screenChooser(rows = 24): string {
  const W = 80
  const BLOCK_W = 64
  const BX = Math.floor((W - BLOCK_W) / 2) // col 8
  const INDENT = ' '.repeat(BX + 2)         // col 10

  const lines: string[] = []

  // Top spacer: ~25% of rows
  const topSpacer = Math.max(1, Math.floor(rows * 0.25))
  const midSpacer = Math.max(1, Math.floor(rows * 0.08))
  for (let i = 0; i < topSpacer; i++) lines.push('')

  // Title: "koma" in accent, 2-space indent within block
  lines.push(INDENT + ACC + 'koma' + RST)
  lines.push('')
  lines.push('')

  // Question
  lines.push(INDENT + DIM + 'how do you want to connect?' + RST)
  lines.push('')

  // Options — matches onboard.rs CHOICES + LABEL_W=14
  const OPTS = [
    { label: 'koma free', desc: 'start now, no key - free models hosted by koma', sel: true },
    { label: 'provider',  desc: 'sign in to a provider account',                  sel: false },
    { label: 'custom',    desc: 'your own endpoint + API key',                    sel: false },
  ]
  for (const opt of OPTS) {
    const prefix = opt.sel ? ACC + '> ' : '  '
    const labelColor = opt.sel ? ACC : FG
    const rawLabel = opt.label.padEnd(14)
    lines.push(
      INDENT + prefix + labelColor + rawLabel + RST + DIM + opt.desc + RST,
    )
  }

  lines.push('')
  lines.push('')

  // Callout box
  const BORDER_DASHES = BLOCK_W - 2
  const CONTENT_W = BLOCK_W - 4
  const bxPad = ' '.repeat(BX)
  const c1 = 'you can change this anytime in /settings'
  const c2 = 'or type /free to switch to the free tier later'

  lines.push(bxPad + WARN + '┌' + bar('─', BORDER_DASHES) + '┐' + RST)
  lines.push(bxPad + WARN + '│ ' + c1.padEnd(CONTENT_W) + ' │' + RST)
  lines.push(bxPad + WARN + '│ ' + c2.padEnd(CONTENT_W) + ' │' + RST)
  lines.push(bxPad + WARN + '└' + bar('─', BORDER_DASHES) + '┘' + RST)

  for (let i = 0; i < midSpacer; i++) lines.push('')

  // Footer key hints
  lines.push(INDENT + DIM + 'up/down move \u00b7 enter select \u00b7 q quit' + RST)

  // Pad/truncate to target rows
  while (lines.length < rows) lines.push('')
  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

// ─── Screen: Loading Splash ───────────────────────────────────────────
// Matches loading.rs::draw — centered, braille spinner on "indexing workspace".

function screenLoading(rows = 24): string {
  const W = 80
  const lines: string[] = []

  // Top spacer: ~30% of rows
  const topSpacer = Math.max(1, Math.floor(rows * 0.30))
  const botSpacer = Math.max(1, Math.floor(rows * 0.30))
  for (let i = 0; i < topSpacer; i++) lines.push('')

  // Title: "koma" centred in accent
  const title = ACC + 'koma' + RST
  const titlePad = Math.floor((W - 4) / 2)
  lines.push(' '.repeat(titlePad) + title)

  lines.push('')

  // Steps — two lines, centred
  const spinner = ACC + '\u{2599}' + RST
  const done = ACC + '\u{25cf}' + RST
  const step1 = spinner + '  ' + FG + 'indexing workspace' + RST
  const step2 = done + '  ' + FG + 'reading project docs' + RST + '  ' + DIM + '(14 files)' + RST

  const s1Pad = Math.floor((W - stripAnsi(step1).length) / 2)
  const s2Pad = Math.floor((W - stripAnsi(step2).length) / 2)
  lines.push(' '.repeat(s1Pad) + step1)
  lines.push(' '.repeat(s2Pad) + step2)

  // Flexible spacer
  for (let i = 0; i < botSpacer; i++) lines.push('')

  // Footer
  const footer = DIM + 'warming up \u00b7 1.4s   \u00b7   esc to skip' + RST
  const fPad = Math.floor((W - stripAnsi(footer).length) / 2)
  lines.push(' '.repeat(fPad) + footer)

  // Pad/truncate to target rows
  while (lines.length < rows) lines.push('')
  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

// ─── Screen: Chat Interface ───────────────────────────────────────────
// Matches chat/mod.rs layout: header(2) | transcript | model(1) | input(3) | status(1)
// Header: Borders::BOTTOM, Padding::horizontal(2)
// Input: Borders::TOP|BOTTOM, Padding::horizontal(2)

function screenChatWelcome(rows = 24): string {
  const W = 80
  const PAD = '  ' // horizontal padding 2
  const MODEL = 'claude-sonnet-4-20250514'
  const lines: string[] = []

  // ── Header (2 rows) ──
  const version = '0.3.16'
  const brand = DIM + 'koma' + RST + ' '
  const verStr = ACC + version + RST
  const modeStr = ACC + '\u{25cf} normal' + RST
  const headerInnerW = W - 6
  const leftPart = 'koma ' + version
  const rightPart = '\u{25cf} normal'
  const gap = Math.max(1, headerInnerW - leftPart.length - rightPart.length)
  lines.push(PAD + brand + verStr + ' '.repeat(gap) + modeStr)
  lines.push(DIM + bar('─', W) + RST)

  // ── Transcript ──
  lines.push('')
  lines.push(PAD + FG + '\u{25cf}' + RST + ' ' + FG + 'welcome! i\'m koma, your coding agent.' + RST)
  lines.push(PAD + '  ' + DIM + 'i read your code, plan changes, edit files,' + RST)
  lines.push(PAD + '  ' + DIM + 'run commands, and verify everything works.' + RST)
  lines.push('')
  lines.push(PAD + FG + '\u{25cf}' + RST + ' ' + FG + 'try typing a task below \u2014 i\'ll get to work.' + RST)
  lines.push('')

  // Fill transcript to leave room for: fill + model(1) + input(3) + status(1)
  const reservedBelow = 1 + 3 + 1 // model + input + status
  const contentSoFar = lines.length
  const fillTarget = rows - reservedBelow
  while (lines.length < fillTarget) lines.push('')

  // ── Model name row — right-aligned with 2-char margin each side ──
  const modelAreaW = W - 4 // inner width after horizontal margin
  lines.push(PAD + rightAlign(DIM + MODEL + RST, modelAreaW) + PAD)

  // ── Input box (3 rows) ──
  lines.push(DIM + bar('─', W) + RST)
  lines.push(PAD + ACC + '[$] ' + RST + ACC + '\u{2588}' + RST)
  lines.push(DIM + bar('─', W) + RST)

  // ── Status bar ──
  const statusText = 'ready'
  const rightPart2 = '\u{2191}2.1k \u{2193}0.8k  $0.0032 [!]'
  const sGap = Math.max(1, W - 4 - statusText.length - rightPart2.length)
  lines.push(PAD + DIM + statusText + RST + ' '.repeat(sGap) + ACC + '\u{2191}2.1k \u{2193}0.8k' + RST + '  ' + DIM + '$0.0032 [!]' + RST)

  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

// ─── Screen: User Types a Message ─────────────────────────────────────
// Same chat layout but with user text in the input and a user message band.

function screenUserMessage(rows = 24): string {
  const W = 80
  const PAD = '  '
  const MODEL = 'claude-sonnet-4-20250514'
  const lines: string[] = []

  // Header
  const brand = DIM + 'koma' + RST + ' '
  const verStr = ACC + '0.3.16' + RST
  const modeStr = ACC + '\u{25cf} normal' + RST
  const headerInnerW = W - 6
  const gap = Math.max(1, headerInnerW - 'koma 0.3.16'.length - '\u{25cf} normal'.length)
  lines.push(PAD + brand + verStr + ' '.repeat(gap) + modeStr)
  lines.push(DIM + bar('─', W) + RST)

  // Transcript — user message band + assistant response
  lines.push('')
  lines.push(ACC + '\u{258c}' + RST + ' ' + FG + 'fix the race condition in the connection pool' + RST)
  lines.push('')
  lines.push(PAD + FG + '\u{25cf}' + RST + ' ' + DIM + 'let me look at the connection pool code...' + RST)
  lines.push('')
  lines.push(PAD + '  ' + DIM + '\u{258c} read src/db/pool.rs' + RST)
  lines.push(PAD + '  ' + DIM + '\u{258c} read src/db/mod.rs' + RST)
  lines.push('')

  // Fill transcript
  const reservedBelow = 1 + 3 + 1
  const fillTarget = rows - reservedBelow
  while (lines.length < fillTarget) lines.push('')

  // Model — right-aligned
  const modelAreaW = W - 4
  lines.push(PAD + rightAlign(DIM + MODEL + RST, modelAreaW) + PAD)

  // Input
  lines.push(DIM + bar('─', W) + RST)
  lines.push(PAD + ACC + '[$] ' + RST + FG + 'fix the race condition in the connection pool' + ACC + '\u{2588}' + RST)
  lines.push(DIM + bar('─', W) + RST)

  // Status
  const rightPart2 = '\u{2191}2.1k \u{2193}0.8k  $0.0032 [!]'
  const sGap = Math.max(1, W - 4 - 'thinking'.length - rightPart2.length)
  lines.push(PAD + DIM + 'thinking' + RST + ' \u00b7 3s' + ' '.repeat(sGap) + ACC + '\u{2191}2.1k \u{2193}0.8k' + RST + '  ' + DIM + '$0.0032 [!]' + RST)

  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

// ─── Screen: Streaming Response ───────────────────────────────────────

function screenStreamingResponse(rows = 24): string {
  const W = 80
  const PAD = '  '
  const MODEL = 'claude-sonnet-4-20250514'
  const lines: string[] = []

  // Header
  const brand = DIM + 'koma' + RST + ' '
  const verStr = ACC + '0.3.16' + RST
  const headerInnerW = W - 6
  const gap = Math.max(1, headerInnerW - 'koma 0.3.16'.length - '\u{25cf} normal'.length)
  lines.push(PAD + brand + verStr + ' '.repeat(gap) + ACC + '\u{25cf} auto' + RST)
  lines.push(DIM + bar('─', W) + RST)

  // Transcript
  lines.push('')
  lines.push(ACC + '\u{258c}' + RST + ' ' + FG + 'fix the race condition in the connection pool' + RST)
  lines.push('')
  lines.push(PAD + FG + '\u{25cf}' + RST + ' ' + FG + 'found it \u2014 the check-out path reads `active`' + RST)
  lines.push(PAD + '  ' + FG + 'without holding the mutex. here\'s the fix:' + RST)
  lines.push('')
  // Tool call result — code block
  lines.push(PAD + DIM + '\u{250c}\u{2500}' + ' src/db/pool.rs ' + bar('\u{2500}', 42) + '\u{2510}' + RST)
  lines.push(PAD + DIM + '\u{2502} ' + RST + FG + '- let count = self.active.load(Ordering::Relaxed);' + RST)
  lines.push(PAD + DIM + '\u{2502} ' + RST + ACC + '+ let count = self.active.load(Ordering::Acquire);' + RST)
  lines.push(PAD + DIM + '\u{2514}' + bar('\u{2500}', 58) + '\u{2518}' + RST)
  lines.push('')

  // Fill transcript
  const reservedBelow = 1 + 3 + 1
  const fillTarget = rows - reservedBelow
  while (lines.length < fillTarget) lines.push('')

  // Model — right-aligned
  const modelAreaW = W - 4
  lines.push(PAD + rightAlign(DIM + MODEL + RST, modelAreaW) + PAD)

  // Input
  lines.push(DIM + bar('─', W) + RST)
  lines.push(PAD + ACC + '[$] ' + RST + ACC + '\u{2588}' + RST)
  lines.push(DIM + bar('─', W) + RST)

  // Status — running
  const rightPart = '\u{2191}4.8k \u{2193}1.2k  $0.0089 [!]'
  const sGap = Math.max(1, W - 4 - 'thinking \u00b7 12s'.length - rightPart.length)
  lines.push(PAD + DIM + 'thinking' + RST + ' \u00b7 12s' + ' '.repeat(sGap) + ACC + '\u{2191}4.8k \u{2193}1.2k' + RST + '  ' + DIM + '$0.0089 [!]' + RST)

  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

// ─── Tutorial Steps ───────────────────────────────────────────────────

export interface TutorialStep {
  title: string
  narration: string
  /** Optional bullet points below the narration */
  points?: string[]
  /** The complete ANSI screen to render in xterm.js */
  screen: string
}

/** Build the tutorial steps for a given terminal row count. */
export function getFirstRunSteps(rows = 24): TutorialStep[] {
  return [
    {
      title: 'First Launch',
      narration:
        'When you launch koma for the very first time, you see the connection chooser \u2014 a simple three-way pick of how to connect.',
      points: [
        'No credentials or accounts are needed yet',
        'Three options: free tier, provider sign-in, or custom endpoint',
        'Your choice can be changed later in /settings',
      ],
      screen: screenChooser(rows),
    },
    {
      title: 'Zero-Config Start',
      narration:
        'Select "koma free" to start instantly. Free models are hosted by koma \u2014 no API key, no sign-up, no configuration. Just pick it and go.',
      points: [
        'Free tier includes capable models for everyday coding tasks',
        'No account or credit card required',
        'Switch to a paid provider anytime for more powerful models',
      ],
      screen: screenChooser(rows),
    },
    {
      title: 'Warming Up',
      narration:
        'After choosing your connection, koma warms up your workspace. It indexes your files and reads project docs so it understands your codebase from the first message.',
      points: [
        'Indexing is fast \u2014 usually under 2 seconds',
        'Press Esc to skip if you want to start immediately',
        'koma re-indexes automatically when files change',
      ],
      screen: screenLoading(rows),
    },
    {
      title: 'The Chat Interface',
      narration:
        'You land in the main chat \u2014 the primary interface. The header shows version and mode. The transcript displays the conversation. The input box with the [$] prompt is where you type tasks.',
      points: [
        'The header shows your version and current mode (normal, auto, plan)',
        'The input prompt is [$] \u2014 type any task or question',
        'The status bar shows token usage and cost in real time',
      ],
      screen: screenChatWelcome(rows),
    },
    {
      title: 'Getting Work Done',
      narration:
        'Type a task and koma gets to work. It reads your code, plans changes, edits files, runs tests, and verifies everything works \u2014 all from the chat.',
      points: [
        'Tool calls show which files are being read and edited',
        'Code diffs appear inline with added/removed lines',
        'The status bar shows what koma is doing and how long it\'s been working',
      ],
      screen: screenUserMessage(rows),
    },
    {
      title: 'In Action',
      narration:
        'As koma works, you see real-time progress. Tool calls stream in, code edits appear with diff markers, and the mode indicator shows when koma is working autonomously.',
      points: [
        'Auto mode lets koma work without approval for each tool call',
        'Tool results show file reads, edits, and command output',
        'Token usage updates live so you always know the cost',
      ],
      screen: screenStreamingResponse(rows),
    },
  ]
}
