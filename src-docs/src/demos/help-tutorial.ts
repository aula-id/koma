/**
 * Tutorial screens for /help — the full-screen command reference (80×24).
 *
 * Step 1: Command Reference — full list of 31 commands + 11 keybindings,
 *         with "updating koma" info block, search bar, /settings selected.
 * Step 2: Filtered Search — query "mod", two matches: /model (selected) and /mode.
 *
 * Layout matches src-agent/src/view/help.rs exactly at 80×24:
 *   Row 0:     " help " (dim) text
 *   Row 1:     dim BOTTOM rule
 *   Rows 2-5:  "updating koma" info block (4 lines)
 *   Rows 6-7:  search bar + spacer
 *   Rows 8-22: filtered command/keybinding list (15 rows)
 *   Row 23:    INVERSE footer bar
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

const KEY_W = 11 // key column padding width (visible: 1 leading space + 11 = 12)

interface HelpEntry {
  key: string
  desc: string
  isCommand: boolean
}

/**
 * Full flat list: 31 commands then 11 keybindings, matching command.rs order.
 * Commands get accent-colored keys; keybindings get dim-colored keys.
 */
const ALL_ENTRIES: HelpEntry[] = [
  // Commands (COMMANDS table from command.rs)
  { key: '/new',           desc: 'Spawn a new session, swap to it (current keeps running)',     isCommand: true },
  { key: '/new kill',      desc: 'Spawn a new session and close the current one',               isCommand: true },
  { key: '/new remote',    desc: 'Start a new session on a saved remote host',                  isCommand: true },
  { key: '/resume',        desc: 'Open the session hub (live + past sessions)',                 isCommand: true },
  { key: '/resume remote', desc: 'Resume a session on a saved remote host',                    isCommand: true },
  { key: '/mode',          desc: 'Toggle Normal/Auto tool approval',                           isCommand: true },
  { key: '/effort',        desc: 'Set model reasoning/thinking effort',                        isCommand: true },
  { key: '/free',          desc: 'Toggle this session to use koma-free',                       isCommand: true },
  { key: '/internet',      desc: 'Toggle internet mode (simple | full)',                       isCommand: true },
  { key: '/settings',      desc: 'Edit key, model, provider, theme, name',                    isCommand: true },
  { key: '/agents',        desc: 'Create, modify, or delete agent definitions',               isCommand: true },
  { key: '/mcp',           desc: 'Add, edit, or remove MCP servers',                          isCommand: true },
  { key: '/extension',     desc: 'Manage installed extensions (detail, uninstall, screens)',   isCommand: true },
  { key: '/store',         desc: 'Browse and install extensions from the koma.run marketplace',isCommand: true },
  { key: '/security',      desc: 'Security daemon control panel',                              isCommand: true },
  { key: '/remote',        desc: 'Manage remote SSH hosts and sessions',                       isCommand: true },
  { key: '/task',          desc: 'Run an agent on a task, or open the sub-agents viewer (no args)', isCommand: true },
  { key: '/model',         desc: 'Switch session / agent model  (/model for help)',            isCommand: true },
  { key: '/bash',          desc: 'Manage background bash jobs',                                isCommand: true },
  { key: '/todo',          desc: 'View the session task list',                                 isCommand: true },
  { key: '/skill',         desc: 'Load or unload agent skills',                                isCommand: true },
  { key: '/attach',        desc: 'Attach a .screenshoot/*.png to the next message',           isCommand: true },
  { key: '/cd',            desc: 'Change the session working directory',                       isCommand: true },
  { key: '/adddir',        desc: 'Add a directory to the workspace roots',                    isCommand: true },
  { key: '/compact',       desc: 'Summarize and compact the conversation',                     isCommand: true },
  { key: '/clear',         desc: 'Clear the chat history (keeps system prompt + archive)',     isCommand: true },
  { key: '/usage',         desc: 'Show the cost and token usage dashboard',                    isCommand: true },
  { key: '/rename',        desc: 'Rename the current session',                                 isCommand: true },
  { key: '/select',        desc: 'Dump full history to terminal for native copy',              isCommand: true },
  { key: '/help',          desc: 'List the available commands',                                 isCommand: true },
  { key: '/quit',          desc: 'Quit koma',                                                  isCommand: true },
  // Keybindings (KEYBINDINGS table from command.rs)
  { key: 'Enter',          desc: 'send message / run command',                                 isCommand: false },
  { key: 'Tab',            desc: 'complete the selected command',                              isCommand: false },
  { key: 'Ctrl+R',         desc: 'resend the last message',                                   isCommand: false },
  { key: 'Ctrl+E',         desc: 'toggle internet mode (simple / full)',                       isCommand: false },
  { key: 'Ctrl+J',         desc: 'insert a newline',                                          isCommand: false },
  { key: 'Ctrl+V',         desc: 'paste an image from the clipboard',                         isCommand: false },
  { key: 'Ctrl+X',         desc: 'kill the selected bash job / sub-agent',                    isCommand: false },
  { key: 'Esc',            desc: 'interrupt while busy',                                      isCommand: false },
  { key: 'Esc Esc',        desc: 'edit a previous message (rewind)',                          isCommand: false },
  { key: 'Up/Down/wheel',  desc: 'scroll the transcript',                                     isCommand: false },
  { key: '$',              desc: 'open the sub-agents panel \u2014 Ctrl+X kills the selected',     isCommand: false },
]

