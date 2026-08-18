import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getTodoSteps } from '../../../demos/todo-tutorial'

export const Route = createFileRoute('/_docs/tui/commands-todo')({
  component: CommandsTodoPage,
})

function CommandsTodoPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Command: /todo</h1>
      <p className="mb-6 text-koma-fg">
        The <code className="text-koma-fg">/todo</code> command opens the task panel
        — a model-managed, read-only checklist shown above the input bar with a
        two-pane layout: task list on the left, detail on the right.
      </p>

      <TuiTutorial steps={getTodoSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Status Symbols</strong>{' '}
          ○ pending · ◐ in progress · ● completed · ⊘ cancelled. The agent uses the
          checklist tool to add, update, and complete items as it works.
        </p>
        <p>
          <strong className="text-koma-accent">Two-Pane View</strong>{' '}
          The left pane shows the task list with status symbols. The right pane shows
          detail for the selected item: status, priority, and content.
        </p>
        <p>
          <strong className="text-koma-accent">Controls</strong>{' '}
          Use ↑/↓ or k/j to navigate. Enter resets an unlocked non-pending item to
          pending, signalling the model to redo it; Esc closes the panel. The panel has
          no user add or delete controls.
        </p>
        <p>
          <strong className="text-koma-accent">Persistent</strong>{' '}
          Tasks are stored in <code className="text-koma-fg">memory/TODO.md</code> and
          survive across sessions. The panel auto-refreshes from disk.
        </p>
      </div>
    </article>
  )
}
