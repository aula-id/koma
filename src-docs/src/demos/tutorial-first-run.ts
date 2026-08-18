import { RST, ACC, FG, DIM, WARN, SEL_FG, SEL_BG, INVERSE, trunc, padRight, bar, chatHeader, chatInput } from './chat-chrome'
import type { TutorialStep } from './first-run-tutorial'

const W = 80

function screenChooser(rows = 24): string {
  const BLOCK_W = 64, BX = Math.floor((W - BLOCK_W) / 2), INDENT = ' '.repeat(BX + 2)
  const lines: string[] = []
  const topSpacer = Math.max(1, Math.floor(rows * 0.25))
  for (let i = 0; i < topSpacer; i++) lines.push('')
  lines.push(INDENT + ACC + 'koma' + RST, '', '')
  lines.push(INDENT + DIM + 'how do you want to connect?' + RST, '')
  const OPTS = [
    { label: 'koma free', desc: 'start now, no key - free models hosted by koma', sel: false },
    { label: 'provider', desc: 'sign in to a provider account', sel: true },
    { label: 'custom', desc: 'your own endpoint + API key', sel: false },
  ]
  for (const opt of OPTS) {
    const prefix = opt.sel ? ACC + '> ' : '  '
    lines.push(INDENT + prefix + (opt.sel ? ACC : FG) + opt.label.padEnd(14) + RST + DIM + opt.desc + RST)
  }
  lines.push('', '')
  const bxPad = ' '.repeat(BX), BORDER = BLOCK_W - 2, CONTENT = BLOCK_W - 4
  lines.push(bxPad + WARN + '\u250c' + bar('\u2500', BORDER) + '\u2510' + RST)
  lines.push(bxPad + WARN + '\u2502 ' + 'you can change this anytime in /settings'.padEnd(CONTENT) + ' \u2502' + RST)
  lines.push(bxPad + WARN + '\u2502 ' + 'or type /free to switch to the free tier later'.padEnd(CONTENT) + ' \u2502' + RST)
  lines.push(bxPad + WARN + '\u2514' + bar('\u2500', BORDER) + '\u2518' + RST)
  lines.push('')
  lines.push(INDENT + DIM + 'up/down move \u00b7 enter select \u00b7 q quit' + RST)
  while (lines.length < rows) lines.push('')
  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

function screenOnboardOAuthPicker(rows = 24): string {
  const lines = [DIM + '  sign in to a provider' + RST, DIM + bar('\u2500', W) + RST]
  const providers = ['Codex', 'Kilo Code', 'koma.run', 'xAI', 'Claude', 'Command Code']
  for (const [i, label] of providers.entries()) {
    lines.push(i === 2
      ? SEL_FG + SEL_BG + padRight('\u203a ' + label, W) + RST
      : ACC + '  ' + RST + FG + label + RST)
  }
  while (lines.length < rows - 1) lines.push('')
  lines.push(INVERSE + padRight(' \u2191\u2193 select \u00b7 enter choose \u00b7 esc back', W) + RST)
  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

function screenOnboardOAuthWait(rows = 24): string {
  const lines = [DIM + '  sign in to a provider' + RST, DIM + bar('\u2500', W) + RST]
  lines.push(ACC + 'koma.run login' + RST, '')
  lines.push(DIM + 'https://auth.koma.run/authorize?client_id=koma&scope=openid+profile' + RST)
  while (lines.length < rows - 1) lines.push('')
  lines.push(INVERSE + padRight(' c copy url \u00b7 o open browser \u00b7 esc cancel', W) + RST)
  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

function screenModelSelect(rows = 24): string {
  const lines = [DIM + '  select a model' + RST, DIM + bar('\u2500', W) + RST]
  lines.push('  search: claude' + ACC + '\u{2588}' + RST)
  lines.push(DIM + '  \u2500'.padEnd(W - 2) + RST, '')
  const models = ['claude-sonnet-4-20250514', 'claude-haiku-3-20240307', 'claude-3-5-haiku-20241022']
  for (const [i, m] of models.entries()) {
    lines.push(i === 0
      ? '  ' + SEL_FG + SEL_BG + padRight('\u203a ' + m, W - 4) + RST
      : '  ' + FG + '  ' + m + RST)
  }
  while (lines.length < rows - 1) lines.push('')
  lines.push(INVERSE + padRight(' \u2191\u2193 pick \u00b7 enter select \u00b7 type to filter', W) + RST)
  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

function screenChatWelcome(rows = 24): string {
  const PAD = '  ', MODEL = 'claude-sonnet-4-20250514'
  const lines: string[] = []
  const brand = DIM + 'koma' + RST + ' '
  const verStr = ACC + '0.3.16' + RST
  const modeStr = ACC + '\u{25cf} normal' + RST
  const headerInnerW = W - 6
  const gap = Math.max(1, headerInnerW - 'koma 0.3.16'.length - '\u{25cf} normal'.length)
  lines.push(PAD + brand + verStr + ' '.repeat(gap) + modeStr)
  lines.push(DIM + bar('\u2500', W) + RST)
  lines.push('')
  lines.push(PAD + FG + '\u{25cf}' + RST + ' ' + FG + 'welcome! i\'m koma, your coding agent.' + RST)
  lines.push(PAD + '  ' + DIM + 'i read your code, plan changes, edit files,' + RST)
  lines.push(PAD + '  ' + DIM + 'run commands, and verify everything works.' + RST)
  lines.push('')
  lines.push(PAD + FG + '\u{25cf}' + RST + ' ' + FG + 'try typing a task below \u2014 i\'ll get to work.' + RST)
  const reservedBelow = 1 + 3 + 1
  while (lines.length < rows - reservedBelow) lines.push('')
  const modelAreaW = W - 4
  const visModel = DIM + MODEL + RST
  lines.push(PAD + ' '.repeat(Math.max(0, modelAreaW - MODEL.length)) + visModel + PAD)
  lines.push(DIM + bar('\u2500', W) + RST)
  lines.push(PAD + ACC + '[$] ' + RST + ACC + '\u{2588}' + RST)
  lines.push(DIM + bar('\u2500', W) + RST)
  const statusText = 'ready'
  const rightPart = '\u{2191}2.1k \u{2193}0.8k  $0.0032 [!]'
  const sGap = Math.max(1, W - 4 - statusText.length - rightPart.length)
  lines.push(PAD + DIM + statusText + RST + ' '.repeat(sGap) + ACC + '\u{2191}2.1k \u{2193}0.8k' + RST + '  ' + DIM + '$0.0032 [!]' + RST)
  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

export function getTutorialFirstRunSteps(rows = 24): TutorialStep[] {
  return [
    { title: 'First launch', narration: 'On first launch koma shows a three-way chooser. Select provider to sign in through your browser, or custom to enter your own endpoint and API key.', points: ['koma free starts instantly with no key.', 'Your choice can be changed later in /settings.'], screen: screenChooser(rows) },
    { title: 'Choose a provider', narration: 'Selecting provider opens the OAuth provider picker. Choose koma.run (or any listed provider) with ↑↓ and press Enter.', points: ['The same provider list appears in /settings \u2192 OAuth later.'], screen: screenOnboardOAuthPicker(rows) },
    { title: 'Sign in in your browser', narration: 'Koma shows the browser login URL. Press c to copy it, or o to open it directly. Approve the sign-in in your browser.', points: ['Esc cancels the flow and returns to the chooser.'], screen: screenOnboardOAuthWait(rows) },
    { title: 'Pick a model', narration: 'After signing in, choose which model to use. Type to filter the list, then press Enter to confirm.', points: ['The model is assigned the Main role automatically.', 'You can change it later with /model.'], screen: screenModelSelect(rows) },
    { title: 'You\'re connected', narration: 'Koma lands in the main chat, ready to work with your chosen provider and model. Type any task to get started.', points: ['The status bar shows token usage and cost in real time.'], screen: screenChatWelcome(rows) },
  ]
}
