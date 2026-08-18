import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/gui/oauth')({
  component: TutorialGuiOAuthPage,
})

function TutorialGuiOAuthPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">
        Tutorial: OAuth
      </h1>
      <p className="mb-6 text-koma-fg">
        This tutorial walks through connecting a provider through OAuth in
        the GUI — signing in via your browser without managing API keys.
        Supported providers include koma.run, Codex, Claude, Kilo Code,
        xAI, and Command Code.
      </p>

      <div className="space-y-4 text-sm leading-relaxed text-koma-dim">
        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Step 1: Open the Connector Panel</h3>
          <p>
            Click the Connector icon in the activity bar, then click{' '}
            <strong className="text-koma-accent">Add Provider</strong>. Choose a
            named provider (e.g. Codex, Claude, koma.run).
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Step 2: Start the OAuth Flow</h3>
          <p>
            The Connector opens a browser-based sign-in page for the selected
            provider. A status indicator in the side panel shows{' '}
            <strong className="text-koma-accent">Waiting for authentication...</strong>
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Step 3: Complete Sign-In</h3>
          <p>
            In your browser, sign in with the provider's credentials. For
            device-code flows, the GUI displays a code to enter. For browser
            redirect flows, approval is captured automatically. Some providers
            may show a manual-copy fallback if the redirect cannot reach the
            GUI.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Step 4: Confirm the Connection</h3>
          <p>
            After approval, the Connector panel updates to show the new
            connection with provider name, email, and a Connected status.
            The associated models become available in the model picker.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Step 5: Select a Model</h3>
          <p>
            Open the model picker in the chat panel. The provider's models
            are listed with the provider name as a prefix. Select one to
            make it the active session model.
          </p>
        </div>
      </div>
    </article>
  )
}
