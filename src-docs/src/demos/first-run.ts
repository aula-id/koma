/**
 * "koma first run" TUI script — exact visual recreation of the real
 * onboarding flow: chooser → select "koma free" → chat screen.
 *
 * ANSI escape codes:
 *   \x1b[2J     = clear screen
 *   \x1b[H      = cursor home
 *   \x1b[0m      = reset attributes
 *   \x1b[32m     = green (accent)
 *   \x1b[37m     = white (fg)
 *   \x1b[90m     = bright black (dim)
 *   \x1b[33m     = yellow (warn)
 *   \x1b[36m     = cyan (info)
 *   \x1b[1;32m   = bold green
 *   \x1b[7m      = reverse video (for selection)
 */

import { CLR, RST, ACC, FG, DIM, WARN, BOLD_ACC, REVERSE, padRight } from './chat-chrome'

// helper: center-pad a string in cols
function center(text: string, cols = 80): string {
  const stripped = text.replace(/\x1b\[[0-9;]*m/g, '')
  const pad = Math.max(0, Math.floor((cols - stripped.length) / 2))
  return ' '.repeat(pad) + text
}

// ─── Screen 1: Onboarding Chooser ────────────────────────────────────

const SCREEN_CHOOSER = (() => {
  const lines: string[] = []

  // Top spacer
  for (let i = 0; i < 6; i++) lines.push('')

  // Title
  lines.push(center(`${ACC}koma${RST}`))
  lines.push('')
  lines.push('')

  // Question
  lines.push(center(`${DIM}how do you want to connect?${RST}`))
  lines.push('')

  // Options — option 0 is selected
  const OPTS = [
    { label: 'koma free', desc: 'start now, no key - free models hosted by koma', sel: true },
    { label: 'provider', desc: 'sign in to a provider account', sel: false },
    { label: 'custom', desc: 'your own endpoint + API key', sel: false },
  ]

  for (const opt of OPTS) {
    const prefix = opt.sel ? `${BOLD_ACC}> ${ACC}` : '  '
    const labelColor = opt.sel ? ACC : FG
    const descStr = opt.desc
    if (opt.sel) {
      lines.push(center(`${prefix}${labelColor}${opt.label}${RST}  ${DIM}${descStr}${RST}`))
    } else {
      lines.push(center(`  ${labelColor}${opt.label}${RST}  ${DIM}${descStr}${RST}`))
    }
  }

  lines.push('')
  lines.push('')

  // Callout box
  const BOX_W = 62
  lines.push(center(`${WARN}┌${'─'.repeat(BOX_W - 2)}┐${RST}`))
  lines.push(center(`${WARN}│${RST}  ${WARN}you can change this anytime in /settings${RST}${' '.repeat(18)}${WARN}│${RST}`))
  lines.push(center(`${WARN}│${RST}  ${WARN}or type /free to switch to the free tier later${RST}${' '.repeat(14)}${WARN}│${RST}`))
  lines.push(center(`${WARN}└${'─'.repeat(BOX_W - 2)}┘${RST}`))
  lines.push('')

  // Footer
  lines.push(center(`${DIM}up/down move · enter select · q quit${RST}`))

  // Pad to fill screen
  while (lines.length < 24) lines.push('')

  return lines.join('\n')
})()

// ─── Screen 2: Chat (post-onboarding) ────────────────────────────────

const SCREEN_CHAT = (() => {
  const lines: string[] = []
  const W = 80

  // Header
  lines.push(`  ${DIM}koma${RST}  ${ACC}0.3.16${RST}` + ' '.repeat(W - 28) + `${ACC}● normal${RST}`)
  lines.push(`${DIM}${'─'.repeat(W)}${RST}`)
  lines.push('')

  // Transcript area — show a welcome message from the assistant
  lines.push(`  ${FG}●${RST} ${FG}welcome! i'm koma, your coding agent.${RST}`)
  lines.push(`  ${FG}  ${RST}${DIM}i read your code, plan changes, edit files, run commands,${RST}`)
  lines.push(`  ${FG}  ${RST}${DIM}and verify everything works.${RST}`)
  lines.push('')
  lines.push(`  ${FG}●${RST} ${FG}try typing a task below — i'll get to work.${RST}`)
  lines.push('')
  lines.push('')

  // Fill remaining transcript space
  while (lines.length < 18) lines.push('')

  // Model name row
  lines.push(`${DIM}${'─'.repeat(W)}${RST}`)
  lines.push(`  ${DIM}claude-sonnet-4-20250514${RST}`.padEnd(W))

  // Input box
  lines.push(`${DIM}${'─'.repeat(W)}${RST}`)
  lines.push(`  ${ACC}[$]${RST} ${ACC}█${RST}`)
  lines.push(`${DIM}${'─'.repeat(W)}${RST}`)

  // Status bar
  lines.push(`  ${DIM}ready${RST}` + ' '.repeat(W - 10) + `${ACC}↑12 ↓8${RST}  ${DIM}${ACC}↑0.3k ↓0.1k${RST}  ${DIM}$0.00${RST}`)

  return lines.join('\n')
})()

// ─── Transition: user types a command ─────────────────────────────────

const USER_MSG = (() => {
  const lines: string[] = []

  // Replace input line with typed text, then show the user message band
  // This is the "user typed something" state
  lines.push(`  ${ACC}[$]${RST} hello!${ACC}█${RST}`)

  return lines.join('\n')
})()

// ─── Full Script ──────────────────────────────────────────────────────

export const FIRST_RUN_SCRIPT = [
  // 1. Clear and show onboarding
  { text: CLR + SCREEN_CHOOSER, delay: 800 },

  // 2. Brief pause to read
  { text: '', delay: 2000 },

  // 3. Transition: select "koma free" → clear → show chat
  { text: CLR + SCREEN_CHAT, delay: 600 },
]
