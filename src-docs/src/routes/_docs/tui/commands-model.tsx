import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getCommandsModelSteps } from '../../../demos/commands-model-tutorial'

export const Route = createFileRoute('/_docs/tui/commands-model')({
  component: CommandsModelPage,
})

function CommandsModelPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Command: /model</h1>
      <p className="mb-6 text-koma-fg">
        The <code className="text-koma-fg">/model</code> command switches the
        active model for a specific role or agent — right from the chat input.
        No need to navigate to settings.
      </p>

      <TuiTutorial steps={getCommandsModelSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Quick Switch</strong>{' '}
          Type <code className="text-koma-fg">/model main</code> to pick a model
          for the primary coding role. The picker shows all configured models
          with their IDs.
        </p>
        <p>
          <strong className="text-koma-accent">Inherit</strong>{' '}
          The first option in the picker is “(inherit global)” — this removes the
          session override and uses the globally configured model for that role.
        </p>
        <p>
          <strong className="text-koma-accent">Per-Agent</strong>{' '}
          Type <code className="text-koma-fg">/model {'<agent>'}</code> to
          override the model for a specific sub-agent. This is useful when you
          want a sub-agent to use a cheaper or faster model.
        </p>
        <p>
          <strong className="text-koma-accent">Roles</strong>{' '}
          Available roles: main (primary coder), awareness (context), planner
          (task decomposition), compactor (context management), safeguard
          (safety checks).
        </p>
      </div>
    </article>
  )
}
