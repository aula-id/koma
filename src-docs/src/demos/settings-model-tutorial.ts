import { RST, ACC, FG, DIM, SEL_FG, SEL_BG, INVERSE, stripAnsi, trunc, padRight, bar } from './chat-chrome'
import type { TutorialStep } from './first-run-tutorial'
import { settingsChatEntry, settingsMenu } from './settings-appearance-tutorial'

const W = 80
const finish = (lines: string[], rows: number, footer: string) => {
  while (lines.length < rows - 1) lines.push('')
  lines.push(INVERSE + padRight(footer, W) + RST)
  return lines.slice(0, rows).map(line => trunc(line, W)).join('\n')
}
const header = (page = 'Models') => [DIM + `  settings > ${page}` + RST, DIM + bar('─', W) + RST, '']

function screenModelList(rows = 24): string {
  const lines = header()
  lines.push(DIM + '  Model List' + RST)
  lines.push('  ' + SEL_FG + SEL_BG + '[+add global]' + RST + '  ' + ACC + '[+add local]' + RST)
  lines.push(DIM + '  [X]all [ ]local [ ]global' + RST)
  const nameW = 12, roleW = 11, provW = 12, modelW = W - 4 - nameW - roleW - provW - 3
  lines.push(DIM + '  ' + 'Name'.padEnd(nameW) + 'Role'.padEnd(roleW) + 'Model'.padEnd(modelW) + 'Provider'.padEnd(provW) + RST)
  const models = [
    ['* ', 'main', 'main', 'claude-sonnet-4-20250514', 'anthropic'],
    ['* ', 'awareness', 'awareness', 'claude-haiku-3-20240307', 'anthropic'],
    ['  ', 'code-review', 'main', 'gpt-4o', 'openai'],
  ]
  for (const [glyph, name, role, model, provider] of models) {
    const row = padRight(glyph + trunc(name, nameW - 2), nameW) + trunc(role, roleW).padEnd(roleW) + trunc(model, modelW).padEnd(modelW) + trunc(provider, provW)
    lines.push('  ' + DIM + glyph + RST + FG + row.slice(stripAnsi(glyph).length) + RST)
  }
  return finish(lines, rows, ' ↑↓ line · ←→ item · space select · enter open · esc back')
}

function screenModelForm(rows = 24): string {
  const lines = header('Models > Add')
  // The form has no extra top spacer: body_inner starts here at column 2.
  lines.push(ACC + '  Name'.padEnd(12) + RST + FG + 'my-custom-model' + RST)
  lines.push(DIM + '  Provider'.padEnd(12) + RST + DIM + '‹ openrouter ›' + RST)
  // An endpoint-backed provider uses the omnisearch shape: readout, search, rule.
  lines.push(ACC + '  Model'.padEnd(12) + RST + FG + 'openai/gpt-4o-mini' + RST)
  lines.push(' '.repeat(12) + DIM + 'type to search models…█' + RST)
  lines.push(' '.repeat(12) + DIM + bar('─', W - 14) + RST)
  lines.push(DIM + '  Role'.padEnd(12) + RST + DIM + 'main' + RST)
  lines.push('')
  const buttons = '[ Save ]  [ Cancel ]'
  lines.push('  ' + ' '.repeat(Math.floor((W - 4 - buttons.length) / 2)) + ACC + '[ Save ]' + RST + '  ' + ACC + '[ Cancel ]' + RST)
  return finish(lines, rows, ' ↑↓ field · ←→ provider · enter select · esc cancel')
}

export function getSettingsModelSteps(rows = 24): TutorialStep[] {
  return [
    { title: 'Open settings', narration: 'From normal chat, type /settings in the composer and submit it.', points: ['The command opens the compact settings menu above the composer.'], screen: settingsChatEntry(rows) },
    { title: 'Select Models', narration: 'Press 5 or select Models in the five-item settings menu.', points: ['This opens the full-screen Models page.'], screen: settingsMenu(rows, 4) },
    { title: 'Model list', narration: 'Models is a full-screen grid. Select [+add global] or [+add local] with ←→, then press Enter to create the corresponding scope.', points: ['Use ↑↓ to change lines and ←→ to change the control on that line.', 'Space selects an all, local, or global filter; Enter opens the selected model row.'], screen: screenModelList(rows) },
    { title: 'Add model', narration: 'The full-screen form begins at the Settings body inset with no extra spacer. For an endpoint-backed provider, Model shows the selected-model readout above a separate search input.', points: ['Type in the search input to search that provider’s live catalogue, then press Enter to pick a result.', 'Use ←→ while Provider is focused to change providers.', 'Save and Cancel are separated by two spaces.'], screen: screenModelForm(rows) },
  ]
}
