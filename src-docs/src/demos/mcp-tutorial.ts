import { RST, ACC, FG, DIM, INVERSE, SEL_FG, SEL_BG, trunc, padRight, bar, chatHeader, chatInput } from './chat-chrome'
import type { TutorialStep } from './first-run-tutorial'

const W = 80
function commandEntry(rows: number): string { const lines = [...chatHeader(), '', '  Type /mcp in the composer to manage global servers.', ...chatInput('/mcp')]; while (lines.length < rows) lines.splice(2, 0, ''); return lines.slice(0, rows).map(l => trunc(l, W)).join('\n') }
function finish(lines: string[], hint: string, rows: number) { while (lines.length < rows - 1) lines.push(''); lines.push(INVERSE + padRight(' ' + hint, W) + RST); return lines.slice(0, rows).map(l => trunc(l, W)).join('\n') }
function shell(detail: string[], selected = 0): string[] {
  const lines = [DIM + '  mcp servers' + RST, DIM + bar('─', W) + RST]
  const list = ['context7         ● stdio', 'github           ● http', 'local            ○ stdio']
  for (let i = 0; i < Math.max(list.length, detail.length); i++) {
    const left = list[i] ?? ''
    lines.push((i === selected ? SEL_FG + SEL_BG + '› ' + padRight(left, 23) + RST : DIM + '  ' + padRight(left, 23) + RST) + DIM + '│' + RST + '  ' + (detail[i] ?? ''))
  }
  return lines
}
function browse(rows: number) { return finish(shell([ACC + 'name          context7' + RST, DIM + 'enabled       yes' + RST, DIM + 'transport     stdio' + RST, ACC + 'status        ● 12 tools' + RST, DIM + 'command       npx' + RST, DIM + 'args          -y @upstash/context7-mcp' + RST, DIM + 'env           (none)' + RST]), '↑/↓ pick · →/Enter edit · n new · d delete · Esc close', rows) }
function stdioEdit(rows: number) { return finish(shell([ACC + '› name          context7█' + RST, DIM + '  enabled       yes' + RST, DIM + '  transport     stdio  ←/→ stdio/http' + RST, DIM + '  command       npx' + RST, DIM + '  args          -y @upstash/context7-mcp' + RST, DIM + '  env           (KEY=VAL, KEY2=VAL2)' + RST]), 'type to edit · Enter/Esc done', rows) }
function httpEdit(rows: number) { return finish(shell([DIM + '  name          github' + RST, DIM + '  enabled       yes' + RST, ACC + '› transport     http  ←/→ stdio/http' + RST, DIM + '  url           https://api.githubcopilot.com/mcp/' + RST], 1), '←/→/Space toggle · ↑/↓ field · s save · Esc cancel', rows) }
function deleting(rows: number) { return finish(shell(['', FG + 'delete ' + RST + ACC + "'local'" + RST + FG + '?' + RST, DIM + 'this removes the server from config.json' + RST], 2), 'y delete · n/Esc cancel', rows) }
export function getMcpSteps(rows = 24): TutorialStep[] { return [
  { title: 'Enter /mcp', narration: 'Type /mcp in the normal chat composer and submit it. The command opens global MCP management and does not require an active session.', points: ['The dashboard replaces chat after the command is submitted.', '/mcp is available even when no session is active.'], screen: commandEntry(rows) },
  { title: 'Browse MCP servers', narration: 'The /mcp dashboard is available globally; no active chat session is required. Its 26-column sidebar lists configured servers with enabled and transport indicators.', points: ['The detail pane shows connection status when the global MCP manager is available.', '● N tools means a connected server; ○ — means it is not connected.', 'There is no test-server control in this panel.'], screen: browse(rows) },
  { title: 'Create or edit stdio', narration: 'A stdio server has Name, Enabled, Transport, Command, Args, and Env fields. Text fields edit inline and wrap when necessary.', points: ['n starts a new server; Enter edits a selected text field.', 'Args are space-separated and Env uses KEY=VAL pairs.', 's saves the server configuration.'], screen: stdioEdit(rows) },
  { title: 'Switch to HTTP', narration: 'Transport is a toggle, not free text. Switching from stdio to HTTP replaces Command, Args, and Env with the URL field.', points: ['Use ←/→ or Space on Transport to switch stdio/http.', 'HTTP requires a URL before it can be saved.', 'Save reconnects the global MCP configuration.'], screen: httpEdit(rows) },
  { title: 'Delete confirmation', narration: 'Deletion is explicit and removes the selected server from config.json.', points: ['Press y to delete.', 'Press n or Esc to cancel.'], screen: deleting(rows) },
] }
