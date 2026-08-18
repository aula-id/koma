import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getFirstRunSteps } from '../../../demos/first-run-tutorial'

export const Route = createFileRoute('/_docs/tui/')({
  component: TuiPage,
})

function TuiPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Terminal UI</h1>
      <p className="mb-6 text-koma-fg">
        The TUI runs in your terminal with ratatui. It provides a full-featured
        chat interface with markdown rendering, tool call visualization, and
        sub-agent management.
      </p>

      <h2 className="mb-3 text-lg font-semibold text-koma-fg">First Run Walkthrough</h2>
      <p className="mb-4 text-sm text-koma-dim">
        Step through the first-run experience below. Each step shows the actual
        TUI screen with an explanation of what you're seeing. Use the{' '}
        <span className="text-koma-fg">arrow keys</span> or click{' '}
        <span className="text-koma-accent">Next</span> to advance.
      </p>

      <TuiTutorial steps={getFirstRunSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Header</strong>{' '}
          Shows the version number and current mode (normal, auto, plan, yolo).
          The mode colour changes to reflect the current operating mode.
        </p>
        <p>
          <strong className="text-koma-accent">Input</strong>{' '}
          The <code className="text-koma-fg">[$]</code> prompt is the multiline
          composer. Type any task — koma reads code, edits files, runs commands,
          and verifies results.
        </p>
        <p>
          <strong className="text-koma-accent">Status Bar</strong>{' '}
          Shows the current phase (ready, thinking, streaming) with a comet
          animation while working. Token counts and cost update in real time.
        </p>
        <p>
          <strong className="text-koma-accent">Tool Calls</strong>{' '}
          File reads, edits, and shell commands appear inline in the transcript
          with status indicators (&#x2713; success, &#x2717; failed).
        </p>
      </div>
    </article>
  )
}
