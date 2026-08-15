/**
 * Tutorial screens for /settings → Providers.
 *
 * Step 1: the settings MENU is a compact overlay above the composer
 *         (dim border, " settings " title, anchored above input bar).
 * Steps 2-3: Provider TABLE and FORM are fullscreen pages with
 *            breadcrumb header + inverse footer.
 *
 * Layouts match src-agent/src/view/settings/{mod.rs, pages/menu.rs,
 * pages/providers.rs, pages/provider_form.rs} exactly.
 */

const RST = '\x1b[0m'
const ACC = '\x1b[32m'
const FG  = '\x1b[37m'
const DIM = '\x1b[90m'
const SEL_FG = '\x1b[30m'
const SEL_BG = '\x1b[42m'
const INVERSE = SEL_FG + SEL_BG + '\x1b[1m'

function stripAnsi(s: string): string {
  return s.replace(/\x1b\[[0-9;]*m/g, '')
}

function trunc(line: string, w: number): string {
  let vis = 0
  let out = ''
  const re = /\x1b\[[0-9;]*m/g
  let last = 0
  let m: RegExpExecArray | null
  while ((m = re.exec(line)) !== null) {
    const text = line.slice(last, m.index)
    for (const ch of text) {
      if (vis >= w) return out + RST
      out += ch
      vis++
    }
    out += m[0]
    last = re.lastIndex
  }
  const tail = line.slice(last)
  for (const ch of tail) {
    if (vis >= w) return out + RST
    out += ch
    vis++
  }
  return out
}

function padRight(text: string, w: number): string {
  const vis = stripAnsi(text).length
  return text + ' '.repeat(Math.max(0, w - vis))
}

function bar(ch: string, w: number): string {
  return ch.repeat(w)
}

// ─── Shared: Chat chrome (header + input bar) ─────────────────────────

function chatHeader(): string[] {
  const W = 80
  const brand = DIM + 'koma' + RST + ' ' + ACC + '0.3.16' + RST
  const mode = ACC + '\u25cf normal' + RST
  const gap = Math.max(1, W - 4 - stripAnsi(brand).length - stripAnsi(mode).length)
  return [
    '  ' + brand + ' '.repeat(gap) + mode,
    DIM + bar('\u2500', W) + RST,
  ]
}

function chatInput(text: string): string[] {
  const W = 80
  return [
    DIM + bar('\u2500', W) + RST,
    '  ' + ACC + '[$] ' + RST + ACC + text + '\u{2588}' + RST,
    DIM + bar('\u2500', W) + RST,
  ]
}

// ─── Screen: Settings Menu (compact overlay above composer) ───────────
// Matches render_menu_overlay() in mod.rs: dim border, " settings "
// title, horizontal(1) padding, full width of input area.

function screenSettingsMenu(rows = 24): string {
  const W = 80
  const lines: string[] = []

  // Chat header
  lines.push(...chatHeader())

  // Minimal transcript — overlay covers most of the screen
  lines.push('')
  lines.push('  ' + FG + 'what files changed in the last commit?' + RST)
  lines.push('')
  lines.push('  ' + FG + 'Let me check the recent git history for you.' + RST)

  // /settings overlay — compact popup above input bar
  // Border: dim. Title: " settings " in dim. Full width.
  const overlayLines: string[] = []
  const title = ' settings '
  overlayLines.push(DIM + '\u250c' + title + bar('\u2500', W - 2 - title.length) + '\u2510' + RST)
  overlayLines.push(DIM + '\u2502' + RST + ' '.repeat(W - 2) + DIM + '\u2502' + RST)

  const items = [
    { num: 1, label: 'Appearance', sel: false },
    { num: 2, label: 'General', sel: false },
    { num: 3, label: 'Providers', sel: true },
    { num: 4, label: 'OAuth', sel: false },
    { num: 5, label: 'Models', sel: false },
  ]

  const innerW = W - 2
  for (const item of items) {
    const chip = `[${item.num}]`
    const text = `  ${item.label}`
    if (item.sel) {
      const content = ` ${chip}${text}`
      const padded = padRight(content, innerW)
      overlayLines.push(DIM + '\u2502' + RST + INVERSE + padded + RST + DIM + '\u2502' + RST)
    } else {
      const content = ` ${chip}${text}`
      overlayLines.push(DIM + '\u2502' + RST + ACC + chip + RST + FG + text + RST + ' '.repeat(Math.max(0, innerW - stripAnsi(content).length)) + DIM + '\u2502' + RST)
    }
  }

  overlayLines.push(DIM + '\u2502' + RST + ' '.repeat(innerW) + DIM + '\u2502' + RST)
  overlayLines.push(DIM + '\u2514' + bar('\u2500', W - 2) + '\u2518' + RST)

  // Place overlay right above the input bar
  const inputBar = chatInput('/settings')
  const targetStart = rows - inputBar.length - overlayLines.length
  while (lines.length < targetStart) lines.push('')
  lines.push(...overlayLines)
  lines.push(...inputBar)

  while (lines.length < rows) lines.push('')
  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

// ─── Screen: Provider Table (fullscreen page) ─────────────────────────
// Matches pages/providers.rs: breadcrumb header, no box borders,
// column headers in dim, data rows, [ + add provider ] button.

function screenProviderTable(rows = 24): string {
  const W = 80
  const lines: string[] = []

  // Breadcrumb header (2 rows)
  lines.push(DIM + '  settings > Providers' + RST)
  lines.push(DIM + bar('\u2500', W) + RST)

  // Column header (dim)
  const nameW = 14
  const typeW = 11
  const keyW = 8
  const epW = W - nameW - typeW - keyW - 3
  const hdr = 'Name'.padEnd(nameW) + 'Endpoint'.padEnd(epW) + 'Type'.padEnd(typeW) + 'Key'.padEnd(keyW)
  lines.push(DIM + hdr + RST)

  // Data rows
  const providers = [
    { name: 'openai', ep: 'https://api.openai.com/v1', type: 'openai', key: true, sel: false },
    { name: 'anthropic', ep: 'https://api.anthropic.com', type: 'openai', key: true, sel: true },
    { name: 'google', ep: 'https://generativelanguage.', type: 'google', key: true, sel: false },
  ]

  const bullet = '\u2022\u2022\u2022\u2022\u2022\u2022'
  for (const p of providers) {
    const nameCol = trunc(p.name.padEnd(nameW), nameW)
    const epCol = trunc(p.ep, epW)
    const typeCol = p.type.padEnd(typeW)
    const keyCol = p.key ? bullet : '\u2014'
    const full = nameCol + epCol + typeCol + keyCol

    if (p.sel) {
      lines.push(SEL_FG + SEL_BG + padRight(full, W) + RST)
    } else {
      lines.push(FG + full + RST)
    }
  }

  // Add button
  lines.push('')
  lines.push(ACC + '[ + add provider ]' + RST)

  // Footer — inverse bar
  while (lines.length < rows - 1) lines.push('')
  const footerText = ' \u2191\u2193 select \u00b7 a add \u00b7 ctrl+x delete \u00b7 esc back'
  lines.push(INVERSE + padRight(footerText, W) + RST)

  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

// ─── Screen: Add Provider Form (fullscreen page) ──────────────────────
// Matches pages/provider_form.rs: breadcrumb header, label(14)+value
// fields, [Save]/[Cancel] buttons, inverse footer.

function screenProviderForm(rows = 24): string {
  const W = 80
  const LABEL_W = 14
  const lines: string[] = []

  // Breadcrumb header
  lines.push(DIM + '  settings > Providers > Add' + RST)
  lines.push(DIM + bar('\u2500', W) + RST)

  // Top spacer
  const topSpacer = Math.max(1, Math.floor(rows * 0.35))
  for (let i = 0; i < topSpacer; i++) lines.push('')

  // Fields
  const fields = [
    { label: 'Name', value: 'anthropic', active: false },
    { label: 'Endpoint', value: 'https://api.anthropic.com', active: true },
    { label: 'API key', value: '\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022', active: false },
  ]

  for (const f of fields) {
    const lc = f.active ? ACC : DIM
    const vc = f.active ? FG : DIM
    const valW = W - 4 - LABEL_W
    let val = trunc(f.value, valW - 1)
    if (f.active) val += '\u{2588}'
    const label = lc + f.label.padEnd(LABEL_W) + RST
    const value = vc + val + RST
    lines.push('  ' + label + value)
  }

  // Blank line
  lines.push('')

  // Buttons centered
  const saveText = '[ Save ]'
  const cancelText = '[ Cancel ]'
  const gap = '   '
  const groupLen = saveText.length + gap.length + cancelText.length
  const padLeft = Math.floor((W - 4 - groupLen) / 2)
  lines.push(' '.repeat(padLeft + 2) + ACC + saveText + RST + gap + ACC + cancelText + RST)

  // Footer — inverse bar
  while (lines.length < rows - 1) lines.push('')
  const footerText = ' \u2191\u2193 field \u00b7 enter advance \u00b7 esc back'
  lines.push(INVERSE + padRight(footerText, W) + RST)

  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

// ─── Tutorial Steps ───────────────────────────────────────────────────

import type { TutorialStep } from './first-run-tutorial'

export function getSettingsProviderSteps(rows = 24): TutorialStep[] {
  return [
    {
      title: 'Settings Menu',
      narration:
        'Type /settings in the chat to open the settings menu. It appears as a compact overlay above the input bar with five categories.',
      points: [
        'Press 1-5 to jump to a category, or Esc to close',
        'Providers manages your custom API connections (endpoint + key)',
        'OAuth manages provider sign-ins via browser authentication',
      ],
      screen: screenSettingsMenu(rows),
    },
    {
      title: 'Provider List',
      narration:
        'The Providers page shows all configured API providers in a table with name, endpoint, type, and masked key. Each row is a saved connection you can edit or delete.',
      points: [
        'Navigate with \u2191\u2193 and press Enter or \'a\' to add a new provider',
        'Press Ctrl+X twice to delete the selected provider',
        'Keys are masked (\u2022\u2022\u2022\u2022\u2022\u2022) for security',
      ],
      screen: screenProviderTable(rows),
    },
    {
      title: 'Add Provider Form',
      narration:
        'The add-provider form has three fields: a friendly name, the API endpoint URL, and your API key. Use Tab or Enter to advance between fields.',
      points: [
        'The active field shows in green with a blinking cursor',
        'Inactive fields appear dimmed',
        'Press Save to persist the provider \u2014 it becomes available for model configuration',
      ],
      screen: screenProviderForm(rows),
    },
  ]
}
