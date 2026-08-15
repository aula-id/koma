import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getKeyboardShortcutsSteps } from '../../../demos/keyboard-shortcuts-tutorial'

export const Route = createFileRoute('/_docs/docs/keyboard-shortcuts')({
  component: KeyboardShortcutsPage,
})

function KeyboardShortcutsPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Keyboard Shortcuts</h1>
      <p className="mb-6 text-koma-fg">
        Global keyboard shortcuts available in the chat input. These work across all
        modes and can be used to quickly navigate, edit, and control the agent.
      </p>

      <TuiTutorial steps={getKeyboardShortcutsSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Quick Actions</strong>{' '}
          Enter sends messages, Tab switches context, Ctrl+R regenerates the last
          response, and Ctrl+E edits it.
        </p>
        <p>
          <strong className="text-koma-accent">Navigation</strong>{' '}
          Up/Down scrolls the transcript, Esc closes overlays or cancels operations,
          and pressing Esc twice rewinds to edit a previous message.
        </p>
        <p>
          <strong className="text-koma-accent">Panel Shortcuts</strong>{' '}
          $ opens the sub-agents panel, # toggles code selection mode, and
          Ctrl+F searches chat history.
        </p>
      </div>
    </article>
  )
}
