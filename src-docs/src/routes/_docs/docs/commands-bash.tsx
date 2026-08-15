import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getBashSteps } from '../../../demos/bash-tutorial'

export const Route = createFileRoute('/_docs/docs/commands-bash')({
  component: CommandsBashPage,
})

function CommandsBashPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Command: /bash</h1>
      <p className="mb-6 text-koma-fg">
        The <code className="text-koma-fg">/bash</code> command opens the background
        jobs panel — a bordered overlay showing all running and completed background
        bash commands. View output, check status, or kill jobs from here.
      </p>

      <TuiTutorial steps={getBashSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Two-Pane View</strong>{' '}
          The left pane lists jobs with id, status, and elapsed time. The right
          pane shows the selected job's command, status, and output tail.
        </p>
        <p>
          <strong className="text-koma-accent">Job Status</strong>{' '}
          Running jobs appear in accent color. Finished or errored jobs appear
          dim. Each row shows elapsed time in seconds.
        </p>
        <p>
          <strong className="text-koma-accent">Kill Job</strong>{' '}
          Select a running job and press Ctrl+X to terminate it. The output
          pane shows the last lines of output before termination.
        </p>
      </div>
    </article>
  )
}
