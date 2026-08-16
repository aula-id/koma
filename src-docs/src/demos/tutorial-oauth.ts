import { RST, ACC, FG, DIM, SEL_FG, SEL_BG, INVERSE, trunc, padRight, bar, chatHeader, chatInput, commandEntryScreen } from './chat-chrome'
import type { TutorialStep } from './first-run-tutorial'

const W = 80
const finish = (lines: string[], rows: number, footer: string) => {
  while (lines.length < rows - 1) lines.push('')
  lines.push(INVERSE + padRight(footer, W) + RST)
  return lines.slice(0, rows).map(line => trunc(line, W)).join('\n')
}
const hdr = (page = 'OAuth') => [DIM + `  settings > ${page}` + RST, DIM + bar('─', W) + RST, '']

function settingsMenu(rows: number, selected = 3): string {
  const lines = [...chatHeader()]
  const menu = ['Appearance', 'General', 'Providers', 'OAuth', 'Models']
  const overlay = [DIM + '┌ settings ' + bar('─', W - 12) + '┐' + RST]
  for (const [i, label] of menu.entries()) {
    const text = ` [${i + 1}]  ${label}`
    overlay.push(DIM + '│' + RST + (i === selected ? INVERSE + padRight(text, W - 2) + RST : padRight(ACC + `[${i + 1}]` + RST + `  ${label}`, W - 2)) + DIM + '│' + RST)
  }
  overlay.push(DIM + '└' + bar('─', W - 2) + '┘' + RST)
  const input = chatInput('/settings')
  while (lines.length < rows - input.length - overlay.length) lines.push('')
  lines.push(...overlay, ...input)
  return lines.slice(0, rows).map(line => trunc(line, W)).join('\n')
}

function screenOAuthList(rows = 24): string {
  const lines = hdr()
  const provW = 12, statusW = 16, acctW = W - 4 - provW - statusW - 2
  lines.push(DIM + '  ' + 'Provider'.padEnd(provW) + 'Account'.padEnd(acctW) + 'Status'.padEnd(statusW) + RST)
  while (lines.length < rows - 2) lines.push('')
  lines.push('  ' + SEL_FG + SEL_BG + padRight('[ + connect ]', W - 4) + RST)
  return finish(lines, rows, ' ↑↓ select · enter connect · ctrl+x delete · esc back')
}

function screenOAuthPicker(rows = 24): string {
  const lines = hdr()
  const providers = ['Codex', 'Kilo Code', 'koma.run', 'xAI', 'Claude', 'Command Code']
  for (const [i, label] of providers.entries()) {
    lines.push(i === 2 ? SEL_FG + SEL_BG + padRight('› ' + label, W) + RST : ACC + '  ' + RST + FG + label + RST)
  }
  return finish(lines, rows, ' ↑↓ select · enter choose · esc back')
}

function screenOAuthWait(rows = 24): string {
  const lines = hdr()
  lines.push(ACC + 'koma.run login' + RST, '')
  lines.push(DIM + 'https://auth.koma.run/authorize?client_id=koma&scope=openid+profile' + RST)
  return finish(lines, rows, ' c copy url · o open browser · esc cancel')
}

export function getTutorialOAuthSteps(rows = 24): TutorialStep[] {
  return [
    { title: 'Type /settings', narration: 'From normal chat, type /settings in the composer and submit it.', screen: commandEntryScreen(rows, '/settings') },
    { title: 'Select OAuth', narration: 'Press 4 or select OAuth in the five-item settings menu.', points: ['The compact menu is anchored above the composer.'], screen: settingsMenu(rows, 3) },
    { title: 'OAuth connections', narration: 'The OAuth page lists linked accounts. Move to [ + connect ] and press Enter.', points: ['The table is empty on first visit.', 'Ctrl+X deletes a selected connection.'], screen: screenOAuthList(rows) },
    { title: 'Choose a provider', narration: 'The provider picker overlays the OAuth body. Choose koma.run with ↑↓ and Enter.', points: ['Six browser-based providers plus two paste-token options.'], screen: screenOAuthPicker(rows) },
    { title: 'Complete browser sign-in', narration: 'The body shows the login URL. Press c to copy it, or o to open it in your browser. Approve the sign-in.', points: ['Esc cancels the flow.', 'On success, the connection appears in the OAuth list with status active.'], screen: screenOAuthWait(rows) },
  ]
}
