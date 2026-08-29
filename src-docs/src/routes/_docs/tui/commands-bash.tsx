import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getBashSteps } from '../../../demos/bash-tutorial'

export const Route = createFileRoute('/_docs/tui/commands-bash')({
  component: CommandsBashPage,
})

function CommandsBashPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Command: /bash</h1>
      <p className="mb-6 text-koma-fg">
        The <code className="text-koma-fg">/bash</code> command opens the bash jobs
        panel — a bordered overlay of every shell job this session registered,
        including <strong className="text-koma-fg">foreground</strong> commands that
        still park the turn and true background jobs (
        <code className="text-koma-fg">run_in_background</code> or Ctrl+B-promoted).
        View output, check status, or kill jobs from here.
      </p>

      <TuiTutorial steps={getBashSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Two-Pane View</strong>{' '}
          The left pane lists jobs with id, status, and elapsed time. The right
          pane shows the selected job&apos;s command, status, and output tail.
        </p>
        <p>
          <strong className="text-koma-accent">Foreground vs background</strong>{' '}
          Default model <code className="text-koma-fg">bash</code> parks the main
          turn as a job (<code className="text-koma-fg">bash-N</code> appears here
          immediately). <code className="text-koma-fg">run_in_background: true</code>{' '}
          returns a job id without parking. While a FG job is running, composer{' '}
          <strong className="text-koma-accent">Ctrl+B</strong> promotes it to true
          background (turn resumes; process keeps running; completion nudge later).
        </p>
        <p>
          <strong className="text-koma-accent">Job Status</strong>{' '}
          Running jobs appear in accent color. Finished or errored jobs appear
          dim. Each row shows elapsed time in seconds.
        </p>
        <p>
          <strong className="text-koma-accent">Kill Job</strong>{' '}
          Select a running job and press Ctrl+X (or <code className="text-koma-fg">k</code>)
          to terminate it. Esc during a still-blocking FG job also kills it with
          the turn; already-detached jobs survive Esc.
        </p>
      </div>
    </article>
  )
}
