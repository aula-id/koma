/**
 * Tutorial screens for /skill — the skill hub overlay.
 *
 * Step 1: the skill overlay showing all 8 skills with filter chips
 *         and the "active" chip selected.
 * Step 2: filtered results after typing "domain" in the search bar,
 *         showing only domain-web and domain-cli.
 *
 * Layout matches src-agent/src/view/skill_cmd.rs exactly: bordered overlay
 * anchored above composer, header on bottom rule, search bar, chip row,
 * filtered list, inverse footer bar.
 */

const RST = '\x1b[0m'
const ACC = '\x1b[32m'
const FG  = '\x1b[37m'
const DIM = '\x1b[90m'
const WARN = '\x1b[33m'
const SEL_FG = '\x1b[30m'
const SEL_BG = '\x1b[42m'
const INVERSE = SEL_FG + SEL_BG + '\x1b[1m'
const SUCCESS = '\x1b[92m'

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

// ─── Screen: Skill Hub overlay ────────────────────────────────────────
// Matches src-agent/src/view/skill_cmd.rs: bordered overlay anchored
// above composer, header on bottom rule, search bar, chip row,
// filtered skill list, inverse footer bar.

interface SkillEntry {
  name: string
  active: boolean
  description: string
  visible: boolean
}

function buildSkillOverlay(skills: SkillEntry[], searchQuery: string, selectedIdx: number, allChipActive: boolean, activeChipActive: boolean, rows: number): string {
  const W = 80
  const lines: string[] = []

  // Build the overlay lines
  const overlayLines: string[] = []
  const innerW = W - 2 // inside borders

  // Header: "skills" (dim) on BOTTOM rule, 2 rows
  overlayLines.push(DIM + bar('\u2500', W) + RST)
  overlayLines.push(DIM + ' skills' + bar('\u2500', W - ' skills'.length) + RST)

  // Search line: \u203a (dim) + query + \u{2588} (accent cursor), 1 row
  const searchContent = searchQuery + '\u{2588}'
  overlayLines.push(DIM + '\u203a ' + RST + FG + searchContent + RST)

  // Chip row: [X]all  [ ]active
  const allChip = allChipActive
    ? INVERSE + '[X]all' + RST
    : DIM + '[ ]all' + RST
  const activeChip = activeChipActive
    ? INVERSE + '[ ]active' + RST
    : DIM + '[ ]active' + RST
  overlayLines.push('  ' + allChip + '  ' + activeChip)

  // Spacer
  overlayLines.push('')

  // Filtered list: name (24 chars) + [active]/[      ] badge + description
  const nameW = 24
  const badgeActive = ' [active]  '
  const badgeInactive = ' [        ] '
  let visibleIdx = 0
  for (let i = 0; i < skills.length; i++) {
    const skill = skills[i]
    if (!skill.visible) continue
    const badge = skill.active
      ? SUCCESS + badgeActive + RST
      : DIM + badgeInactive + RST
    const namePart = FG + skill.name.padEnd(nameW) + RST
    const descPart = DIM + skill.description + RST
    const full = namePart + badge + descPart

    if (visibleIdx === selectedIdx) {
      overlayLines.push(INVERSE + padRight(namePart + (skill.active ? badgeActive : badgeInactive) + descPart, innerW) + RST)
    } else {
      overlayLines.push(' ' + full)
    }
    visibleIdx++
  }

  // Pad remaining rows for the overlay area
  while (overlayLines.length < 15) {
    overlayLines.push(DIM + '\u2502' + RST + ' '.repeat(innerW) + DIM + '\u2502' + RST)
  }

  // Footer — inverse bar
  const footerText = ' enter toggle \u00b7 \u2190\u2192 filter \u00b7 esc close '
  overlayLines.push(INVERSE + padRight(footerText, W) + RST)

  // Compose full screen: chat header above, overlay anchored above input
  lines.push(...chatHeader())
  lines.push('')
  lines.push('  ' + FG + 'which skills are loaded?' + RST)

  const inputBar = chatInput('/skill')
  const targetStart = rows - inputBar.length - overlayLines.length
  while (lines.length < targetStart) lines.push('')
  lines.push(...overlayLines)
  lines.push(...inputBar)

  while (lines.length < rows) lines.push('')
  return lines.slice(0, rows).map(l => padRight(trunc(l, W), W)).join('\n')
}

// ─── Tutorial Steps ───────────────────────────────────────────────────

import type { TutorialStep } from './first-run-tutorial'

export function getSkillSteps(rows = 24): TutorialStep[] {
  // Full list of 8 skills
  const allSkills: SkillEntry[] = [
    { name: 'coding-guidelines', active: true,  description: 'Code style and best practices',     visible: true },
    { name: 'domain-web',        active: true,  description: 'Web service development',            visible: true },
    { name: 'domain-cli',        active: false, description: 'CLI tools and terminal apps',        visible: true },
    { name: 'm01-ownership',     active: false, description: 'Ownership and borrow checker',       visible: true },
    { name: 'm07-concurrency',   active: false, description: 'Async and concurrency issues',       visible: true },
    { name: 'rust-learner',      active: true,  description: 'Rust version and crate info',        visible: true },
    { name: 'rust-refactor',     active: false, description: 'Safe refactoring with LSP',          visible: true },
    { name: 'unsafe-checker',    active: false, description: 'Unsafe code and FFI review',         visible: true },
  ]

  // Step 1: All skills, "active" chip selected, row 0 ("coding-guidelines") selected
  const screen1 = buildSkillOverlay(allSkills, '', 0, false, true, rows)

  // Step 2: Filtered by "domain" — only domain-web and domain-cli
  const filteredSkills: SkillEntry[] = [
    { name: 'domain-web', active: true,  description: 'Web service development',  visible: true },
    { name: 'domain-cli', active: false, description: 'CLI tools and terminal apps', visible: true },
  ]
  const screen2 = buildSkillOverlay(filteredSkills, 'domain', 0, false, false, rows)

  return [
    {
      title: 'Skill Hub',
      narration:
        'Type /skill to open the skill hub overlay. It lists all available skills with their active status and lets you toggle them on or off.',
      points: [
        'Skills marked [active] are loaded and injected into your context',
        'Use the [X]all / [ ]active filter chips to narrow the list',
        'Press Enter to toggle a skill on or off',
      ],
      screen: screen1,
    },
    {
      title: 'Filtered Skills',
      narration:
        'Start typing in the search bar to filter skills by name. Here "domain" narrows the list to domain-specific skills for web and CLI development.',
      points: [
        'Partial name matching works — "domain" matches both domain-web and domain-cli',
        'Only active skills contribute to your context — inactive ones are ignored',
        'Press Esc to clear the filter and return to the full list',
      ],
      screen: screen2,
    },
  ]
}
