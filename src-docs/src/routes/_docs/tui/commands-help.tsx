import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getHelpSteps } from '../../../demos/help-tutorial'

export const Route = createFileRoute('/_docs/tui/commands-help')({
  component: CommandsHelpPage,
})

function CommandsHelpPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Command: /help</h1>
      <p className="mb-6 text-koma-fg">
        The <code className="text-koma-fg">/help</code> command opens a full-screen
        searchable reference of all available slash commands and keyboard shortcuts.
        You can filter by typing, select with arrow keys, and launch any command directly.
      </p>

      <TuiTutorial steps={getHelpSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Search</strong>{' '}
          Start typing to filter commands and shortcuts by name or description.
          The list updates live as you type.
        </p>
        <p>
          <strong className="text-koma-accent">Launch</strong>{' '}
          Press Enter on a selected command to execute it directly from the help screen.
        </p>
        <p>
          <strong className="text-koma-accent">Update Info</strong>{' '}
          The top of the help screen shows your current version and whether an update
          is available, with one-line install instructions.
        </p>
      </div>
    </article>
  )
}
