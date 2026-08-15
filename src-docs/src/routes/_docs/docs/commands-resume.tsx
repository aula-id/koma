import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getResumeSteps } from '../../../demos/resume-tutorial'

export const Route = createFileRoute('/_docs/docs/commands-resume')({
  component: CommandsResumePage,
})

function CommandsResumePage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Command: /resume</h1>
      <p className="mb-6 text-koma-fg">
        The <code className="text-koma-fg">/resume</code> command opens the session hub
        — a full-screen view split into two halves showing live "cooking" sessions and
        past "history" sessions. Swap between sessions, kill running ones, or delete
        old ones from disk.
      </p>

      <TuiTutorial steps={getResumeSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Cooking</strong>{' '}
          The top half shows live sessions with ● working / ○ ready status markers.
          The current foreground session is italic and underlined.
        </p>
        <p>
          <strong className="text-koma-accent">History</strong>{' '}
          The bottom half shows past sessions with relative timestamps. Type to
          filter by name. The focused pane gets an accent header rule.
        </p>
        <p>
          <strong className="text-koma-accent">Tab to Switch</strong>{' '}
          Press Tab to toggle focus between cooking and history panes. Use
          Ctrl+X to kill a cooking session or delete a history entry.
        </p>
      </div>
    </article>
  )
}
