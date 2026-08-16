import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getSkillSteps } from '../../../demos/skill-tutorial'

export const Route = createFileRoute('/_docs/tui/commands-skill')({
  component: CommandsSkillPage,
})

function CommandsSkillPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Command: /skill</h1>
      <p className="mb-6 text-koma-fg">
        The <code className="text-koma-fg">/skill</code> command opens the skill hub
        — a searchable overlay for loading and unloading agent skills. Skills are
        context bundles (crate docs, domain knowledge, coding guidelines) that enhance
        the agent's knowledge for the current session.
      </p>

      <TuiTutorial steps={getSkillSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Toggle</strong>{' '}
          Press Enter on a skill to load or unload it. Active skills show a
          green [active] badge and their names appear in accent color.
        </p>
        <p>
          <strong className="text-koma-accent">Filter</strong>{' '}
          Use the [X]all / [ ]active chip toggles to show all skills or only
          currently loaded ones. Type to search by name or description.
        </p>
        <p>
          <strong className="text-koma-accent">Skill Types</strong>{' '}
          Includes domain skills (web, CLI, embedded, ML), Rust concept skills
          (ownership, traits, concurrency), and tool skills (code-navigator,
          refactoring, testing).
        </p>
      </div>
    </article>
  )
}
