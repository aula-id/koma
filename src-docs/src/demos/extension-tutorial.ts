import type { TutorialStep } from './first-run-tutorial'
import { ACC, DIM, FG, INVERSE, RST, SEL_BG, SEL_FG, WARN, commandEntryScreen, line80, padRight, trunc } from './chat-chrome'

function screen(rows: number, title: string, body: string[], footer: string): string {
  const lines = [line80(DIM + `  ${title}` + RST), line80(DIM + '─'.repeat(80) + RST), '', ...body.map(line => line80('  ' + trunc(line, 76)))]
  while (lines.length < rows - 1) lines.push('')
  lines.push(INVERSE + padRight(` ${footer}`, 80) + RST)
  return lines.slice(0, rows).join('\n')
}

export function getExtensionSteps(rows = 24): TutorialStep[] {
  const browse = screen(rows, 'extensions', [
    ACC + '› ' + RST + SEL_FG + SEL_BG + 'Sample Workflow                 ' + RST + DIM + '  0.1.0    daemon   free  ' + RST + ACC + '● running' + RST,
    '  ' + FG + 'Sample Notes                    ' + RST + DIM + '  0.2.0    daemon   free  ' + RST + DIM + '○ stopped  ' + RST + WARN + '(disabled)' + RST,
  ], '↑/↓ pick · →/Enter detail · Esc close')
  const detail = screen(rows, 'extensions / Sample Workflow', [
    DIM + 'id            ' + RST + FG + 'sample-workflow' + RST,
    DIM + 'description   ' + RST + FG + 'Illustrative installed-extension example.' + RST,
    DIM + 'version       ' + RST + FG + '0.1.0' + RST,
    DIM + 'tier          ' + RST + FG + 'free' + RST,
    DIM + 'kind          ' + RST + FG + 'daemon' + RST,
    DIM + 'enabled       ' + RST + FG + 'yes' + RST,
    DIM + 'running       ' + RST + ACC + '● running' + RST,
    DIM + 'contributes   ' + RST + FG + '2 tools · 1 panels · 0 sub-agents · 0 models' + RST,
    DIM + 'granted       ' + RST + DIM + '(none)' + RST,
    DIM + 'workspace     ' + RST + DIM + '(none)' + RST,
  ], 'u uninstall · Esc back')
  const uninstall = screen(rows, 'extensions / Sample Workflow', [
    '', FG + 'uninstall ' + ACC + "'Sample Workflow'" + FG + '?' + RST,
    DIM + 'removes the package from disk, deregisters its tools/models, and drops its config entry' + RST,
  ], 'y uninstall · n/Esc cancel')
  return [
    { title: 'Type /extension', narration: 'From chat, type /extension and press Enter.', screen: commandEntryScreen(rows, '/extension') },
    { title: 'Browse installed extensions', narration: 'The command opens the full-screen installed-extension manager. Use ↑/↓ (or Tab) to choose an installed row; → or Enter opens it. Every name, version, tier, and runtime status in this tutorial is illustrative sample data, not a claim about installed product extensions.', screen: browse },
    { title: 'Inspect detail', narration: 'The detail view renders registry and manifest information, contribution counts, permissions, workspace data, and any selectable extension screens. u arms removal; Esc returns to the browse list.', screen: detail },
    { title: 'Confirm uninstall', narration: 'The real confirmation names the selected extension and explains that removal deletes its package, deregisters tools and models, and drops its configuration entry. Press y to uninstall; n or Esc cancels back to detail.', screen: uninstall },
  ]
}
