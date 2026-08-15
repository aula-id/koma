import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/docs/keyboard-shortcuts')({
  component: KeyboardShortcutsPage,
})

function KeyboardShortcutsPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Keyboard Shortcuts</h1>
      <p className="mb-6 text-koma-fg">
        Keyboard shortcuts are documented in the real <code className="text-koma-fg">/help</code>{' '}
        reference; there is no separate keyboard-shortcuts TUI page.
      </p>

      <div className="space-y-3 text-sm text-koma-dim">
        <p><strong className="text-koma-accent">Enter</strong> sends a message or runs a slash command; <strong className="text-koma-accent">Tab</strong> completes the selected command.</p>
        <p><strong className="text-koma-accent">Ctrl+R</strong> resends the last message while idle. <strong className="text-koma-accent">Ctrl+E</strong> toggles internet mode, and <strong className="text-koma-accent">Ctrl+J</strong> inserts a newline.</p>
        <p><strong className="text-koma-accent">Ctrl+V</strong> stages an image from the clipboard. <strong className="text-koma-accent">Esc</strong> interrupts while busy; Esc twice opens rewind editing.</p>
        <p><strong className="text-koma-accent">Up/Down/wheel</strong> scroll the transcript. On an empty composer, <strong className="text-koma-accent">$</strong> opens the sub-agents panel; Ctrl+X kills a selected bash job or sub-agent in its panel.</p>
      </div>
    </article>
  )
}
