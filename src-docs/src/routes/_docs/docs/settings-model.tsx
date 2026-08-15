import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getSettingsModelSteps } from '../../../demos/settings-model-tutorial'

export const Route = createFileRoute('/_docs/docs/settings-model')({
  component: SettingsModelPage,
})

function SettingsModelPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Settings: Add Model</h1>
      <p className="mb-6 text-koma-fg">
        Models define which AI model koma uses for each role. Add global models
        (shared across sessions) or local models (session-only). Open{' '}
        <code className="text-koma-fg">/settings</code> then press{' '}
        <span className="text-koma-accent">5</span> to open the Models page.
      </p>

      <TuiTutorial steps={getSettingsModelSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Global vs Local</strong>{' '}
          Global models (marked with <code className="text-koma-fg">*</code>) are
          available in every session. Local models are session-only and don't
          persist after the session ends.
        </p>
        <p>
          <strong className="text-koma-accent">Filters</strong>{' '}
          The radio bar [X]all / [ ]local / [ ]global filters the table to show
          only models matching the selected scope.
        </p>
        <p>
          <strong className="text-koma-accent">Model Form</strong>{' '}
          Pick a provider, then search for a model ID or type it manually. For
          omnisearchable providers (like OpenRouter), the model field shows live
          search results as you type.
        </p>
        <p>
          <strong className="text-koma-accent">Roles</strong>{' '}
          Each model can be assigned roles: main (primary coder), awareness
          (context gathering), planner (task decomposition), or compactor
          (context management).
        </p>
      </div>
    </article>
  )
}