// ─── Screen Builder ───────────────────────────────────────────────────

/**
 * Build a full 80×24 help screen.
 *
 * @param rows         Total screen height (default 24)
 * @param entries      The entries to display (full list or filtered subset)
 * @param query        Current search query string
 * @param selectedIdx  Index of the selected entry within `entries`
 */
function buildHelpScreen(
  rows: number,
  entries: HelpEntry[],
  query: string,
  selectedIdx: number,
): string {
  const W = 80
  const lines: string[] = []

  // ── Row 0: " help " (dim) text ────────────────────────────────────
  lines.push(line80(DIM + '  help'))

  // ── Row 1: dim BOTTOM rule (full width) ───────────────────────────
  lines.push(line80(DIM + '\u2500'.repeat(W) + RST))

  // ── Rows 2-5: "updating koma" info block ──────────────────────────
  lines.push(line80(DIM + '  updating koma'))
  lines.push(line80(DIM + '  current 0.3.16  \u00b7  available ' + SUCCESS + '[0.4.0]' + RST))
  lines.push(line80(DIM + '  run  koma update  or  curl -fsSL https://koma.run/install.sh | sh' + RST))
  lines.push(line80('')) // spacer

  // ── Rows 6-7: search zone ─────────────────────────────────────────
  const queryPart = query ? FG + query + RST : ''
  lines.push(line80(DIM + '\u203a ' + RST + queryPart + ACC + '\u2588' + RST))
  lines.push(line80('')) // spacer

  // ── Rows 8-22: filtered list (15 rows) ────────────────────────────
  const listRows = rows - 9 // 24 - 9 = 15
  const sel = Math.min(selectedIdx, entries.length - 1)

  // Window around selection so the selected row stays visible
  let start = Math.max(0, sel - Math.floor(listRows / 2))
  let end = Math.min(entries.length, start + listRows)
  if (end - start < listRows) {
    start = Math.max(0, end - listRows)
  }

  for (let vi = 0; vi < listRows; vi++) {
    const i = start + vi
    if (i >= entries.length) {
      lines.push(line80(''))
      continue
    }
    const e = entries[i]
    // Key column: 1 leading space + key padded to KEY_W = 12 visible chars
    const keyPart = (' ' + e.key).padEnd(KEY_W + 1)

    if (i === sel) {
      // Selected row: entire line INVERSE (black on accent-green, bold)
      const full = keyPart + e.desc
      lines.push(INVERSE + padRight(trunc(full, W), W) + RST)
    } else {
      // Non-selected: command keys in accent, keybinding keys in dim
      const keyColor = e.isCommand ? ACC : DIM
      const styledKey = keyColor + keyPart + RST
      const full = styledKey + FG + e.desc + RST
      lines.push(line80(full))
    }
  }

  // ── Row 23: INVERSE footer bar ────────────────────────────────────
  const footerHint = 'type to search \u00b7 \u2191\u2193 select \u00b7 Enter run \u00b7 Esc close'
  lines.push(INVERSE + padRight(' ' + footerHint, W) + RST)

  return lines.slice(0, rows).join('\n')
}

// ─── Tutorial Steps ───────────────────────────────────────────────────

import type { TutorialStep } from './first-run-tutorial'

export function getHelpSteps(rows = 24): TutorialStep[] {
  // Step 1: Full list — /settings selected (index 9 in flat list)
  const screen1 = buildHelpScreen(rows, ALL_ENTRIES, '', 9)

  // Step 2: Filtered by "mod" — /model (selected) and /mode
  const filteredMod: HelpEntry[] = [
    { key: '/model', desc: 'Switch session / agent model  (/model for help)', isCommand: true },
    { key: '/mode',  desc: 'Toggle Normal/Auto tool approval',                isCommand: true },
  ]
  const screen2 = buildHelpScreen(rows, filteredMod, 'mod', 0)

  return [
    {
      title: 'Command Reference',
      narration:
        'Type /help in the chat to open the full-screen command reference. ' +
        'It lists every slash command and keyboard shortcut with a search bar at the top, ' +
        'an "updating koma" info block, and live filtering.',
      points: [
        'Commands (green) and keybindings (dim) are listed together in a flat list',
        'Start typing in the search bar to filter the list instantly',
        'Press \u2191\u2193 to highlight a row, Enter to execute it',
      ],
      screen: screen1,
    },
    {
      title: 'Filtered Search',
      narration:
        'As you type in the search bar, the list narrows in real time. ' +
        'Here "mod" filters to /model and /mode \u2014 two commands for switching ' +
        'your AI provider and tool-approval mode.',
      points: [
        'Partial matches work \u2014 "mod" matches both /model and /mode',
        'The cursor stays in the search bar for further filtering',
        'Press Esc to clear the filter and return to the full list',
      ],
      screen: screen2,
    },
  ]
}
