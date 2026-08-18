/**
 * Shared chat chrome for all non-fullscreen tutorial screens.
 *
 * Provides ANSI constants, string helpers, and the two universal chrome
 * components — chatHeader() and chatInput() — that every non-fullscreen
 * tutorial wraps around its overlay content.
 *
 * Fullscreen tutorials (resume, usage, settings-model, settings-oauth,
 * help, keyboard-shortcuts) import the ANSI + helper exports only.
 */

// ─── ANSI constants ─────────────────────────────────────────────────

export const RST      = '\x1b[0m'
export const ACC      = '\x1b[32m'      // accent green #39ff14
export const FG       = '\x1b[37m'      // fg white #e6e6e6
export const DIM      = '\x1b[90m'      // dim #adadad
export const WARN     = '\x1b[33m'      // warn amber #ffb43c
export const SEL_FG   = '\x1b[30m'      // selection foreground (black)
export const SEL_BG   = '\x1b[42m'      // selection background (green)
export const INVERSE  = SEL_FG + SEL_BG + '\x1b[1m'
export const BOLD_ACC = '\x1b[1;32m'
export const REVERSE  = '\x1b[7m'       // raw reverse video
export const CLR      = '\x1b[2J\x1b[H' // clear screen + home

// ─── String helpers ─────────────────────────────────────────────────

/** Strip ANSI escape codes from a string. */
export function stripAnsi(s: string): string {
  return s.replace(/\x1b\[[0-9;]*m/g, '')
}

/**
 * Truncate a line to `w` visible characters, preserving ANSI state.
 * Appends a reset if the line is cut mid-sequence.
 */
export function trunc(line: string, w: number): string {
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
export function padRight(text: string, w: number): string {
  const vis = stripAnsi(text).length
  return text + ' '.repeat(Math.max(0, w - vis))
}

/** Right-align `text` (with ANSI) within `w` visible chars. */
export function rightAlign(text: string, w: number): string {
  const vis = stripAnsi(text).length
  return ' '.repeat(Math.max(0, w - vis)) + text
}

/** Fill a row with `ch` repeated `w` times. */
export function bar(ch: string, w: number): string {
  return ch.repeat(w)
}

/** Pad a line to exactly 80 visible characters. */
export function line80(content: string): string {
  return padRight(trunc(content, 80), 80)
}

// ─── Chat chrome: Header ────────────────────────────────────────────
/**
 * The 2-row header block that appears at the top of every non-fullscreen
 * chat screen: brand + version left-aligned, mode right-aligned, then a
 * dim horizontal rule.
 *
 * Matches src-agent/src/view/chat/header.rs render_header().
 */
export function chatHeader(modeLabel = 'normal'): string[] {
  const W = 80
  const brand = DIM + 'koma' + RST + ' ' + ACC + '0.3.16' + RST
  const modeStyles: Record<string, string> = {
    auto: WARN + '» auto' + RST,
    normal: ACC + '\u{25cf} normal' + RST,
    planning: '\x1b[36m● planning' + RST,
    sdlc: '\x1b[36m◆ sdlc:assess' + RST,
  }
  const mode = modeStyles[modeLabel] ?? (ACC + '\u{25cf} ' + modeLabel + RST)
  const gap = Math.max(1, W - 4 - stripAnsi(brand).length - stripAnsi(mode).length)
  return [
    '  ' + brand + ' '.repeat(gap) + mode,
    DIM + bar('\u2500', W) + RST,
  ]
}

// ─── Chat chrome: Composer (input bar) ──────────────────────────────
/**
 * The 5-row input block anchored at the bottom of every non-fullscreen
 * chat screen:
 *
 *   row 0  — model name (right-aligned, dim)
 *   row 1  — top border ──────── session-name (right-aligned on border)
 *   row 2  — [$] <text>█       (input content)
 *   row 3  — bottom border ────
 *   row 4  — ready              (status bar, dim)
 *
 * Matches src-agent/src/view/chat/input.rs render_input() with
 * Block::borders(TOP|BOTTOM) and session as right-aligned title.
 */
export function chatInput(text: string): string[] {
  const W = 80
  // Model name row: right-aligned, dim (matches render_model_row)
  const model     = 'claude-3.5-sonnet'
  const modelLine = DIM + ' '.repeat(W - 2 - model.length) + model + RST
  // Top border with session name right-aligned (matches input.rs Block title)
  const sess      = ' session-71cdd2dc '
  const topBorder = DIM + bar('\u2500', W - sess.length) + ACC + sess + RST
  return [
    modelLine,
    topBorder,
    '  ' + ACC + '[$] ' + RST + ACC + text + '\u{2588}' + RST,
    DIM + bar('\u2500', W) + RST,
    '  ' + DIM + 'ready' + RST,
  ]
}

/**
 * Compose a complete non-fullscreen screen: chat chrome (header + transcript
 * lines) + input bar, padded to `rows` total lines, then truncated to `W`
 * visible columns per line.
 *
 * Usage in a tutorial step builder:
 *   return composeChatScreen(rows, userLines, 'what files changed?')
 *
 * `commandEntryScreen` creates a normal 80×24 chat frame with a slash command
 * typed in the composer, before that command is run.
 */
export function commandEntryScreen(rows: number, command: string, modeLabel = 'normal'): string {
  return composeChatScreen(rows, [], command, 80, modeLabel)
}

export function composeChatScreen(
  rows: number,
  transcriptLines: string[],
  inputText: string,
  cols = 80,
  modeLabel = 'normal',
): string {
  const header = chatHeader(modeLabel)
  const input  = chatInput(inputText)
  const all    = [...header, ...transcriptLines, ...input]
  // Pad transcript with blank lines so input lands at the bottom
  while (all.length < rows) all.splice(header.length, 0, '')
  return all.slice(0, rows).map(l => trunc(l, cols)).join('\n')
}
