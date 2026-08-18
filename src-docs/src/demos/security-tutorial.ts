/**
 * Tutorial screens for `/security` — the security daemon control panel.
 *
 * `/security` opens a full-screen status panel (no chat chrome) that shows the
 * daemon lifecycle, the installed tool inventory grouped by domain, and an
 * optional dependency-health pane. The daemon is optional: it must first be
 * provisioned from a normal shell with `koma --security-install` (Python 3.8+,
 * unsupported on Windows), then started from this panel with Space on every
 * Koma launch (it is not auto-started).
 *
 * Layouts match src-agent/src/view/security/mod.rs exactly:
 *   - Header: dim "security" label (row 0) + BOTTOM-only border (row 1).
 *   - Body (cols 2..77, rows 3..): daemon checkbox, installed/tool-count line,
 *     YOLO checkbox (locked until daemon running), then tools grouped by domain.
 *   - Footer (last row): full-width inverse hint bar.
 *
 * The security daemon ships 23 tools across four domains (web, pwn, crypto,
 * web-re). Tool names, domains, compute classes, and risk flags are taken
 * verbatim from src-security/koma_sec_daemon/registry.py + the per-tool
 * descriptor files. The dependency-health rows are a representative subset of
 * src-security/koma_sec_daemon/install_manifest.py (the real list reflects your
 * installed dependencies) and are labelled illustrative in narration.
 *
 * Colours use the same ANSI mappings as theme.rs dark():
 *   \x1b[32m  accent   (#39ff14)
 *   \x1b[37m  fg       (#e6e6e6)
 *   \x1b[90m  dim      (#adadad)
 *   \x1b[33m  warn     (#ffb43c)
 *   \x1b[30m  sel_fg   (black)
 *   \x1b[42m  sel_bg   (accent)
 */

import { RST, ACC, FG, DIM, WARN, INVERSE, BOLD_ACC, trunc, padRight, bar, commandEntryScreen } from './chat-chrome'
import type { TutorialStep } from './first-run-tutorial'

const RED = '\x1b[31m'
const BOLD_RED = '\x1b[1;31m'

// ─── Security daemon tool inventory ────────────────────────────────────────
// Verbatim from src-security/koma_sec_daemon/registry.py (REGISTRY insertion
// order) plus each tool's descriptor (name/domain/compute/risk).
interface SecTool {
  name: string
  domain: string
  compute: string
  risk: boolean
}

const SEC_TOOLS: SecTool[] = [
  { name: 'sec_http',        domain: 'web',    compute: 'network',         risk: false },
  { name: 'sec_remote',      domain: 'pwn',    compute: 'executes-target', risk: true  },
  { name: 'sec_sqlmap',      domain: 'web',    compute: 'executes-target', risk: true  },
  { name: 'sec_nuclei',      domain: 'web',    compute: 'network',         risk: true  },
  { name: 'sec_ffuf',        domain: 'web',    compute: 'network',         risk: true  },
  { name: 'sec_dalfox',      domain: 'web',    compute: 'network',         risk: true  },
  { name: 'sec_zap',         domain: 'web',    compute: 'network',         risk: true  },
  { name: 'sec_xss_confirm', domain: 'web',    compute: 'executes-target', risk: true  },
  { name: 'sec_z3',          domain: 'crypto', compute: 'long-cpu',        risk: false },
  { name: 'sec_sage',        domain: 'crypto', compute: 'long-cpu',        risk: false },
  { name: 'sec_rsa',         domain: 'crypto', compute: 'long-cpu',        risk: false },
  { name: 'sec_factor',      domain: 'crypto', compute: 'network',         risk: false },
  { name: 'sec_lattice',     domain: 'crypto', compute: 'long-cpu',        risk: false },
  { name: 'sec_crack',       domain: 'crypto', compute: 'gpu',             risk: false },
  { name: 'sec_hashid',      domain: 'crypto', compute: 'instant-cpu',     risk: false },
  { name: 'sec_decode',      domain: 'crypto', compute: 'instant-cpu',     risk: false },
  { name: 'sec_jsdeobf',     domain: 'web-re', compute: 'instant-cpu',     risk: false },
  { name: 'sec_unmin',       domain: 'web-re', compute: 'instant-cpu',     risk: false },
  { name: 'sec_sourcemap',   domain: 'web-re', compute: 'instant-cpu',     risk: false },
  { name: 'sec_wasm',        domain: 'web-re', compute: 'instant-cpu',     risk: false },
  { name: 'sec_triage',      domain: 'pwn',    compute: 'instant-cpu',     risk: false },
  { name: 'sec_rop',         domain: 'pwn',    compute: 'instant-cpu',     risk: false },
  { name: 'sec_pwntmpl',     domain: 'pwn',    compute: 'instant-cpu',     risk: false },
]

