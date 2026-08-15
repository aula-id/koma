/**
 * Tutorial screens for /skill — the skill hub overlay.
 *
 * Step 1: the skill overlay showing all 8 skills with the "all" filter.
 * Step 2: filtered results after typing "domain" in the search bar,
 *         showing only domain-web and domain-cli.
 *
 * Layout matches src-agent/src/view/skill_cmd.rs exactly: a 14-row overlay
 * anchored above the five-row composer, with a bottom-rule header, search,
 * filter chips, a 9-row list viewport, and an inverse footer.
 */

import { RST, ACC, FG, DIM, INVERSE, trunc, padRight, bar, chatHeader, chatInput, commandEntryScreen } from './chat-chrome'

const SUCCESS = '\x1b[92m'

// ─── Screen: Skill Hub overlay ────────────────────────────────────────

interface SkillEntry {
  name: string
  active: boolean
  description: string
  visible: boolean
}

function buildSkillOverlay(skills: SkillEntry[], searchQuery: string, selectedIdx: number, allChipActive: boolean, rows: number): string {
  const W = 80
  const lines: string[] = []
  const overlayLines: string[] = []
  const listRows = 9

  // Header: title with a full-width bottom rule.
  overlayLines.push('  ' + DIM + 'skills' + RST)
  overlayLines.push(DIM + bar('\u2500', W) + RST)

  // Search and filter chips are inset two columns, matching the TUI Margin.
  overlayLines.push('  ' + DIM + '\u203a ' + RST + FG + searchQuery + RST + ACC + '\u2588' + RST)
  const allChip = allChipActive
    ? INVERSE + '[X]all ' + RST
    : DIM + '[X]all ' + RST
  const activeChip = allChipActive
    ? DIM + '[ ]active' + RST
    : INVERSE + '[ ]active' + RST
  overlayLines.push('  ' + allChip + activeChip)

  // The real list has a 76-column viewport with a leading space, a 24-column
  // name field, a 10-column active badge, and the description.
  const nameW = 24
  let visibleIdx = 0
  for (const skill of skills) {
    if (!skill.visible) continue

    const name = ' ' + skill.name.padEnd(nameW)
    const badge = skill.active ? ' [active] ' : '          '
    if (visibleIdx === selectedIdx) {
      overlayLines.push(INVERSE + name + badge + skill.description + RST)
    } else {
      const nameStyle = skill.active ? ACC : FG
      const badgeStyle = skill.active ? SUCCESS : DIM
      overlayLines.push(
        nameStyle + name + RST +
        badgeStyle + badge + RST +
        DIM + skill.description + RST,
      )
    }
    visibleIdx++
  }

  while (overlayLines.length < 4 + listRows) overlayLines.push('')

  // Footer — full-width inverse hint bar.
  const footerText = ' enter toggle \u00b7 \u2190\u2192 filter \u00b7 esc close'
  overlayLines.push(INVERSE + padRight(' ' + footerText, W) + RST)

  // The 14-row overlay begins immediately above the five-row composer.
  lines.push(...chatHeader())
  lines.push('  ' + ACC + '\u2605 ' + RST + FG + 'which skills are loaded?' + RST)

  const inputBar = chatInput('/skill')
  const targetStart = rows - inputBar.length - overlayLines.length
  while (lines.length < targetStart) lines.push('')
  lines.push(...overlayLines)
  lines.push(...inputBar)

  while (lines.length < rows) lines.push('')
  return lines.slice(0, rows).map(line => padRight(trunc(line, W), W)).join('\n')
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

  // Step 1: all skills, "all" selected, row 0 selected.
  const screen1 = buildSkillOverlay(allSkills, '', 0, true, rows)

  // Step 2: filtered by "domain" — only domain-web and domain-cli.
  const filteredSkills: SkillEntry[] = [
    { name: 'domain-web', active: true,  description: 'Web service development',  visible: true },
    { name: 'domain-cli', active: false, description: 'CLI tools and terminal apps', visible: true },
  ]
  const screen2 = buildSkillOverlay(filteredSkills, 'domain', 0, true, rows)

  return [
    {
      title: 'Type /skill',
      narration: 'From normal chat, type /skill in the composer and press Enter to open the skill hub.',
      screen: commandEntryScreen(rows, '/skill'),
    },
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
