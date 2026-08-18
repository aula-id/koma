import { RST, ACC, FG, DIM, INVERSE, trunc, padRight, bar } from './chat-chrome'
import type { TutorialStep } from './first-run-tutorial'
import { settingsChatEntry, settingsMenu } from './settings-appearance-tutorial'

const W = 80
const fields = [
  ['Session name', 'docs-demo'], ['Workdir', 'list'], ['Awareness', 'on'], ['Harness', 'on'],
  ['Allowed dirs', 'list'], ['Short-send', 'on'], ['Sliding cache', 'off'], ['Bash shorts', 'on'],
  ['Coding autosave', 'on'], ['Internet mode', 'simple'], ['Mouse capture', 'auto'], ['Max turns', '25'],
]

function screenGeneral(rows: number, state: 'editing' | 'paths'): string {
  const lines = [DIM + '  settings > General' + RST, DIM + bar('─', W) + RST, '']
  for (const [index, [label, value]] of fields.entries()) {
    const selected = state === 'editing' ? index === 0 : index === 4
    const marker = selected ? ACC + '› ' + RST : DIM + '  ' + RST
    const labelStyle = selected ? ACC : DIM
    if (label === 'Workdir' || label === 'Allowed dirs') {
      lines.push(marker + labelStyle + label.padEnd(14) + RST + (selected && state !== 'paths' ? DIM + 'list' + RST : ''))
      if (label === 'Allowed dirs' && state === 'paths') {
        lines.push('  ' + ACC + '› ' + RST + ACC + '/workspace/shared' + RST)
        lines.push('    ' + DIM + '/workspace/vendor/very-long-directory-that-wraps-across-the-detail-width' + RST)
      } else if (label === 'Workdir') lines.push('    ' + DIM + '/workspace/koma' + RST)
      else lines.push('    ' + DIM + '/workspace/shared' + RST)
      continue
    }
    const shown = selected && state === 'editing' ? value + '█' : value
    lines.push(marker + labelStyle + label.padEnd(14) + RST + (selected ? FG : DIM) + shown + RST)
  }
  while (lines.length < rows - 1) lines.push('')
  const hint = state === 'editing' ? ' type to edit · Enter/Esc done' : ' ↑/↓ entry · + add · - remove · Enter edit · Esc done'
  lines.push(INVERSE + padRight(hint, W) + RST)
  return lines.slice(0, rows).map((line) => trunc(line, W)).join('\n')
}

export function getSettingsGeneralSteps(rows = 24): TutorialStep[] {
  return [
    { title: 'Open settings', narration: 'Type /settings in the chat composer and submit it to open the settings overlay.', points: ['Settings starts in the normal chat frame.', 'The compact overlay appears above the composer.'], screen: settingsChatEntry(rows) },
    { title: 'Select General', narration: 'Press 2 or select General from the settings overlay to open the full-screen General page.', points: ['The numbered selection makes the navigation path explicit.', 'Esc from the menu saves settings and closes it.'], screen: settingsMenu(rows, 1) },
    { title: 'General fields', narration: 'General is a full-screen, ordered session-settings form. Each ordinary row is a two-column marker + 14-column label + value layout.', points: ['The fields appear in order from Session name through Max turns.', 'Enter edits text fields or toggles supported values.', 'Workdir and Allowed dirs expand below their label rows rather than occupying a fixed single row.'], screen: screenGeneral(rows, 'editing') },
    { title: 'Managing a path list', narration: 'Selecting a path-list field enters a list-management state. Entries take as many wrapped visual rows as their paths need, so following fields move down and can clip on short terminals.', points: ['The selected list entry has its own › marker.', '+ adds and - removes entries; Enter edits the selected entry.', 'Esc leaves list management and returns to field navigation.'], screen: screenGeneral(rows, 'paths') },
  ]
}