// ─── Dependency-health rows (install_manifest.py subset, illustrative) ─────
interface Dep {
  name: string
  tier: number
  present: boolean
  method: string
  hint: string
  tools: string[]
}

// Representative subset: pip deps installed after `koma --security-install`,
// heavy tier-2/tier-3 binaries left missing to show the [!!] path.
const SEC_DEPS: Dep[] = [
  { name: 'requests',  tier: 1, present: true,  method: 'pip',    hint: 'pip install requests',       tools: ['sec_http', 'sec_zap', 'sec_factor'] },
  { name: 'pwntools',  tier: 1, present: true,  method: 'pip',    hint: 'pip install pwntools>=4.15', tools: ['sec_remote', 'sec_pwntmpl', 'sec_triage', 'sec_rop'] },
  { name: 'z3-solver', tier: 1, present: true,  method: 'pip',    hint: 'pip install z3-solver',      tools: ['sec_z3'] },
  { name: 'sqlmap',    tier: 1, present: true,  method: 'pip',    hint: 'pip install sqlmap',         tools: ['sec_sqlmap'] },
  { name: 'checksec',  tier: 1, present: true,  method: 'pip',    hint: 'pip install checksec.py',    tools: ['sec_triage'] },
  { name: 'nuclei',    tier: 2, present: false, method: 'binary', hint: 'go install .../nuclei@latest', tools: ['sec_nuclei'] },
  { name: 'ffuf',      tier: 2, present: false, method: 'binary', hint: 'go install .../ffuf/v2@latest', tools: ['sec_ffuf'] },
  { name: 'zap',       tier: 3, present: false, method: 'manual', hint: 'download from zaproxy.org',  tools: ['sec_zap'] },
  { name: 'hashcat',   tier: 3, present: false, method: 'manual', hint: 'apt install hashcat',        tools: ['sec_crack'] },
]

function missingDepFor(toolName: string, deps: Dep[]): boolean {
  return deps.some((d) => d.tools.includes(toolName) && !d.present)
}

const TOOL_COUNT = SEC_TOOLS.length

// ─── Shell-state renderer (prerequisite, outside Koma) ─────────────────────
// Renders a plain shell frame: a `$ ` prompt, an optional typed command (with a
// block cursor), and installer output lines. Used for the `koma --security-install`
// and `koma --internet-fullmode-install` prerequisite steps.

export interface ShellOpts {
  /** The command already typed at the prompt (with a block cursor). */
  typed?: string
  /** Output lines printed after the command (installer stdout). */
  output?: string[]
  /** Trailing prompt shown after output (no cursor). */
  trailingPrompt?: boolean
  prompt?: string
}

export function shellScreen(rows: number, opts: ShellOpts): string {
  const W = 80
  const prompt = opts.prompt ?? '$ '
  const lines: string[] = []
  if (opts.typed !== undefined) {
    lines.push(DIM + prompt + RST + FG + opts.typed + RST + ACC + '\u{2588}' + RST)
  }
  for (const o of opts.output ?? []) {
    lines.push(FG + o + RST)
  }
  if (opts.trailingPrompt && !opts.typed) {
    lines.push(DIM + prompt + RST)
  }
  while (lines.length < rows) lines.push('')
  return lines
    .slice(0, rows)
    .map((l) => padRight(trunc(l, W), W))
    .join('\n')
}

