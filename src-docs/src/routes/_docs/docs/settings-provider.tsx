import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getSettingsProviderSteps } from '../../../demos/settings-provider-tutorial'

export const Route = createFileRoute('/_docs/docs/settings-provider')({
  component: SettingsProviderPage,
})

function SettingsProviderPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Settings: Providers</h1>
      <p className="mb-6 text-koma-fg">
        Providers store your API connections (endpoint + key) so koma can talk to
        any OpenAI-compatible API. Open{' '}
        <code className="text-koma-fg">/settings</code> in the chat, then press{' '}
        <span className="text-koma-accent">3</span> to open the Providers page.
      </p>

      <TuiTutorial steps={getSettingsProviderSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Settings Menu</strong>{' '}
          The settings overlay appears above the input bar with five numbered categories.
          Press 1-5 to jump directly.
        </p>
        <p>
          <strong className="text-koma-accent">Provider Table</strong>{' '}
          Shows all saved providers with name, endpoint URL, API type, and masked
          key. Move to <code className="text-koma-fg">[ + add provider ]</code>{' '}
          and press <code className="text-koma-fg">Enter</code> to add, or press{' '}
          <code className="text-koma-fg">Ctrl+X</code> twice to delete the selected provider.
        </p>
        <p>
          <strong className="text-koma-accent">Add Form</strong>{' '}
          Enter a friendly name, the API endpoint, and your key. Press Save to
          persist — the provider then appears in the model configuration dropdown.
        </p>
      </div>
    </article>
  )
}
