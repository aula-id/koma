import { createFileRoute } from '@tanstack/react-router'
import { TuiTutorial } from '../../../components/TuiTutorial'
import { getSettingsGeneralSteps } from '../../../demos/settings-general-tutorial'

export const Route = createFileRoute('/_docs/docs/settings-general')({ component: SettingsGeneralPage })

function SettingsGeneralPage() {
  return <article><h1 className="mb-4 text-2xl font-bold text-koma-accent">Settings: General</h1><p className="mb-6 text-koma-fg">Open <code className="text-koma-fg">/settings</code> and press <span className="text-koma-accent">2</span> to configure session-level general settings.</p><TuiTutorial steps={getSettingsGeneralSteps(24)} /></article>
}
