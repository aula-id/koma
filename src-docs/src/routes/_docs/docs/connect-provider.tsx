import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getFirstRunSteps } from '../../../demos/first-run-tutorial'
import { getSettingsProviderSteps } from '../../../demos/settings-provider-tutorial'
import { getSettingsModelSteps } from '../../../demos/settings-model-tutorial'
import { getSettingsOAuthSteps } from '../../../demos/settings-oauth-tutorial'
import { getCommandsModelSteps } from '../../../demos/commands-model-tutorial'

export const Route = createFileRoute('/_docs/docs/connect-provider')({
  component: ConnectProviderPage,
})

function ConnectProviderPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Connect a Provider</h1>
      <p className="mb-6 text-koma-fg">
        There are three ways to get koma talking to a model. Each is shown below
        exactly as it appears in the terminal.
      </p>

      <h2 className="mb-2 mt-8 text-lg font-semibold text-koma-fg">1 · First run</h2>
      <p className="mb-4 text-koma-dim">
        The very first launch shows a three-way chooser.{' '}
        <span className="text-koma-accent">koma free</span> starts instantly with no
        key. <span className="text-koma-accent">provider</span> runs the OAuth
        sign-in (section 3).{' '}
        <span className="text-koma-accent">custom</span> opens the manual add-provider
        form (section 2).
      </p>
      <TuiTutorial steps={getFirstRunSteps(24).slice(0, 1)} />

      <h2 className="mb-2 mt-10 text-lg font-semibold text-koma-fg">
        2 · Add a provider and a model
      </h2>
      <p className="mb-4 text-koma-dim">
        From any chat, type <code className="text-koma-fg">/settings</code>, press{' '}
        <span className="text-koma-accent">3</span> for Providers, then{' '}
        <span className="text-koma-accent">Enter</span> on{' '}
        <span className="text-koma-fg">[ + add provider ]</span>. A manual provider is
        always an OpenAI-compatible endpoint — give it a name, base URL, and API key.
      </p>
      <TuiTutorial steps={getSettingsProviderSteps(24)} />

      <p className="mb-4 mt-8 text-koma-dim">
        Once a provider exists, press <span className="text-koma-accent">5</span> for
        Models and add one. Pick the provider, choose a model from its catalogue (or
        type an id), then assign a role.
      </p>
      <TuiTutorial steps={getSettingsModelSteps(24)} />

      <h2 className="mb-2 mt-10 text-lg font-semibold text-koma-fg">
        3 · Connect via OAuth
      </h2>
      <p className="mb-4 text-koma-dim">
        For koma.run, Codex, Claude, Kilo Code, xAI, or Command Code, OAuth is
        simpler than pasting keys. Open <code className="text-koma-fg">/settings</code>,
        press <span className="text-koma-accent">4</span>, and choose a provider. The
        same flow runs from the first-run{' '}
        <span className="text-koma-accent">provider</span> choice.
      </p>
      <TuiTutorial steps={getSettingsOAuthSteps(24)} />

      <h2 className="mb-2 mt-10 text-lg font-semibold text-koma-fg">
        4 · Switch models with /model
      </h2>
      <p className="mb-4 text-koma-dim">
        You don't have to open Settings to change a model.{' '}
        <code className="text-koma-fg">/model &lt;role&gt;</code> swaps the model for a
        role for the current session;{' '}
        <code className="text-koma-fg">/model agent &lt;name&gt;</code> changes a
        sub-agent's model.
      </p>
      <TuiTutorial steps={getCommandsModelSteps(24)} />
    </article>
  )
}
