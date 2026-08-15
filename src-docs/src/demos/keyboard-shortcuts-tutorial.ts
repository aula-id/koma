/**
 * Tutorial screen for keyboard shortcuts — full-screen reference page (80×24).
 *
 * Step 1: Keyboard Shortcuts — two-column table listing every keybinding
 *         from src-agent/src/controller/command.rs, with key combo on the
 *         left (accent, 18-char column) and action description on the right (fg).
 *
 * Layout:
 *   Row 0:     "keyboard shortcuts" (accent bold) + dim rule to 80
 *   Row 1:     blank
 *   Rows 2-12: 11 keybinding rows
 *   Rows 13-22: blank
 *   Row 23:    INVERSE footer bar
 *
 * Colours use the same ANSI mappings as theme.rs dark():
 *   \x1b[32m  accent  (#39ff14)
 *   \x1b[37m  fg      (#e6e6e6)
 *   \x1b[90m  dim     (#adadad)
 *   \x1b[30m  sel_fg  (black)
 *   \x1b[42m  sel_bg  (accent)
 *   \x1b[92m  success (#00c853)
 *   \x1b[33m  warn    (#ffb43c)
 *   \x1b[94m  info    (#50c8ff)
 */

const RST = '\x1b[0m'
const ACC = '\x1b[32m'  // accent green #39ff14
const FG  = '\x1b[37m'  // foreground #e6e6e6
const DIM = '\x1b[90m'  // dim #adadad
const SEL_FG = '\x1b[30m'  // selection foreground black
const SEL_BG = '\x1b[42m'  // selection background accent green
const INVERSE = SEL_FG + SEL_BG + '\x1b[1m'
const SUCCESS = '\x1b[92m'  // bright green #00c853
const WARN = '\x1b[33m'    // yellow #ffb43c
const INFO = '\x1b[94m'    // bright blue #50c8ff

// ─── helpers ──────────────────────────────────────────────────────────

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
  if (vis >= w) return text
  return text + ' '.repeat(w - vis)
}

function line80(content: string): string {
  return padRight(trunc(content, 80), 80)
}

// ─── Data ─────────────────────────────────────────────────────────────

const KEY_COL = 18 // key column width for the keyboard shortcuts page

interface Shortcut {
  key: string
  desc: string
}

/** All 11 keybindings from KEYBINDINGS table in command.rs. */
const SHORTCUTS: Shortcut[] = [
  { key: 'Enter',          desc: 'send message / run command' },
  { key: 'Tab',            desc: 'complete the selected command' },
  { key: 'Ctrl+R',         desc: 'resend the last message' },
  { key: 'Ctrl+E',         desc: 'toggle internet mode (simple / full)' },
  { key: 'Ctrl+J',         desc: 'insert a newline' },
  { key: 'Ctrl+V',         desc: 'paste an image from the clipboard' },
  { key: 'Ctrl+X',         desc: 'kill the selected bash job / sub-agent' },
  { key: 'Esc',            desc: 'interrupt while busy' },
  { key: 'Esc Esc',        desc: 'edit a previous message (rewind)' },
  { key: 'Up/Down/wheel',  desc: 'scroll the transcript' },
  { key: '$',              desc: 'open the sub-agents panel \u2014 Ctrl+X kills the selected' },
]

// ─── Screen Builder ───────────────────────────────────────────────────

/**
 * Build a full 80×24 keyboard shortcuts screen.
 *
 * @param rows Total screen height (default 24)
 */
function buildKeyboardScreen(rows: number): string {
  const W = 80
  const lines: string[] = []

  // ── Row 0: "keyboard shortcuts" (accent bold) + dim rule ──────────
  const headerText = 'keyboard shortcuts' // 18 visible chars
  lines.push(
    line80(
      '\x1b[1m' + ACC + headerText + RST +
        ' ' + DIM + '\u2500'.repeat(W - headerText.length - 1) + RST,
    ),
  )

  // ── Row 1: blank ──────────────────────────────────────────────────
  lines.push(line80(''))

  // ── Rows 2-12: 11 keybinding rows ────────────────────────────────
  for (const s of SHORTCUTS) {
    const keyPart = ACC + s.key.padEnd(KEY_COL) + RST
    const full = keyPart + FG + s.desc + RST
    lines.push(line80(full))
  }

  // ── Rows 13-22: blank fill ────────────────────────────────────────
  while (lines.length < rows - 1) lines.push(line80(''))

  // ── Row 23: INVERSE footer bar ────────────────────────────────────
  const footerText = ' \u2191\u2193 scroll \u00b7 Esc close '
  lines.push(INVERSE + padRight(footerText, W) + RST)

  return lines.slice(0, rows).join('\n')
}

// ─── Tutorial Steps ───────────────────────────────────────────────────

import type { TutorialStep } from './first-run-tutorial'

export function getKeyboardShortcutsSteps(rows = 24): TutorialStep[] {
  return [
    {
      title: 'Keyboard Shortcuts',
      narration:
        'koma uses keyboard-driven navigation for speed. Here is every keybinding ' +
        'available in the TUI \u2014 from sending messages and switching context to ' +
        'resending messages and navigating the transcript.',
      points: [
        'Enter sends your message or runs a slash command',
        'Tab completes the selected command from the palette',
        'Ctrl+R resends the last message; Ctrl+E toggles internet mode',
        'Esc interrupts while busy; press Esc twice to rewind to a previous message',
      ],
      screen: buildKeyboardScreen(rows),
    },
  ]
}
