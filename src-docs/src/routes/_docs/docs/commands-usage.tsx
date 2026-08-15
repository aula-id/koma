import { createFileRoute } from '@tanstack/react-router'

import { TuiTutorial } from '../../../components/TuiTutorial'
import { getUsageSteps } from '../../../demos/usage-tutorial'

export const Route = createFileRoute('/_docs/docs/commands-usage')({
  component: CommandsUsagePage,
})

function CommandsUsagePage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Command: /usage</h1>
      <p className="mb-6 text-koma-fg">
        The <code className="text-koma-fg">/usage</code> command opens the cost and
        token usage dashboard — a full-screen view with KPI metrics, heatmaps, model
        breakdowns, and role split analysis.
      </p>

      <TuiTutorial steps={getUsageSteps(24)} />

      <div className="mt-8 space-y-3 text-sm text-koma-dim">
        <h3 className="text-base font-semibold text-koma-fg">Key Details</h3>
        <p>
          <strong className="text-koma-accent">Global View</strong>{' '}
          Shows total cost, token counts, hourly heatmap, top models with bar
          charts, and main/sub role cost split. Switch ranges with 1-3 keys.
        </p>
        <p>
          <strong className="text-koma-accent">Session View</strong>{' '}
          Press Tab to switch to session-scoped stats: models used in this session,
          hourly activity heatmap, and session KPI totals.
        </p>
        <p>
          <strong className="text-koma-accent">Metric Toggle</strong>{' '}
          Press M to switch the heatmap and bar charts between cost and token
          views. The active metric is shown in the header.
        </p>
      </div>
    </article>
  )
}
