/**
 * Tutorial screens for /settings → Models (Add Model).
 *
 * Two screens: the model list with filters, and the add-model form.
 */

const RST = '\x1b[0m'
const ACC = '\x1b[32m'
const FG  = '\x1b[37m'
const DIM = '\x1b[90m'
const SEL_FG = '\x1b[30m'
const SEL_BG = '\x1b[42m'

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

// ─── Screen: Model List ───────────────────────────────────────────────

function screenModelList(rows = 24): string {
  const W = 80
  const lines: string[] = []

  // Breadcrumb
  lines.push(DIM + '  settings > Models' + RST)
  lines.push(DIM + bar('\u2500', W) + RST)

  // Title
  lines.push(DIM + 'Model List' + RST)

  // Add buttons
  lines.push(ACC + '[+add global]' + RST + '  ' + ACC + '[+add local]' + RST)

  // Filter radio
  lines.push(DIM + '[X]all  [ ]local  [ ]global' + RST)

  // Column header
  const nameW = 12
  const roleW = 11
  const provW = 12
  const modelW = W - nameW - roleW - provW - 3
  const hdr = 'Name'.padEnd(nameW) + 'Role'.padEnd(roleW) + 'Model'.padEnd(modelW) + 'Provider'.padEnd(provW)
  lines.push(DIM + hdr + RST)

  // Data rows
  const models = [
    { glyph: '* ', name: 'main', role: 'main', model: 'claude-sonnet-4-20250514', prov: 'anthropic', sel: true },
    { glyph: '* ', name: 'awareness', role: 'awareness', model: 'claude-haiku-3-20240307', prov: 'anthropic', sel: false },
    { glyph: '  ', name: 'code-review', role: 'main', model: 'gpt-4o', prov: 'openai', sel: false },
  ]

  for (const m of models) {
    const nameText = trunc(m.name, nameW - 2)
    const nameCol = m.glyph + nameText
    const roleCol = trunc(m.role, roleW)
    const modelCol = trunc(m.model, modelW)
    const provCol = trunc(m.prov, provW)
    const full = padRight(nameCol, nameW) + roleCol + modelCol + provCol

    if (m.sel) {
      lines.push(SEL_FG + SEL_BG + padRight(full, W) + RST)
    } else {
      const glyphPart = DIM + m.glyph + RST
      const namePart = FG + nameText + RST
      lines.push(glyphPart + namePart + ' '.repeat(nameW - m.glyph.length - stripAnsi(nameText).length) + FG + roleCol + ' ' + modelCol + ' ' + provCol + RST)
    }
  }

  // Footer — inverse bar
  const INVERSE = SEL_FG + SEL_BG + '\x1b[1m'
  while (lines.length < rows - 1) lines.push('')
  const footerText = ' \u2191\u2193 line \u00b7 \u2190\u2192 item \u00b7 space select \u00b7 enter open \u00b7 a add \u00b7 esc back'
  lines.push(INVERSE + padRight(footerText, W) + RST)

  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

// ─── Screen: Add Model Form ───────────────────────────────────────────

function screenModelForm(rows = 24): string {
  const W = 80
  const LABEL_W = 10
  const lines: string[] = []

  // Breadcrumb
  lines.push(DIM + '  settings > Models > Add' + RST)
  lines.push(DIM + bar('\u2500', W) + RST)

  // Top spacer
  const topSpacer = Math.max(1, Math.floor(rows * 0.20))
  for (let i = 0; i < topSpacer; i++) lines.push('')

  // Name field (active)
  const nameVal = trunc('my-custom-model', W - 4 - LABEL_W - 1)
  lines.push('  ' + ACC + 'Name'.padEnd(LABEL_W) + RST + FG + nameVal + '\u{2588}' + RST)

  // Provider toggle
  lines.push('  ' + ACC + 'Provider'.padEnd(LABEL_W) + RST + ACC + '\u2039 anthropic \u203a' + RST)

  // Model field (active, with search line + rule)
  lines.push('  ' + DIM + 'Model'.padEnd(LABEL_W) + RST + DIM + 'claude-sonnet-4-20250514' + RST)
  lines.push('  ' + DIM + ' '.repeat(LABEL_W) + bar('\u2500', W - 4 - LABEL_W) + RST)

  // Role
  lines.push('  ' + DIM + 'Role'.padEnd(LABEL_W) + RST + DIM + 'main' + RST)

  // Blank
  lines.push('')

  // Buttons centered
  const saveText = '[ Save ]'
  const cancelText = '[ Cancel ]'
  const gap = '   '
  const groupLen = saveText.length + gap.length + cancelText.length
  const padLeft = Math.floor((W - 4 - groupLen) / 2)
  lines.push(' '.repeat(padLeft + 2) + ACC + saveText + RST + gap + ACC + cancelText + RST)

  // Footer — inverse bar
  const INVERSE = SEL_FG + SEL_BG + '\x1b[1m'
  while (lines.length < rows - 1) lines.push('')
  const footerText = ' \u2191\u2193 field \u00b7 enter select \u00b7 esc back'
  lines.push(INVERSE + padRight(footerText, W) + RST)

  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

// ─── Tutorial Steps ───────────────────────────────────────────────────

import type { TutorialStep } from './first-run-tutorial'

export function getSettingsModelSteps(rows = 24): TutorialStep[] {
  return [
    {
      title: 'Model List',
      narration:
        'The Models page shows all configured models in a table. Global models (available across all sessions) are marked with *, while local models are session-only.',
      points: [
        'Use the filter radio ([X]all / [ ]local / [ ]global) to narrow the list',
        'Add global models with [+add global] or local models with [+add local]',
        'Each model entry tracks its name, role assignment, model ID, and provider',
      ],
      screen: screenModelList(rows),
    },
    {
      title: 'Add Model Form',
      narration:
        'The model form lets you configure a new model entry. Pick a provider with the toggle, then search for a model ID or type one manually.',
      points: [
        'Name is a friendly label for this model entry',
        'Provider cycles through your configured providers with \u2190\u2192',
        'For omnisearchable providers, typing in the Model field searches the live catalogue',
        'Role determines how the model is used: main, awareness, planner, or compactor',
      ],
      screen: screenModelForm(rows),
    },
  ]
}
