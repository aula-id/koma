import { createFileRoute } from '@tanstack/react-router'
import { TuiTutorial } from '../../../components/TuiTutorial'
import { getStoreSteps } from '../../../demos/store-tutorial'

export const Route = createFileRoute('/_docs/tui/commands-store')({
  component: () => (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">/store</h1>
      <p className="mb-6 text-koma-fg">Browse the public KomaRun extension catalogue and install an extension after connecting KomaRun OAuth.</p>
      <TuiTutorial steps={getStoreSteps(24)} />
    </article>
  ),
})
