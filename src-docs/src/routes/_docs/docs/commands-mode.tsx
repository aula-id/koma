import { createFileRoute } from '@tanstack/react-router'
import { TuiTutorial } from '../../../components/TuiTutorial'
import { getModeSteps } from '../../../demos/mode-tutorial'

export const Route = createFileRoute('/_docs/docs/commands-mode')({
  component: () => (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">/mode</h1>
      <p className="mb-6 text-koma-fg">Cycle or explicitly select the session’s Auto, Normal, Plan, SDLC, or armed YOLO mode.</p>
      <TuiTutorial steps={getModeSteps(24)} />
    </article>
  ),
})