// ─── Panel state + builder ─────────────────────────────────────────────────

interface SecState {
  running: boolean
  /** Selected row index into tool_items() (0 = Daemon checkbox). */
  selected: number
  yoloArmed: boolean
  healthFetching: boolean
  healthView: boolean
  healthSelected: number
  inactive: Set<string>
  deps: Dep[]
}

const W = 80
const BODY_W = W - 4 // body inner width (cols 2..77)

function tierLabel(tier: number): string {
  switch (tier) {
    case 1: return 'pip'
    case 2: return 'auto-download'
    case 3: return 'manual'
    default: return 'other'
  }
}

/** Distinct domains in first-seen order (matches tool_items() grouping). */
function domainsFirstSeen(): string[] {
  const out: string[] = []
  for (const t of SEC_TOOLS) if (!out.includes(t.domain)) out.push(t.domain)
  return out
}

function selLine(isSel: boolean, plain: string, base: string): string {
  // When selected, INVERSE (black fg + green bg + bold) owns the whole row — the
  // plain text must stay unstyled so it renders as black-on-green, not the base
  // colour bleeding through the reverse video.
  if (isSel) return INVERSE + padRight('  ' + plain, BODY_W) + RST
  return '  ' + base + plain + RST
}

function buildSecurity(rows: number, st: SecState): string {
  const content: string[] = []
  const toolFg = st.running ? FG : DIM
  const installedLabel = 'yes'

  const items: ('daemon' | 'yolo' | number)[] = ['daemon', 'yolo']
  for (const d of domainsFirstSeen()) {
    SEC_TOOLS.forEach((t, i) => {
      if (t.domain === d) items.push(i)
    })
  }

  let lastDomain: string | null = null
  items.forEach((it, pos) => {
    const isSel = pos === st.selected
    if (it === 'daemon') {
      const plain = st.running ? '[x] Daemon running' : '[ ] Daemon stopped'
      const base = st.running ? BOLD_ACC : FG
      content.push(selLine(isSel, plain, base))

      // installed / tool-count line, with optional health spinner or [!!] marker
      let info = DIM + `installed: ${installedLabel} · ${TOOL_COUNT} tools` + RST
      if (st.healthFetching) {
        info += DIM + ' · ' + RST + ACC + '\u{280b} checking dependencies\u2026' + RST
      } else if (st.deps.length && SEC_TOOLS.some((t) => missingDepFor(t.name, st.deps))) {
        info += DIM + ' · ' + RST + WARN + '[!!]' + RST + DIM + ' dependency not installed' + RST
      }
      content.push('  ' + info)
      content.push('')
    } else if (it === 'yolo') {
      if (!st.running) {
        content.push(
          '  ' + DIM + '[ ] Enable YOLO mode' + RST + DIM + '   (start daemon first)' + RST,
        )
      } else if (st.yoloArmed) {
        content.push(selLine(isSel, '[x] Enable YOLO mode', BOLD_ACC))
        content.push('  ' + BOLD_RED + '! YOLO MODE ENABLED' + RST)
      } else {
        content.push(selLine(isSel, '[ ] Enable YOLO mode', FG))
        content.push('  ' + DIM + 'YOLO mode disabled \u2014 harness active' + RST)
      }
      content.push('')
    } else {
      const t = SEC_TOOLS[it]
      if (lastDomain !== t.domain) {
        if (lastDomain !== null) content.push('')
        content.push('  ' + DIM + `[${t.domain}]` + RST)
        lastDomain = t.domain
      }
      const isInactive = st.inactive.has(t.name)
      const nameStyle = isInactive ? DIM : toolFg
      let inner = '  ' + nameStyle + t.name.padEnd(20) + RST
      if (t.compute) inner += DIM + `  [${t.compute}]` + RST
      if (t.risk) inner += DIM + '  risky' + RST
      if (isInactive) inner += DIM + '  off' + RST
      if (missingDepFor(t.name, st.deps)) inner += WARN + '  [!!]' + RST
      content.push('  ' + inner)
    }
  })

  // Footer: full-width inverse hint bar
  const hint = st.healthView
    ? '↑↓ move · i install · h tools · r restart · Esc'
    : '↑↓ move · Space toggle · d domain · h deps · r restart · Esc'
  const footer = INVERSE + ' ' + hint + ' '.repeat(Math.max(0, 79 - hint.length)) + RST

  const lines: string[] = []
  lines.push('  ' + DIM + 'security' + RST)
  lines.push(DIM + bar('\u2500', W) + RST)
  lines.push('') // body top margin
  lines.push(...content)
  while (lines.length < rows - 1) lines.push('')
  return lines.slice(0, rows - 1).concat(footer).join('\n')
}

