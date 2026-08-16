import { createFileRoute } from '@tanstack/react-router'

import { GuiFirstRunTutorial } from '../../../components/GuiFirstRunTutorial'

export const Route = createFileRoute('/_docs/docs/tutorial-gui-first-run')({
  component: TutorialGuiFirstRunPage,
})

function TutorialGuiFirstRunPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Tutorial: First Run</h1>
      <p className="mb-6 text-koma-fg">
        Follow the desktop GUI&apos;s supported first-run path: configuration loads,
        you select a theme, choose Koma Free, and arrive at the session start screen.
      </p>

      <GuiFirstRunTutorial />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">What Happens Next</h3>
        <p>
          Koma Free creates the keyless provider and its main model. The real GUI
          leaves onboarding only after that configuration is saved by the host.
        </p>
        <p>
          The start screen is intentionally shown before chat: use <strong className="text-koma-accent">New session</strong>{' '}
          to choose a workspace, then the GUI can attach a session and render chat.
        </p>
      </div>
    </article>
  )
}
