import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getTutorialFirstRunSteps } from '../../../demos/tutorial-first-run'

export const Route = createFileRoute('/_docs/docs/tutorial-first-run')({
  component: TutorialFirstRunPage,
})

function TutorialFirstRunPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">
        Tutorial: First Run
      </h1>
      <p className="mb-6 text-koma-fg">
        The very first time you launch koma you see a three-way chooser.
        This tutorial walks through connecting a provider (OAuth sign-in) or
        a custom endpoint — from first launch to working chat.
      </p>

      <TuiTutorial steps={getTutorialFirstRunSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">What Happened</h3>
        <p>
          <strong className="text-koma-accent">Provider (OAuth)</strong>{' '}
          Choosing provider opens the OAuth flow — pick a provider, sign in
          in your browser, then choose a model. The connection and model
          are saved to your config automatically.
        </p>
        <p>
          <strong className="text-koma-accent">Custom</strong>{' '}
          Choosing custom lets you enter any OpenAI-compatible endpoint and
          API key directly — useful for local models or corporate proxies.
        </p>
        <p>
          <strong className="text-koma-accent">koma free</strong>{' '}
          Starts instantly with no key — free models hosted by koma.
          Switch to a paid provider anytime in /settings.
        </p>
      </div>
    </article>
  )
}
