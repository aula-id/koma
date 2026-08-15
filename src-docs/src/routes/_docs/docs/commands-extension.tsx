import { createFileRoute } from '@tanstack/react-router'
import { TuiTutorial } from '../../../components/TuiTutorial'
import { getExtensionSteps } from '../../../demos/extension-tutorial'

export const Route = createFileRoute('/_docs/docs/commands-extension')({
  component: () => (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">/extension</h1>
      <p className="mb-6 text-koma-fg">Open the installed-extension manager to inspect extension metadata, screens, and removal confirmation.</p>
      <TuiTutorial steps={getExtensionSteps(24)} />
    </article>
  ),
})
