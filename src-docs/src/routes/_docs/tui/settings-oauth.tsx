import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getSettingsOAuthSteps } from '../../../demos/settings-oauth-tutorial'

export const Route = createFileRoute('/_docs/tui/settings-oauth')({
  component: SettingsOAuthPage,
})

function SettingsOAuthPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Settings: OAuth</h1>
      <p className="mb-6 text-koma-fg">
        OAuth lets you sign in to providers like koma.run, Codex, or Kilo Code
        through your browser — no need to manage API keys manually. Open{' '}
        <code className="text-koma-fg">/settings</code> then press{' '}
        <span className="text-koma-accent">4</span> to open the OAuth page.
      </p>

      <TuiTutorial steps={getSettingsOAuthSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Connection List</strong>{' '}
          Shows all linked accounts with provider name, email, and status.
          Connected entries are ready to use; expired tokens need re-authentication.
        </p>
        <p>
          <strong className="text-koma-accent">Provider Picker</strong>{' '}
          Eight providers available, including paste-token fallbacks for manual key
          entry when browser auth is unavailable.
        </p>
        <p>
          <strong className="text-koma-accent">Browser Flow</strong>{' '}
          Koma shows the browser URL in the OAuth body. Press{' '}
          <code className="text-koma-fg">c</code> to copy it (then the copy
          confirmation appears), or <code className="text-koma-fg">o</code> to
          open it. Approve in the browser and the token is captured.
        </p>
      </div>
    </article>
  )
}
