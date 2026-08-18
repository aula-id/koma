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

import { RST, ACC, FG, DIM, INVERSE, trunc, padRight, bar, chatHeader, chatInput, commandEntryScreen } from './chat-chrome'

// ─── Screen: /model Help ──────────────────────────────────────────────
// Full-width overlay anchored right above the input bar.
// Border: dim. Title in border: " model — help ".
// Content: help lines in accent. Hint: "Esc close" in dim (inside box).

function screenModelHelp(rows = 24): string {
  const W = 80
  const lines: string[] = []

  // Chat header
  lines.push(...chatHeader())

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
      title: 'Type /model',
      narration: 'From normal chat, type /model in the composer and press Enter to open its help overlay.',
      screen: commandEntryScreen(rows, '/model'),
    },
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
      title: 'Type /model main',
      narration: 'Return to normal chat, type /model main, and press Enter to open the main-role model picker.',
      screen: commandEntryScreen(rows, '/model main'),
    },
    {
      title: 'Role Picker',
      narration:
        'After /model main, a picker overlay lists all configured models. The current selection is highlighted. Choose a different model or press Escape to keep the current one.',
      points: [
        'Inherit removes the session override and uses the global role model',
        'Koma free uses the keyless models hosted by koma',
        'Models show their name, ID, and provider',
        'Press Enter to apply, Esc to cancel without changing',
      ],
      screen: screenModelRolePicker(rows),
    },
  ]
}
