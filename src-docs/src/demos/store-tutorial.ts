import type { TutorialStep } from './first-run-tutorial'
import { ACC, DIM, FG, INVERSE, RST, SEL_BG, SEL_FG, WARN, commandEntryScreen, line80, padRight, trunc } from './chat-chrome'

function screen(rows: number, title: string, body: string[], footer: string): string {
  const lines = [line80(DIM + `  ${title}` + RST), line80(DIM + '─'.repeat(80) + RST), '', ...body.map(line => line80('  ' + trunc(line, 76)))]
  while (lines.length < rows - 1) lines.push('')
  lines.push(INVERSE + padRight(` ${footer}`, 80) + RST)
  return lines.slice(0, rows).join('\n')
}

export function getStoreSteps(rows = 24): TutorialStep[] {
  const browse = screen(rows, 'store', [
    ACC + '› ' + RST + SEL_FG + SEL_BG + 'Sample Workflow                 ' + RST + DIM + '  free  0.1.0    Example catalogue flow       sample' + RST,
    '  ' + FG + 'Sample Notes                    ' + RST + DIM + '  free  0.2.0    Example note integration      sample' + RST,
  ], '↑/↓ pick · Enter detail · Esc close')
  const detail = screen(rows, 'store / Sample Workflow', [
    DIM + 'id            ' + RST + FG + 'sample-workflow' + RST,
    DIM + 'tier          ' + RST + FG + 'free' + RST,
    DIM + 'kind          ' + RST + FG + 'daemon' + RST,
    DIM + 'author        ' + RST + FG + 'sample' + RST,
    DIM + 'contributes   ' + RST + FG + '0 models · 1 panels · 2 tools · 0 sub-agents' + RST,
    DIM + 'requires      ' + RST + DIM + '(none)' + RST,
    DIM + 'versions      ' + RST + FG + '0.1.0' + RST,
    '', FG + 'An illustrative catalogue item used only by this documentation.' + RST,
  ], 'i install · Esc back')
  const install = screen(rows, 'store / Sample Workflow', ['', FG + 'install ' + ACC + "'Sample Workflow'" + FG + '?' + RST], 'y install · n/Esc cancel')
  const oauth = screen(rows, 'store / Sample Workflow', ['', WARN + 'connect koma.run in /settings → OAuth first' + RST], 'Esc back')
  return [
    { title: 'Type /store', narration: 'From chat, type /store and press Enter.', screen: commandEntryScreen(rows, '/store') },
    { title: 'Loading catalogue', narration: 'The command immediately replaces chat with the full-screen store browser and starts its asynchronous public catalogue fetch.', points: ['Browsing does not require an active session or sign-in'], screen: screen(rows, 'store', [DIM + 'loading catalogue…' + RST], '↑/↓ pick · Enter detail · Esc close') },
    { title: 'Browse catalogue', narration: 'After the fetch, use ↑/↓ (or Tab) to move the catalogue cursor and Enter to open its detail. The names, metadata, and status shown here are illustrative sample data, not product catalogue facts.', screen: browse },
    { title: 'Inspect detail', narration: 'Detail shows the selected item’s fetched metadata, contribution counts, requirements, versions, and plain-text description. Press i only for an item that is not already installed.', screen: detail },
    { title: 'Confirm installation', narration: 'With KomaRun OAuth connected, pressing i shows this y/n confirmation. Press y to begin the asynchronous install, or n or Esc to cancel back to detail. The extension name remains illustrative sample data.', screen: install },
    { title: 'Install prerequisite', narration: 'Without a KomaRun OAuth connection, the same install-confirmation state shows this prerequisite instead of accepting y. Connect KomaRun in /settings → OAuth, then return to install.', screen: oauth },
  ]
}
