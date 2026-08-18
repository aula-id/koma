import {
  RST,
  ACC,
  FG,
  DIM,
  INVERSE,
  trunc,
  padRight,
  bar,
  commandEntryScreen,
  composeChatScreen,
} from './chat-chrome'
import type { TutorialStep } from './first-run-tutorial'

const BOLD = '\x1b[1m'
const W = 80

// ─── Quit-confirm overlay renderer ───────────────────────────────────────────
// Faithful reproduction of src-agent/src/view/quit_confirm.rs.
//
// Layout (80×rows): top+bottom rule title bar (rows 0-2, " quit " on the top
// rule), a dead-centred body (question + optional working warning + blank +
// button row + blank + focused-button description), and a dim keybinding hint
// on the last row. The three buttons — [ quit ] [ detach ] [ cancel ] — are
// laid out with a 3-col gap; the focused one is reversed onto the accent
// colour (black-on-green, bold). request_quit() ALWAYS opens this overlay
// (even when idle) so the user can choose to detach; only zero sessions or a
// landing/unconfigured screen quit immediately.

const QC_LABELS = ['quit', 'detach', 'cancel']
const QC_DESCS = [
  'Quit session and stop current progress',
  'Minimize — as usual agent keep cooking',
  'Back to chat',
]

interface QuitOpts {
  working: number
  total: number
  /** Focused button index (0=quit, 1=detach, 2=cancel). Default 2 (cancel). */
  selected: number
  /** Exiting phase: show a braille spinner inside the focused (activated) chip. */
  exiting: boolean
}

function buildQuit(rows: number, opts: QuitOpts): string {
  const lines: string[] = new Array(rows).fill('')
  // Title bar (rows 0-2): " quit " on the TOP rule, dim subtitle, bottom rule.
  lines[0] = DIM + '─ quit ' + bar('\u2500', W - 7) + RST
  lines[1] =
    DIM +
    (opts.working > 0
      ? 'a quit was requested while work is still in flight'
      : 'a quit was requested') +
    RST
  lines[2] = DIM + bar('\u2500', W) + RST
  // Footer hint (last row), dim.
  lines[rows - 1] = DIM + '←/→ move · Enter select · k/d/Esc shortcut · click' + RST

  // Body lines (top-down).
  const body: string[] = []
  body.push(BOLD + FG + 'Do you want to quit?' + RST)
  if (opts.working > 0) {
    const plural = opts.working === 1 ? 'session' : 'sessions'
    body.push(
      DIM + `${opts.working} ${plural} still working — in-flight work will be lost.` + RST,
    )
  }
  body.push('')
  // Button row: [ quit ]   [ detach ]   [ cancel ] (GAP = 3).
  let row = ''
  QC_LABELS.forEach((lab, i) => {
    const focused = i === opts.selected
    let chip: string
    if (opts.exiting && focused) {
      chip = INVERSE + BOLD + `[ \u{280b}${lab} ]` + RST
    } else if (focused) {
      chip = INVERSE + `[ ${lab} ]` + RST
    } else {
      chip = ACC + `[ ${lab} ]` + RST
    }
    if (i > 0) row += ' '.repeat(3)
    row += chip
  })
  body.push(row)
  body.push('')
  body.push(DIM + QC_DESCS[opts.selected] + RST)

  // Dead-centre the body within rows 3..(rows-2).
  const bodyH = rows - 4
  const startRow = 3 + Math.floor((bodyH - body.length) / 2)
  const indent = 14
  body.forEach((bl, i) => {
    lines[startRow + i] = ' '.repeat(indent) + bl
  })

  return lines.join('\n')
}

// ─── Tutorial steps ──────────────────────────────────────────────────────────

/** Build the tutorial steps for a given terminal row count (default 24). */
export function getQuitSteps(rows = 24): TutorialStep[] {
  return [
    {
      title: 'Ctrl+C anywhere',
      narration:
        'Ctrl+C is intercepted globally in every mode and routed through the same quit chokepoint as the /quit command — it never means “cancel” inside a prompt. From a chat, pressing Ctrl+C opens the quit confirm overlay.',
      points: [
        'Works from any mode, not just chat',
        'Identical code path to typing /quit',
        'Ctrl+C never reaches the overlay as a key',
      ],
      screen: composeChatScreen(rows, [], '', 80, 'normal'),
    },
    {
      title: 'Type /quit',
      narration:
        'Type /quit (or the aliases /q and /exit) in the composer and press Enter. It runs the exact same request_quit() the keybind triggers.',
      screen: commandEntryScreen(rows, '/quit'),
    },
    {
      title: 'Quit confirm overlay',
      narration:
        'koma always asks before quitting — even with nothing working — so you can choose to detach idle sessions. Three choices: quit (kill), detach (keep cooking), or cancel (back to chat). Focus defaults to cancel, the safe choice, so an accidental Enter will not close anything.',
      points: [
        'k / quit — close window; the session ends (kept on disk)',
        'd / detach — leave the agent running in the background',
        'Esc / Enter — cancel, return to chat (Enter is safe: cancel is focused)',
      ],
      screen: buildQuit(rows, { working: 1, total: 5, selected: 2, exiting: false }),
    },
    {
      title: 'Choose quit (k)',
      narration:
        'Press k (or → to focus [ quit ] then Enter) to quit. The chip shows a braille spinner while koma tears down; all input is suppressed. d detaches instead — the agent keeps working headless and reappears in the session hub’s history.',
      points: [
        'Exiting phase suppresses every key and click',
        'quit kills this window’s session-daemon',
        'detach leaves it running and resumable',
      ],
      screen: buildQuit(rows, { working: 1, total: 5, selected: 0, exiting: true }),
    },
    {
      title: 'Cancel (Esc)',
      narration:
        'Press Esc (or focus [ cancel ] and Enter) to back out — you return to the chat with nothing changed. Because focus starts on cancel, a plain Enter also cancels safely.',
      screen: composeChatScreen(rows, [], '', 80, 'normal'),
    },
  ]
}
