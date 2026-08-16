import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getTutorialProviderModelSteps } from '../../../demos/tutorial-provider-model'

export const Route = createFileRoute('/_docs/tui/provider-model')({
  component: TutorialProviderModelPage,
})

function TutorialProviderModelPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">
        Tutorial: Provider &amp; Model
      </h1>
      <p className="mb-6 text-koma-fg">
        Add a manual provider (OpenAI-compatible endpoint + API key), create a
        model, assign a role, and learn how to switch models with /model — all
        from within an existing chat session.
      </p>

      <TuiTutorial steps={getTutorialProviderModelSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Providers</strong>{' '}
          Manual providers are always OpenAI-compatible wire. Give it a name,
          base URL, and API key. Anthropic/Codex/koma.run types are created
          only through OAuth.
        </p>
        <p>
          <strong className="text-koma-accent">Models</strong>{' '}
          Global models are persisted to config; local models are session-only.
          For endpoint-backed providers, typing in the Model field searches
          the provider's live catalogue.
        </p>
        <p>
          <strong className="text-koma-accent">Roles</strong>{' '}
          Roles: main (primary coder), awareness, planner, compactor, safeguard.
          Assign via the Settings model form or override per-session with /model.
        </p>
        <p>
          <strong className="text-koma-accent">/model</strong>{' '}
          /model shows current assignments and sub-commands. /model {'<role>'} swaps
          a role's model for the current session only.
        </p>
      </div>
    </article>
  )
}
