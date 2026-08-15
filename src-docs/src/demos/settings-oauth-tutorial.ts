import { RST, ACC, FG, DIM, SEL_FG, SEL_BG, INVERSE, trunc, padRight, bar } from './chat-chrome'
import type { TutorialStep } from './first-run-tutorial'
import { settingsChatEntry, settingsMenu } from './settings-appearance-tutorial'

const W = 80
const finish = (lines: string[], rows: number, footer: string) => {
  while (lines.length < rows - 1) lines.push('')
  lines.push(INVERSE + padRight(footer, W) + RST)
  return lines.slice(0, rows).map(line => trunc(line, W)).join('\n')
}
const header = () => [DIM + '  settings > OAuth' + RST, DIM + bar('─', W) + RST, '']

function screenOAuthList(rows = 24): string {
  const lines = header()
  const provW = 12, statusW = 16, acctW = W - 4 - provW - statusW - 2
  lines.push(DIM + '  ' + 'Provider'.padEnd(provW) + 'Account'.padEnd(acctW) + 'Status'.padEnd(statusW) + RST)
  for (const conn of [
    ['koma.run', 'alice@koma.run', 'active'],
    ['kilo code', 'bob@org.dev', 'renews in 5d'],
  ]) lines.push('  ' + FG + conn[0].padEnd(provW) + conn[1].padEnd(acctW) + conn[2].padEnd(statusW) + RST)
  while (lines.length < rows - 2) lines.push('')
  lines.push('  ' + SEL_FG + SEL_BG + padRight('[ + connect ]', W - 4) + RST)
  return finish(lines, rows, ' ↑↓ select · enter connect · ctrl+x delete · esc back')
}

function screenOAuthPicker(rows = 24): string {
  // The flow clears `body`, not the settings header: rows start at body top, x=0.
  const lines = [DIM + '  settings > OAuth' + RST, DIM + bar('─', W) + RST]
  const providers = ['Codex', 'Kilo Code', 'koma.run', 'xAI', 'Claude', 'Command Code', 'Codex (paste token)', 'Command Code (paste key)']
  for (const [i, label] of providers.entries()) {
    lines.push(i === 2 ? SEL_FG + SEL_BG + padRight('› ' + label, W) + RST : ACC + '  ' + RST + FG + label + RST)
  }
  return finish(lines, rows, ' ↑↓ select · enter choose · esc back')
}

function screenOAuthWait(rows = 24): string {
  // This is the post-`c` state, so the confirmation is intentionally present.
  const lines = [
    DIM + '  settings > OAuth' + RST,
    DIM + bar('─', W) + RST,
    ACC + 'koma.run login' + RST,
    '',
    DIM + 'https://auth.koma.run/authorize?client_id=koma&scope=openid+profile' + RST,
    DIM + 'url copied to clipboard' + RST,
  ]
  return finish(lines, rows, ' c copy url · o open browser · esc cancel')
}

export function getSettingsOAuthSteps(rows = 24): TutorialStep[] {
  return [
    { title: 'Open settings', narration: 'From normal chat, type /settings in the composer and submit it.', points: ['The command opens the compact settings menu above the composer.'], screen: settingsChatEntry(rows) },
    { title: 'Select OAuth', narration: 'Press 4 or select OAuth in the five-item settings menu.', points: ['This opens the full-screen OAuth page.'], screen: settingsMenu(rows, 3) },
    { title: 'OAuth connections', narration: 'The full-screen OAuth page lists linked accounts. Move to “[ + connect ]” and press Enter to choose a provider.', points: ['Ctrl+X twice disconnects the selected connection.', 'The final connect row is a distinct control.'], screen: screenOAuthList(rows) },
    { title: 'Choose a provider', narration: 'The provider picker overlays the OAuth body. The settings > OAuth breadcrumb and footer remain visible, while the list starts at the top of the body.', points: ['Use ↑↓ to select a provider and Enter to choose it.', 'Esc closes the picker and returns to the connections list.'], screen: screenOAuthPicker(rows) },
    { title: 'Complete browser sign-in', narration: 'After choosing koma.run, the OAuth body shows its login URL while the Settings header remains in place. Press c to copy that URL, or o to open it.', points: ['“url copied to clipboard” appears only after c successfully copies the URL.', 'Esc cancels the waiting flow.'], screen: screenOAuthWait(rows) },
  ]
}
