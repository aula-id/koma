import { RST, ACC, FG, DIM, SEL_FG, SEL_BG, INVERSE, trunc, padRight, bar } from './chat-chrome'
import type { TutorialStep } from './first-run-tutorial'
import { settingsChatEntry, settingsMenu } from './settings-appearance-tutorial'

const W = 80
const finish = (lines: string[], rows: number, footer: string) => {
  while (lines.length < rows - 1) lines.push('')
  lines.push(INVERSE + padRight(footer, W) + RST)
  return lines.slice(0, rows).map(line => trunc(line, W)).join('\n')
}
const header = (page = 'Providers') => [DIM + `  settings > ${page}` + RST, DIM + bar('─', W) + RST, '']

function screenProviderTable(rows = 24): string {
  const lines = header()
  const nameW = 14, typeW = 11, keyW = 8, epW = W - 4 - nameW - typeW - keyW - 3
  lines.push(DIM + '  ' + 'Name'.padEnd(nameW) + 'Endpoint'.padEnd(epW) + 'Type'.padEnd(typeW) + 'Key'.padEnd(keyW) + RST)
  const providers = [
    { name: 'openai', endpoint: 'https://api.openai.com/v1', type: 'openai', selected: false },
    { name: 'anthropic', endpoint: 'https://api.anthropic.com', type: 'openai', selected: false },
    { name: 'google', endpoint: 'https://generativelanguage.', type: 'google', selected: false },
  ]
  for (const provider of providers) {
    const row = provider.name.padEnd(nameW) + trunc(provider.endpoint, epW).padEnd(epW) + provider.type.padEnd(typeW) + '••••••'
    lines.push('  ' + (provider.selected ? SEL_FG + SEL_BG + padRight(row, W - 4) + RST : FG + row + RST))
  }
  // The renderer reserves the last body row for this control; it is selected here.
  while (lines.length < rows - 2) lines.push('')
  lines.push('  ' + SEL_FG + SEL_BG + padRight('[ + add provider ]', W - 4) + RST)
  return finish(lines, rows, ' ↑↓ select · enter open · ctrl+x delete · esc back')
}

function screenProviderForm(rows = 24): string {
  const lines = header('Providers > Add')
  for (let i = 0; i < 6; i++) lines.push('')
  const fields = [
    ['Name', 'anthropic', false],
    ['Endpoint', 'https://api.anthropic.com', true],
    ['API key', '••••••••••••', false],
  ] as const
  for (const [label, value, active] of fields) {
    const shown = active ? value + '█' : value
    lines.push('    ' + (active ? ACC : DIM) + label.padEnd(14) + RST + (active ? FG : DIM) + shown + RST)
  }
  lines.push('')
  const buttons = '[ Save ]   [ Cancel ]'
  lines.push('  ' + ' '.repeat(Math.floor((W - 4 - buttons.length) / 2)) + ACC + '[ Save ]' + RST + '   ' + ACC + '[ Cancel ]' + RST)
  return finish(lines, rows, ' ↑↓ field · enter advance · esc back')
}

export function getSettingsProviderSteps(rows = 24): TutorialStep[] {
  return [
    { title: 'Open settings', narration: 'From a normal chat screen, type /settings in the composer and submit it.', points: ['Settings starts as a compact overlay above the normal composer.'], screen: settingsChatEntry(rows) },
    { title: 'Select Providers', narration: 'In the five-item settings menu, press 3 or select Providers.', points: ['The compact menu is anchored directly above the composer.', 'This opens the full-screen Providers page.'], screen: settingsMenu(rows, 2) },
    { title: 'Provider list', narration: 'Providers is a full-screen table of saved API connections. Move to the final “[ + add provider ]” row and press Enter to open the form.', points: ['Enter adds only from the final add row; existing provider rows cannot be opened for editing from this screen.', 'Ctrl+X arms deletion for the selected provider; press it again to confirm.', 'API keys are rendered as six masked bullets.'], screen: screenProviderTable(rows) },
    { title: 'Add provider', narration: 'The full-screen form begins after its capped one-third body inset. Enter advances through Name, Endpoint, API key, Save, and Cancel.', points: ['The active field has an accent label and cursor.', 'Choose Save to persist the provider; Esc returns to the list.'], screen: screenProviderForm(rows) },
  ]
}
