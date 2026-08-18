/**
 * Tutorial screens for `/internet` — toggling internet mode.
 *
 * `/internet` flips the agent's internet mode between simple and full. It is a
 * chat command (no panel/picker opens) that just updates the session status
 * line. Full mode unlocks the browser-backed tools, but only after the Firefox
 * for Playwright backend is provisioned from a normal shell with
 * `koma --internet-fullmode-install` (which must run BEFORE launching Koma).
 *
 * Chat chrome matches src-agent/src/view/chat/* (header.rs, input.rs, status.rs):
 * the status bar shows the transient feedback string in dim at the left.
 *
 * Colours use the same ANSI mappings as theme.rs dark():
 *   \x1b[32m  accent   (#39ff14)
 *   \x1b[37m  fg       (#e6e6e6)
 *   \x1b[90m  dim      (#adadad)
 */

import {
  RST,
  ACC,
  FG,
  DIM,
  INVERSE,
  trunc,
  padRight,
  bar,
  chatHeader,
  commandEntryScreen,
} from './chat-chrome'
import { shellScreen } from './security-tutorial'
import type { TutorialStep } from './first-run-tutorial'

const W = 80

// ─── Chat input with a custom status line ──────────────────────────────────
// Mirrors chat-chrome chatInput() but lets the bottom status row show the
// internet-mode feedback (e.g. "internet: full") with a cleared composer.

function chatInputStatus(text: string, status: string): string[] {
  const model = 'claude-3.5-sonnet'
  const modelLine = DIM + ' '.repeat(W - 2 - model.length) + model + RST
  const sess = ' session-71cdd2dc '
  const topBorder = DIM + bar('\u2500', W - sess.length) + ACC + sess + RST
  return [
    modelLine,
    topBorder,
    '  ' + ACC + '[$] ' + RST + ACC + text + '\u{2588}' + RST,
    DIM + bar('\u2500', W) + RST,
    '  ' + DIM + status + RST,
  ]
}

function composeChat(rows: number, transcript: string[], text: string, status: string): string {
  const header = chatHeader('normal')
  const input = chatInputStatus(text, status)
  const all = [...header, ...transcript, ...input]
  // Pad transcript with blanks so the input lands at the bottom.
  while (all.length < rows) all.splice(header.length, 0, '')
  return all
    .slice(0, rows)
    .map((l) => padRight(trunc(l, W), W))
    .join('\n')
}

// ─── Tutorial steps ─────────────────────────────────────────────────────────

/** Build the tutorial steps for a given terminal row count (default 24). */
export function getInternetSteps(rows = 24): TutorialStep[] {
  return [
    {
      title: 'Provision Full mode first',
      narration:
        'Full internet mode needs a browser backend. From a normal shell — before you launch Koma — run `koma --internet-fullmode-install`. It provisions a Python venv under ~/.koma/internet/ and downloads Firefox for Playwright (~80 MB).',
      points: [
        'Runs in CLI mode, before the TUI starts',
        'Installs the scrapion_agent package + Firefox for Playwright',
        'Final line tells you to select full in /settings or via /internet full',
      ],
      screen: shellScreen(rows, { typed: 'koma --internet-fullmode-install' }),
    },
    {
      title: 'Installer output',
      narration:
        'The installer prints its real progress: asset extraction, venv creation, the scrapion_agent install, the Firefox-for-Playwright download, and the final instruction to select full. No download bars or machine-specific paths are invented — these are the actual println! strings from internet/mod.rs.',
      points: [
        '~/.koma/internet/ is the canonical install root',
        'Firefox download is ~80 MB (Playwright step)',
        'Re-run with --force to reinstall',
      ],
      screen: shellScreen(rows, {
        output: [
          '$ koma --internet-fullmode-install',
          'extracting internet assets to ~/.koma/internet/...',
          'creating Python venv...',
          'installing scrapion_agent package from ~/.koma/internet/...',
          'installing Firefox for Playwright (this downloads ~80 MB)...',
          ACC + 'internet research installed at ~/.koma/internet/' + RST,
          'set internet mode to `full` in /settings or via `/internet full`',
        ],
        trailingPrompt: true,
      }),
    },
    {
      title: 'Type /internet full',
      narration:
        'Launch Koma and type /internet full in the shared composer. An active session is required — the mode is a per-session setting. This is a plain chat command; no picker or panel opens.',
      screen: commandEntryScreen(rows, '/internet full'),
    },
    {
      title: 'Applied: internet: full',
      narration:
        'The composer clears and the status line flashes “internet: full”. The setting persists in the session. Ctrl+E toggles the same setting; /internet simple returns to Simple explicitly. Simple keeps web_search / web_fetch; Full additionally unlocks the browser-backed tools now that the backend is installed.',
      points: [
        'No panel — the only change is the status-line feedback',
        'Ctrl+E toggles simple ⇄ full; the choice persists for the session',
        '/internet simple reverts to Simple at any time',
      ],
      screen: composeChat(rows, [], '', 'internet: full'),
    },
    {
      title: 'Edge case: Full without provisioning',
      narration:
        'If you select full before running the installer, Koma does not enable the browser tools. Instead the status line shows the exact install command. The full-mode tools are still advertised in both modes — calling one returns an install/mode error rather than being silently hidden.',
      points: [
        'Exact message: internet: full needs `koma --internet-fullmode-install`',
        'The browser tools stay available-but-blocked until you provision',
        'Simple mode keeps working regardless of the backend',
      ],
      screen: composeChat(
        rows,
        [],
        '',
        'internet: full needs `koma --internet-fullmode-install`',
      ),
    },
  ]
}
