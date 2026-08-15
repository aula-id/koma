import { createFileRoute } from '@tanstack/react-router'
import { TuiTutorial } from '../../../components/TuiTutorial'
import { getSettingsAppearanceSteps } from '../../../demos/settings-appearance-tutorial'

export const Route = createFileRoute('/_docs/docs/settings-appearance')({ component: SettingsAppearancePage })

function SettingsAppearancePage() {
  return <article><h1 className="mb-4 text-2xl font-bold text-koma-accent">Settings: Appearance</h1><p className="mb-6 text-koma-fg">Open <code className="text-koma-fg">/settings</code> and press <span className="text-koma-accent">1</span> to choose the terminal palette.</p><TuiTutorial steps={getSettingsAppearanceSteps(24)} /></article>
}