// ─── Dependency-health pane (h) ────────────────────────────────────────────

function buildSecurityDeps(rows: number, st: SecState): string {
  const content: string[] = []
  if (st.deps.length === 0) {
    content.push('  ' + DIM + 'no health data (daemon stopped)' + RST)
  } else {
    // group indices by tier ascending, then manifest order
    const tiers = [...new Set(st.deps.map((d) => d.tier))].sort((a, b) => a - b)
    const order: number[] = []
    for (const tier of tiers) {
      st.deps.forEach((d, i) => {
        if (d.tier === tier) order.push(i)
      })
    }
    let lastTier: number | null = null
    order.forEach((idx, pos) => {
      const e = st.deps[idx]
      if (lastTier !== e.tier) {
        if (lastTier !== null) content.push('')
        content.push('  ' + DIM + `── tier ${e.tier} (${tierLabel(e.tier)}) ──` + RST)
        lastTier = e.tier
      }
      const isSel = pos === st.healthSelected
      const marker = e.present ? 'ok     ' : 'missing'
      const markerStyle = e.present ? ACC : DIM
      const nameText = e.name.padEnd(18)
      const nameRendered = isSel
        ? INVERSE + '  ' + nameText + RST
        : '  ' + (e.present ? ACC : DIM) + nameText + RST
      content.push(nameRendered + DIM + '  ' + RST + markerStyle + marker + RST + DIM + `  ${e.method}` + RST)
      if (e.hint) content.push('  ' + DIM + `      needs: ${e.hint}` + RST)
      if (e.tools.length) content.push('  ' + DIM + `      enables: ${e.tools.join(', ')}` + RST)
    })
  }

  const hint = '↑↓ move · i install · h tools · r restart · Esc'
  const footer = INVERSE + ' ' + hint + ' '.repeat(Math.max(0, 79 - hint.length)) + RST

  const lines: string[] = []
  lines.push('  ' + DIM + 'security' + RST)
  lines.push(DIM + bar('\u2500', W) + RST)
  lines.push('')
  lines.push(...content)
  while (lines.length < rows - 1) lines.push('')
  return lines.slice(0, rows - 1).concat(footer).join('\n')
}

// ─── Tutorial steps ────────────────────────────────────────────────────────

