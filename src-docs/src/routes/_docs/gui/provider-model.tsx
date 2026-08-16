import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/gui/provider-model')({
  component: TutorialGuiProviderModelPage,
})

function TutorialGuiProviderModelPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">
        Tutorial: Provider &amp; Model
      </h1>
      <p className="mb-6 text-koma-fg">
        This tutorial walks through adding a provider via the GUI Connector
        panel, configuring credentials, adding a model, and verifying it in
        the model picker.
      </p>

      <div className="space-y-4 text-sm leading-relaxed text-koma-dim">
        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Step 1: Open the Connector Panel</h3>
          <p>
            Click the Connector icon in the activity bar. The side panel
            shows the list of existing providers (empty on first run) and
            an <strong className="text-koma-accent">Add Provider</strong> button at the bottom.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Step 2: Add a Provider</h3>
          <p>
            Click <strong className="text-koma-accent">Add Provider</strong>. Choose
            the provider type: OpenAI-compatible (manual), or a named provider
            (Codex, Claude, Kilo Code, koma.run, xAI, Command Code). For
            manual providers, enter a name, base URL, and API key.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Step 3: Configure Credentials</h3>
          <p>
            For OpenAI-compatible providers, paste the API key and verify the
            endpoint URL. For named providers, the Connector redirects to the
            provider's OAuth flow (covered in the OAuth tutorial).
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Step 4: Add a Model</h3>
          <p>
            After the provider is saved, the model configuration form appears.
            Select the new provider from the dropdown. Type a model ID or
            search the provider's live catalogue. Assign a role (main,
            awareness, planner, compactor) and save.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Step 5: Verify in the Model Picker</h3>
          <p>
            In the chat panel, click the model picker dropdown. The newly
            added model should appear. Select it to make it the active model
            for the session.
          </p>
        </div>
      </div>
    </article>
  )
}
