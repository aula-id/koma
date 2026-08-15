import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getTaskSteps } from '../../../demos/task-tutorial'

export const Route = createFileRoute('/_docs/docs/commands-task')({
  component: CommandsTaskPage,
})

function CommandsTaskPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Command: /task</h1>
      <p className="mb-6 text-koma-fg">
        The <code className="text-koma-fg">/task</code> command opens the sub-agents
        panel — a bordered overlay showing all running and completed sub-agents. With
        arguments, it dispatches a task to a named agent instead.
      </p>

      <TuiTutorial steps={getTaskSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Two-Pane View</strong>{' '}
          The left pane lists sub-agents with #id, name, status tag, and label.
          The right pane shows the selected agent's live progress or result.
        </p>
        <p>
          <strong className="text-koma-accent">Status Tags</strong>{' '}
          running, done, killed, error. Running agents show a live transcript
          stream; completed agents show their final answer.
        </p>
        <p>
          <strong className="text-koma-accent">Kill / Background</strong>{' '}
          Press Ctrl+X to kill the selected agent. Press Ctrl+B to background
          a running agent so the main chat can continue.
        </p>
      </div>
    </article>
  )
}
