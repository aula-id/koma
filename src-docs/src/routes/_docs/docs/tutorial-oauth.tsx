import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getTutorialOAuthSteps } from '../../../demos/tutorial-oauth'

export const Route = createFileRoute('/_docs/docs/tutorial-oauth')({
  component: TutorialOAuthPage,
})

function TutorialOAuthPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">
        Tutorial: OAuth
      </h1>
      <p className="mb-6 text-koma-fg">
        Connect a provider through OAuth — sign in via your browser without
        managing API keys. This works for koma.run, Codex, Claude, Kilo Code,
        xAI, and Command Code.
      </p>

      <TuiTutorial steps={getTutorialOAuthSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Connection List</strong>{' '}
          Shows all linked accounts with provider name, email, and status.
          Connected entries are ready to use; expired tokens need re-authentication.
        </p>
        <p>
          <strong className="text-koma-accent">Provider Picker</strong>{' '}
          Six browser-based providers (Codex, Kilo Code, koma.run, xAI, Claude,
          Command Code) plus two paste-token fallbacks for manual entry.
        </p>
        <p>
          <strong className="text-koma-accent">Browser Flow</strong>{' '}
          Koma shows the browser URL in the body. Press c to copy it, or o to
          open it. Approve in the browser and the token is captured automatically.
        </p>
        <p>
          <strong className="text-koma-accent">Delete</strong>{' '}
          On the OAuth connections page, Ctrl+X twice disconnects a provider
          and removes its bound models.
        </p>
      </div>
    </article>
  )
}
