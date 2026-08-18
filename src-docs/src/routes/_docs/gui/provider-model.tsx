import { createFileRoute } from '@tanstack/react-router'

import { GuiProviderModelTutorial } from '../../../components/GuiProviderModelTutorial'

export const Route = createFileRoute('/_docs/gui/provider-model')({
  component: TutorialGuiProviderModelPage,
})

function TutorialGuiProviderModelPage() {
  return <article className="gpm-page">
    <h1 className="mb-4 text-2xl font-bold text-koma-accent">Tutorial: Provider &amp; Model</h1>
    <p className="mb-6 text-koma-fg">Follow the GUI&apos;s Connector workflow from an API-key provider, through a global main model, to a usable session model. Every screen and value below is illustrative.</p>
    <GuiProviderModelTutorial />
    <div className="mt-8 space-y-3 text-sm text-koma-dim"><h3 className="text-base font-semibold text-koma-fg">About this path</h3><p>Connector also has a separate <strong className="text-koma-accent">OAuth</strong> catalogue and <strong className="text-koma-accent">Connect account</strong> flow. This guided path intentionally uses the API-key provider form so it can demonstrate credential configuration without showing or requesting a real account secret.</p><p>In the real GUI, choosing a provider fetches its live model catalogue. The model picker offers global models and creates a session-local main override when you select one.</p></div>
  </article>
}
