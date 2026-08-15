/**
 * Tutorial screens for the /model command overlay.
 *
 * Two screens: help text and role picker.
 * The overlay is anchored ABOVE the input bar, full-width — same pattern
 * as /settings menu, /bash, /todo overlays.
 * Matches src-agent/src/view/model_cmd.rs exactly.
 *
 * Key design: dim border, title in border (dim), help text in accent,
 * options: inherit=dim, concrete=accent, selected=inverse.
 * Hint line INSIDE the box, dim.
 */

const RST = '\x1b[0m'
const ACC = '\x1b[32m'
const FG  = '\x1b[37m'
const DIM = '\x1b[90m'
const SEL_FG = '\x1b[30m'
const SEL_BG = '\x1b[42m'
const INVERSE = SEL_FG + SEL_BG + '\x1b[1m'

function stripAnsi(s: string): string {
  return s.replace(/\x1b\[[0-9;]*m/g, '')
}

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

function padRight(text: string, w: number): string {
  const vis = stripAnsi(text).length
  return text + ' '.repeat(Math.max(0, w - vis))
}

function bar(ch: string, w: number): string {
  return ch.repeat(w)
}

// ─── Shared: Chat chrome ──────────────────────────────────────────────

function chatHeader(): string[] {
  const W = 80
  const brand = DIM + 'koma' + RST + ' ' + ACC + '0.3.16' + RST
  const mode = ACC + '\u25cf normal' + RST
  const gap = Math.max(1, W - 4 - stripAnsi(brand).length - stripAnsi(mode).length)
  return [
    '  ' + brand + ' '.repeat(gap) + mode,
    DIM + bar('\u2500', W) + RST,
  ]
}

function chatInput(text: string): string[] {
  const W = 80
  return [
    line80('     ' + DIM + 'claude-3.5-sonnet' + RST),
    DIM + bar('\u2500', W) + RST,
    '  ' + ACC + '[$] ' + RST + ACC + text + '\u{2588}' + RST,
    DIM + bar('\u2500', W) + RST,
    line80('  ' + ACC + 'session-71cdd2dc' + RST),
  ]
}

// ─── Screen: /model Help ──────────────────────────────────────────────
// Full-width overlay anchored right above the input bar.
// Border: dim. Title in border: " model — help ".
// Content: help lines in accent. Hint: "Esc close" in dim (inside box).

function screenModelHelp(rows = 24): string {
  const W = 80
  const lines: string[] = []

  // Chat header
  lines.push(...chatHeader())

  // Brief transcript — overlay covers most of the screen
  lines.push('')
  lines.push('  ' + FG + 'what files changed in the last commit?' + RST)
  lines.push('')

  // Build overlay lines
  const overlayLines: string[] = []
  const innerW = W - 2
  const title = ' model \u2014 help '
  overlayLines.push(DIM + '\u250c' + title + bar('\u2500', W - 2 - title.length) + '\u2510' + RST)

  // Help text — each line in accent, padded with inner margin
  const helpLines = [
    ' /model \u2014 session model switcher',
    '',
    '  main         claude-sonnet-4-20250514',
    '  awareness    claude-haiku-3-20240307',
    '  planner      (unset)',
    '  compactor    (unset)',
    '  safeguard    (unset)',
    '',
    '  /model <role>            swap role model',
    '  /model agent             pick agent, then model',
    '  /model agent <name>      swap model for agent',
    '',
    '  agents: explore, coder',
  ]
  for (const hl of helpLines) {
    overlayLines.push(DIM + '\u2502' + RST + ACC + hl.padEnd(innerW) + RST + DIM + '\u2502' + RST)
  }

  // Hint (inside box, dim)
  overlayLines.push(DIM + '\u2502' + RST + DIM + 'Esc close'.padEnd(innerW) + RST + DIM + '\u2502' + RST)
  overlayLines.push(DIM + '\u2514' + bar('\u2500', W - 2) + '\u2518' + RST)

  // Place overlay right above the input bar
  const inputBar = chatInput('/model')
  const targetStart = rows - inputBar.length - overlayLines.length
  while (lines.length < targetStart) lines.push('')
  lines.push(...overlayLines)
  lines.push(...inputBar)

  while (lines.length < rows) lines.push('')
  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

// ─── Screen: /model Role Picker ───────────────────────────────────────
// Same anchoring. Title: " model — main ".
// Options: inherit=dim, koma free=dim, concrete=accent, selected=inverse.
// Hint inside box.

function screenModelRolePicker(rows = 24): string {
  const W = 80
  const lines: string[] = []

  // Chat header
  lines.push(...chatHeader())

  // Brief transcript
  lines.push('')
  lines.push('  ' + FG + 'what files changed in the last commit?' + RST)
  lines.push('')

  // Build overlay lines
  const overlayLines: string[] = []
  const innerW = W - 2
  const title = ' model \u2014 main '
  overlayLines.push(DIM + '\u250c' + title + bar('\u2500', W - 2 - title.length) + '\u2510' + RST)

  // Options — matches real model_cmd option construction
  const options = [
    { label: '(inherit session default)', concrete: false, sel: false },
    { label: 'koma free \u2014 keyless', concrete: false, sel: false },
    { label: 'claude-sonnet-4-20250514 \u2014 anthropic/claude-sonnet @ Claude', concrete: true, sel: false },
    { label: 'gpt-4o-mini \u2014 openai/gpt-4o-mini @ OpenRouter', concrete: true, sel: true },
    { label: 'claude-haiku-3-20240307 \u2014 anthropic/claude-haiku @ Claude', concrete: true, sel: false },
  ]

  for (const opt of options) {
    const text = ' ' + opt.label + ' '
    if (opt.sel) {
      // Cursor row — inverse, padded to full inner width
      overlayLines.push(DIM + '\u2502' + RST + INVERSE + padRight(text, innerW) + RST + DIM + '\u2502' + RST)
    } else {
      const color = opt.concrete ? ACC : DIM
      overlayLines.push(DIM + '\u2502' + RST + color + text.padEnd(innerW) + RST + DIM + '\u2502' + RST)
    }
  }

  // Hint (inside box, dim)
  overlayLines.push(DIM + '\u2502' + RST + DIM + '\u2191\u2193 select \u00b7 Enter apply \u00b7 Esc cancel'.padEnd(innerW) + RST + DIM + '\u2502' + RST)
  overlayLines.push(DIM + '\u2514' + bar('\u2500', W - 2) + '\u2518' + RST)

  // Place overlay right above the input bar
  const inputBar = chatInput('/model main')
  const targetStart = rows - inputBar.length - overlayLines.length
  while (lines.length < targetStart) lines.push('')
  lines.push(...overlayLines)
  lines.push(...inputBar)

  while (lines.length < rows) lines.push('')
  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

// ─── Tutorial Steps ───────────────────────────────────────────────────

import type { TutorialStep } from './first-run-tutorial'

export function getCommandsModelSteps(rows = 24): TutorialStep[] {
  return [
    {
      title: '/model Help',
      narration:
        'Type /model in the chat to see available model management commands. The overlay appears above the input bar showing all sub-commands and current role assignments.',
      points: [
        '/model main switches the primary coding model',
        '/model awareness switches the context-gathering model',
        '/model <agent> overrides the model for a specific sub-agent',
      ],
      screen: screenModelHelp(rows),
    },
    {
      title: 'Role Picker',
      narration:
        'After /model main, a picker overlay lists all configured models. The current selection is highlighted. Choose a different model or press Escape to keep the current one.',
      points: [
        'Inherit (session default) uses the model from /settings \u2192 Models',
        'Koma free uses the keyless models hosted by koma',
        'Models show their name, ID, and provider',
        'Press Enter to apply, Esc to cancel without changing',
      ],
      screen: screenModelRolePicker(rows),
    },
  ]
}