/** Build the tutorial steps for a given terminal row count (default 24). */
export function getSecuritySteps(rows = 24): TutorialStep[] {
  const baseState: SecState = {
    running: false,
    selected: 0,
    yoloArmed: false,
    healthFetching: false,
    healthView: false,
    healthSelected: 0,
    inactive: new Set(),
    deps: [],
  }

  return [
    {
      title: 'Install the daemon first',
      narration:
        'The security daemon is optional and ships separately. From a normal shell — not inside Koma — run `koma --security-install`. It needs Python 3.8+ and is unsupported on Windows.',
      points: [
        'Runs before Koma starts (CLI mode), not from the chat',
        'Extracts bundled assets into ~/.koma/security/',
        'Creates a Python venv and installs the dependencies',
      ],
      screen: shellScreen(rows, { typed: 'koma --security-install' }),
    },
    {
      title: 'Installer output',
      narration:
        'The installer prints progress to stdout: asset extraction, venv creation, pip upgrades, the requirements install, and a final “installed at” line. No pip progress bars or machine-specific paths are fabricated here — these are the real println! strings from security/mod.rs.',
      points: [
        '~/.koma/security/ is the canonical install root',
        'checksec.py is installed with --no-deps to avoid downgrades',
        'Re-run with --force to reinstall',
      ],
      screen: shellScreen(rows, {
        output: [
          '$ koma --security-install',
          'extracting security assets to ~/.koma/security/...',
          'creating Python venv...',
          'upgrading pip...',
          'installing Python dependencies from ~/.koma/security/requirements.txt...',
          'installing checksec.py (with --no-deps)...',
          ACC + 'security daemon installed at ~/.koma/security/' + RST,
        ],
        trailingPrompt: true,
      }),
    },
    {
      title: 'Open /security',
      narration:
        'Launch Koma normally, then type /security in the shared composer and press Enter. It opens the full-screen control panel (it works with no active session — daemon lifecycle is global state).',
      screen: commandEntryScreen(rows, '/security'),
    },
    {
      title: 'Panel — daemon stopped',
      narration:
        'The panel opens with the daemon stopped. The selected [ ] Daemon running checkbox, an “installed: yes · 23 tools” line, and a YOLO row locked behind “(start daemon first)” are shown. Tools are dimmed because nothing is live yet.',
      points: [
        'Up/Down move the cursor; Space/Enter toggle the selected row',
        'd toggles a whole domain; h opens dependency health; r restarts; Esc closes',
        'YOLO stays locked until the daemon is running',
      ],
      screen: buildSecurity(rows, { ...baseState }),
    },
    {
      title: 'Press Space to start',
      narration:
        'With the daemon row selected, press Space. The checkbox turns green ([x] Daemon running) and Koma kicks off an async dependency-health probe — shown as a braille “checking dependencies…” spinner. The daemon is NOT auto-started on a new Koma launch, so you repeat this each session.',
      points: [
        '“YOLO mode disabled — harness active” appears once the daemon runs',
        'The probe runs off-thread so the panel never freezes',
        'Start it from this panel every launch',
      ],
      screen: buildSecurity(rows, { ...baseState, running: true, healthFetching: true }),
    },
    {
      title: 'Daemon running — tools live',
      narration:
        'Once health resolves, the checkbox stays active and tools render in normal colour. Tools whose dependencies are still missing carry a [!!] marker. YOLO can now be armed (Space on its row) but remains disarmed here.',
      points: [
        '23 tools span web, pwn, crypto, and web-re domains',
        '[!!] flags a tool whose backing dependency is not installed',
        'Toggle individual tools or whole domains with Space / d',
      ],
      screen: buildSecurity(rows, { ...baseState, running: true, deps: SEC_DEPS }),
    },
    {
      title: 'Dependency health (press h)',
      narration:
        'Press h to swap the body for the install-health pane, grouped by tier (pip / auto-download / manual). Each row shows ok or missing plus its install method; i installs the selected dependency. Rows shown are illustrative — the real list reflects what is installed on your machine.',
      points: [
        'Tier 1 (pip) and tier 3 (manual) dependencies back most tools',
        'Tier 2 binaries (e.g. nuclei, ffuf) download from GitHub releases',
        'i repairs the selected missing dependency',
      ],
      screen: buildSecurityDeps(rows, { ...baseState, running: true, deps: SEC_DEPS, healthSelected: 0 }),
    },
  ]
}
