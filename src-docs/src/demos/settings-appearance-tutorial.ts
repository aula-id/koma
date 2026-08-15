import { RST, ACC, DIM, INVERSE, trunc, padRight, bar, chatHeader, chatInput } from './chat-chrome'
import type { TutorialStep } from './first-run-tutorial'

const W = 80
const bg = (n: number) => `\x1b[48;5;${n}m  ${RST}`

export function settingsChatEntry(rows: number): string {
  const lines = [...chatHeader(), '', '  Type a command in the composer.', ...chatInput('/settings')]
  while (lines.length < rows) lines.splice(2, 0, '')
  return lines.slice(0, rows).map(line => trunc(line, W)).join('\n')
}

export function settingsMenu(rows: number, selected = 0): string {
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

function screenAppearance(rows = 24): string {
  const lines = [DIM + '  settings > Appearance' + RST, DIM + bar('─', W) + RST, '']
  const palettes: [string, boolean, boolean, number[]][] = [
    ['dark', true, false, [16, 236, 240, 255, 46, 81, 35, 214, 196]],
    ['light', false, true, [231, 255, 250, 16, 28, 27, 22, 178, 160]],
    ['forest', false, false, [58, 65, 101, 194, 137, 109, 65, 94, 131]],
  ]
  for (const [name, cursor, applied, colors] of palettes) {
    const border = cursor ? ACC : DIM
    const label = `${cursor ? ' > ' : ' '}${name}${applied ? ' · selected' : ''} `
    lines.push(border + '┌─' + label + bar('─', W - 2 - label.length - 1) + '┐' + RST)
    lines.push(border + '│ ' + RST + colors.map(bg).join(' ') + ' '.repeat(W - 30) + border + ' │' + RST)
    lines.push(border + '└' + bar('─', W - 2) + '┘' + RST)
  }
  lines.push(DIM + ' ↓ 7 more' + RST)
  while (lines.length < rows - 1) lines.push('')
  lines.push(INVERSE + padRight(' ↑↓ palette · enter apply · esc back', W) + RST)
  return lines.slice(0, rows).map((line) => trunc(line, W)).join('\n')
}

export function getSettingsAppearanceSteps(rows = 24): TutorialStep[] {
  return [
    { title: 'Open settings', narration: 'Type /settings in the chat composer and submit it to open settings.', points: ['Settings begins from the normal chat screen.', 'The command opens a compact overlay above the composer.'], screen: settingsChatEntry(rows) },
    { title: 'Select Appearance', narration: 'The settings overlay lists five numbered categories. Press 1 or select Appearance to enter the full-screen palette page.', points: ['The menu is anchored directly above the chat composer.', 'Esc from the menu saves settings and closes it.'], screen: settingsMenu(rows) },
    {
      title: 'Appearance palettes',
    narration: 'Appearance is a full-screen palette picker. The cursor and the palette currently applied to the UI are deliberately separate states.',
    points: ['Each palette is a three-row box containing its nine role-colour swatches.', 'The green border and > identify the cursor; · selected identifies the applied palette.', 'Use ↑↓ to browse and Enter to apply; the list windows when more palettes do not fit.'],
    screen: screenAppearance(rows),
  }]
}
