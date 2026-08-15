/**
 * Tutorial screens for /settings → OAuth.
 *
 * Three screens: connection list, provider picker, and browser-wait flow.
 * Uses koma.run as the safe example provider (not Codex/Claude which have
 * external OAuth dependencies).
 *
 * Layout matches src-agent/src/view/settings/pages/oauth.rs exactly.
 * The provider picker is a full-screen clear overlay (no breadcrumb).
 * The browser-wait screen uses a Braille spinner animation frame.
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

// ─── Screen: OAuth Connection List ────────────────────────────────────
// Matches oauth.rs idle state: breadcrumb + table + connect button.
// Table columns: Provider(12), Account(flex), Status(16).

function screenOAuthList(rows = 24): string {
  const W = 80
  const lines: string[] = []

  // Breadcrumb header (2 rows)
  lines.push(DIM + '  settings > OAuth' + RST)
  lines.push(DIM + bar('\u2500', W) + RST)

  // Column header
  const provW = 12
  const statusW = 16
  const acctW = W - provW - statusW - 2
  const hdr = 'Provider'.padEnd(provW) + 'Account'.padEnd(acctW) + 'Status'.padEnd(statusW)
  lines.push(DIM + hdr + RST)

  // Data rows — koma.run as primary example
  const conns = [
    { prov: 'koma.run', acct: 'alice@koma.run', status: 'active', sel: true },
    { prov: 'kilo code', acct: 'bob@org.dev', status: 'renews in 5d', sel: false },
  ]

  for (const c of conns) {
    const provCol = c.prov.padEnd(provW)
    const acctCol = trunc(c.acct, acctW)
    const statusCol = c.status.padEnd(statusW)
    const full = provCol + acctCol + statusCol
    if (c.sel) {
      lines.push(SEL_FG + SEL_BG + padRight(full, W) + RST)
    } else {
      lines.push(FG + full + RST)
    }
  }

  // Blank line + connect button
  lines.push('')
  lines.push(ACC + '[ + connect ]' + RST)

  // Footer — inverse bar
  while (lines.length < rows - 1) lines.push('')
  const footerText = ' \u2191\u2193 select \u00b7 enter connect \u00b7 ctrl+x delete \u00b7 esc back'
  lines.push(INVERSE + padRight(footerText, W) + RST)

  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

// ─── Screen: OAuth Provider Picker ────────────────────────────────────
// Full-screen clear overlay (oauth.rs:203-237). No breadcrumb —
// the background is wiped to bg before drawing the 8-item list.

function screenOAuthPicker(rows = 24): string {
  const W = 80
  const lines: string[] = []

  // Full clear — fill entire screen with bg (simulated as blank lines)
  const providers = [
    'Codex',
    'Kilo Code',
    'koma.run',
    'xAI',
    'Claude',
    'Command Code',
    'Codex (paste token)',
    'Command Code (paste key)',
  ]

  // Top spacer to vertically center the list
  const topSpacer = Math.max(1, Math.floor((rows - 2 - providers.length) / 2))
  for (let i = 0; i < topSpacer; i++) lines.push('')

  for (let i = 0; i < providers.length; i++) {
    const label = providers[i]
    if (i === 2) {
      // Selected: koma.run — cursor row padded to full width
      const text = '\u203a ' + label
      lines.push(INVERSE + padRight(text, W) + RST)
    } else {
      // Unselected: "  " prefix in accent, label in fg
      lines.push(ACC + '  ' + RST + FG + label + RST)
    }
  }

  // Footer — inverse bar
  while (lines.length < rows - 1) lines.push('')
  const footerText = ' \u2191\u2193 select \u00b7 enter choose \u00b7 esc back'
  lines.push(INVERSE + padRight(footerText, W) + RST)

  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

// ─── Screen: OAuth Browser Wait (koma.run) ────────────────────────────
// Matches oauth.rs:73-93. Full clear overlay with Braille spinner,
// headline, URL, and optional clipboard confirmation.
// koma.run listens on localhost:51004.

function screenOAuthWait(rows = 24): string {
  const W = 80
  const lines: string[] = []

  // Full clear — no breadcrumb (overlay)
  // Top spacer
  const topSpacer = Math.max(1, Math.floor(rows * 0.25))
  for (let i = 0; i < topSpacer; i++) lines.push('')

  // Spinner headline in accent
  const spinner = '\u2819' // Braille spinner frame
  lines.push(ACC + spinner + ' waiting for browser \u00b7 koma.run listening on localhost:51004' + RST)
  lines.push('')

  // Auth URL in dim
  const url = 'https://auth.koma.run/authorize?client_id=koma&scope=openid+profile'
  lines.push(DIM + url + RST)
  lines.push('')

  // Clipboard confirmation (shown after pressing 'c')
  lines.push(DIM + 'url copied to clipboard' + RST)

  // Footer — inverse bar
  while (lines.length < rows - 1) lines.push('')
  const footerText = ' c copy url \u00b7 o open browser \u00b7 esc cancel'
  lines.push(INVERSE + padRight(footerText, W) + RST)

  return lines.slice(0, rows).map(l => trunc(l, W)).join('\n')
}

// ─── Tutorial Steps ───────────────────────────────────────────────────

import type { TutorialStep } from './first-run-tutorial'

export function getSettingsOAuthSteps(rows = 24): TutorialStep[] {
  return [
    {
      title: 'OAuth Connections',
      narration:
        'The OAuth page manages provider sign-ins via browser authentication. Connected accounts appear in a table showing provider, email, and status.',
      points: [
        'Sign in to koma.run without managing API keys — just approve in your browser',
        'Status shows active, expiring, or error states',
        'Press Ctrl+X twice to disconnect an account',
      ],
      screen: screenOAuthList(rows),
    },
    {
      title: 'Connect a Provider',
      narration:
        'Press Enter on "+ connect" to open the provider picker. Choose which provider to sign in with — koma will open your browser for authentication.',
      points: [
        'Eight providers available: Codex, Kilo Code, koma.run, xAI, Claude, Command Code, and paste-token variants',
        'Paste-token options let you manually enter a key if browser auth fails',
        'Each provider uses its own OAuth flow',
      ],
      screen: screenOAuthPicker(rows),
    },
    {
      title: 'Browser Authentication',
      narration:
        'After selecting koma.run, koma starts a local listener and opens your browser. Approve the sign-in in the browser, and koma captures the token automatically.',
      points: [
        'The Braille spinner animates while waiting for the browser callback',
        'Press \'c\' to copy the URL, or \'o\' to open it manually',
        'The local listener runs on localhost:51004 and captures the OAuth redirect',
      ],
      screen: screenOAuthWait(rows),
    },
  ]
}
