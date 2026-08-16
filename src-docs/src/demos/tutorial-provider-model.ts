import { RST, ACC, FG, DIM, WARN, SEL_FG, SEL_BG, INVERSE, trunc, padRight, bar, chatHeader, chatInput, commandEntryScreen } from './chat-chrome'
import type { TutorialStep } from './first-run-tutorial'

const W = 80
const finish = (lines: string[], rows: number, footer: string) => {
  while (lines.length < rows - 1) lines.push('')
  lines.push(INVERSE + padRight(footer, W) + RST)
  return lines.slice(0, rows).map(line => trunc(line, W)).join('\n')
}
const hdr = (page = 'Providers') => [DIM + `  settings > ${page}` + RST, DIM + bar('─', W) + RST, '']

function settingsMenu(rows: number, selected = 2): string {
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

function screenProviderList(rows = 24): string {
  const lines = hdr()
  const nameW = 14, typeW = 11, keyW = 8, epW = W - 4 - nameW - typeW - keyW - 3
  lines.push(DIM + '  ' + 'Name'.padEnd(nameW) + 'Endpoint'.padEnd(epW) + 'Type'.padEnd(typeW) + 'Key'.padEnd(keyW) + RST)
  while (lines.length < rows - 2) lines.push('')
  lines.push('  ' + SEL_FG + SEL_BG + padRight('[ + add provider ]', W - 4) + RST)
  return finish(lines, rows, ' ↑↓ select · enter open · ctrl+x delete · esc back')
}

function screenProviderForm(rows = 24): string {
  const lines = hdr('Providers > Add')
  for (let i = 0; i < 6; i++) lines.push('')
  const fields = [['Name', 'openai', false], ['Endpoint', 'https://api.openai.com/v1', true], ['API key', '\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022', false]] as const
  for (const [label, value, active] of fields) {
    const shown = active ? value + '\u{2588}' : value
    lines.push('    ' + (active ? ACC : DIM) + label.padEnd(14) + RST + (active ? FG : DIM) + shown + RST)
  }
  lines.push('')
  const buttons = '[ Save ]   [ Cancel ]'
  lines.push('  ' + ' '.repeat(Math.floor((W - 4 - buttons.length) / 2)) + ACC + '[ Save ]' + RST + '   ' + ACC + '[ Cancel ]' + RST)
  return finish(lines, rows, ' ↑↓ field · enter advance · esc back')
}

function screenModelList(rows = 24): string {
  const lines = hdr('Models')
  lines.push(DIM + '  Model List' + RST)
  lines.push('  ' + SEL_FG + SEL_BG + '[+add global]' + RST + '  ' + ACC + '[+add local]' + RST)
  lines.push(DIM + '  [X]all [ ]local [ ]global' + RST)
  const nameW = 12, roleW = 11, provW = 12, modelW = W - 4 - nameW - roleW - provW - 3
  lines.push(DIM + '  ' + 'Name'.padEnd(nameW) + 'Role'.padEnd(roleW) + 'Model'.padEnd(modelW) + 'Provider'.padEnd(provW) + RST)
  while (lines.length < rows - 1) lines.push('')
  return finish(lines, rows, ' ↑↓ line · ←→ item · space select · enter open · esc back')
}

function screenModelForm(rows = 24): string {
  const lines = hdr('Models > Add')
  lines.push(ACC + '  Name'.padEnd(12) + RST + FG + 'my-gpt' + RST)
  lines.push(DIM + '  Provider'.padEnd(12) + RST + DIM + '\u2039 openai \u203a' + RST)
  lines.push(ACC + '  Model'.padEnd(12) + RST + FG + 'gpt-4o' + RST)
  lines.push(' '.repeat(12) + DIM + 'type to search models\u2026\u{2588}' + RST)
  lines.push(' '.repeat(12) + DIM + bar('─', W - 14) + RST)
  lines.push(ACC + '  Role'.padEnd(12) + RST + FG + '(not set)' + RST)
  lines.push('')
  const buttons = '[ Save ]  [ Cancel ]'
  lines.push('  ' + ' '.repeat(Math.floor((W - 4 - buttons.length) / 2)) + ACC + '[ Save ]' + RST + '  ' + ACC + '[ Cancel ]' + RST)
  return finish(lines, rows, ' ↑↓ field · ←→ provider · enter select · esc cancel')
}

function screenRolePicker(rows = 24): string {
  const lines = []
  for (let i = 0; i < rows; i++) lines.push(DIM + ' '.repeat(W) + RST)
  const head = '  settings > Models'
  lines[0] = DIM + head + DIM + ' '.repeat(W - head.length) + RST
  lines[1] = DIM + bar('─', W) + RST
  const w = 30, bx = Math.floor((W - w) / 2), h = 9, by = Math.floor((rows - h) / 2)
  const innerW = w - 2, title = ' roles '
  const padL = ' '.repeat(bx), padR = ' '.repeat(W - bx - w)
  lines[by] = DIM + padL + '\u250c\u2500' + ACC + title + DIM + bar('\u2500', w - 3 - title.length) + '\u2510' + padR + RST
  const roles = ['main', 'awareness', 'planner', 'compactor', 'safeguard']
  roles.forEach((role, i) => {
    if (i === 0) {
      lines[by + 1 + i] = DIM + padL + '\u2502' + RST + INVERSE + padRight(' [x] ' + role, innerW) + RST + DIM + '\u2502' + padR + RST
    } else {
      lines[by + 1 + i] = DIM + padL + '\u2502' + RST + DIM + '[ ] ' + role + ' '.repeat(innerW - role.length - 4) + RST + DIM + '\u2502' + padR + RST
    }
  })
  lines[by + 6] = DIM + padL + '\u2502' + RST + DIM + padRight(' space toggle', innerW) + RST + DIM + '\u2502' + padR + RST
  lines[by + 7] = DIM + padL + '\u2502' + RST + DIM + padRight(' enter ok \u00b7 esc cancel', innerW) + RST + DIM + '\u2502' + padR + RST
  lines[by + 8] = DIM + padL + '\u2514' + bar('\u2500', w - 2) + '\u2518' + padR + RST
  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

function screenModelHelp(rows = 24): string {
  const lines: string[] = []
  lines.push(...chatHeader())
  const overlayLines: string[] = []
  const innerW = W - 2
  const title = ' model \u2014 help '
  overlayLines.push(DIM + '\u250c' + title + bar('\u2500', W - 2 - title.length) + '\u2510' + RST)
  const helpLines = [
    ' /model \u2014 session model switcher', '',
    '  main         claude-sonnet-4-20250514',
    '  awareness    claude-haiku-3-20240307',
    '  planner      (unset)', '  compactor    (unset)', '  safeguard    (unset)', '',
    '  /model <role>            swap role model',
    '  /model agent             pick agent, then model',
  ]
  for (const hl of helpLines) {
    overlayLines.push(DIM + '\u2502' + RST + ACC + hl.padEnd(innerW) + RST + DIM + '\u2502' + RST)
  }
  overlayLines.push(DIM + '\u2502' + RST + DIM + 'Esc close'.padEnd(innerW) + RST + DIM + '\u2502' + RST)
  overlayLines.push(DIM + '\u2514' + bar('\u2500', W - 2) + '\u2518' + RST)
  const inputBar = chatInput('/model')
  const targetStart = rows - inputBar.length - overlayLines.length
  while (lines.length < targetStart) lines.push('')
  lines.push(...overlayLines)
  lines.push(...inputBar)
  while (lines.length < rows) lines.push('')
  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

function screenRolePickerOverlay(rows = 24): string {
  const lines: string[] = []
  lines.push(...chatHeader())
  lines.push('', '  ' + FG + 'what files changed in the last commit?' + RST, '')
  const overlayLines: string[] = []
  const innerW = W - 2
  const title = ' model \u2014 main '
  overlayLines.push(DIM + '\u250c' + title + bar('\u2500', W - 2 - title.length) + '\u2510' + RST)
  const options = [
    { label: '(inherit session default)', concrete: false, sel: false },
    { label: 'koma free \u2014 keyless', concrete: false, sel: false },
    { label: 'gpt-4o \u2014 openai/gpt-4o @ OpenRouter', concrete: true, sel: true },
  ]
  for (const opt of options) {
    const text = ' ' + opt.label + ' '
    if (opt.sel) {
      overlayLines.push(DIM + '\u2502' + RST + INVERSE + padRight(text, innerW) + RST + DIM + '\u2502' + RST)
    } else {
      overlayLines.push(DIM + '\u2502' + RST + (opt.concrete ? ACC : DIM) + text.padEnd(innerW) + RST + DIM + '\u2502' + RST)
    }
  }
  overlayLines.push(DIM + '\u2502' + RST + DIM + '\u2191\u2193 select \u00b7 Enter apply \u00b7 Esc cancel'.padEnd(innerW) + RST + DIM + '\u2502' + RST)
  overlayLines.push(DIM + '\u2514' + bar('\u2500', W - 2) + '\u2518' + RST)
  const inputBar = chatInput('/model main')
  const targetStart = rows - inputBar.length - overlayLines.length
  while (lines.length < targetStart) lines.push('')
  lines.push(...overlayLines)
  lines.push(...inputBar)
  while (lines.length < rows) lines.push('')
  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

export function getTutorialProviderModelSteps(rows = 24): TutorialStep[] {
  return [
    { title: 'Type /settings', narration: 'From normal chat, type /settings in the composer and submit it.', screen: commandEntryScreen(rows, '/settings') },
    { title: 'Select Providers', narration: 'Press 3 or select Providers in the five-item settings menu.', points: ['The compact menu is anchored above the composer.'], screen: settingsMenu(rows, 2) },
    { title: 'Provider list', narration: 'The Providers page shows a table of saved API connections. Move to [ + add provider ] and press Enter.', points: ['API keys are masked.', 'Ctrl+X deletes a selected provider.'], screen: screenProviderList(rows) },
    { title: 'Add provider', narration: 'Fill in Name, Endpoint, and API key. Enter advances between fields. Choose Save to persist.', points: ['A manual provider is always OpenAI-compatible wire.', 'Esc returns to the list without saving.'], screen: screenProviderForm(rows) },
    { title: 'Open Models', narration: 'Press Esc to return to the settings menu, then press 5 or select Models.', points: ['You can also reopen /settings from chat and press 5 directly.'], screen: settingsMenu(rows, 4) },
    { title: 'Model list', narration: 'Models is a full-screen grid. Select [+add global] or [+add local], then press Enter.', points: ['Global models are persisted; local models are session-only.'], screen: screenModelList(rows) },
    { title: 'Add model', narration: 'Pick a Provider with ←→, then type in the Model search to find a model from the catalogue. Assign a Role.', points: ['Type to filter the live catalogue, then Enter to pick.', 'The Role field opens a checkbox picker.'], screen: screenModelForm(rows) },
    { title: 'Assign a role', narration: 'In the Role field, press Enter to open the role picker. Space toggles a role, Enter confirms.', points: ['Roles: main, awareness, planner, compactor, safeguard.', 'The Main role is the primary coding model.'], screen: screenRolePicker(rows) },
    { title: 'Type /model', narration: 'From chat, type /model to see your current role assignments and available sub-commands.', points: ['/model shows a compact overlay above the input bar.'], screen: screenModelHelp(rows) },
    { title: 'Switch with /model', narration: 'Type /model main to pick a different model for the main role. Select from the list and press Enter.', points: ['Inherit removes the session override.', 'Esc cancels without changing.'], screen: screenRolePickerOverlay(rows) },
  ]
}
