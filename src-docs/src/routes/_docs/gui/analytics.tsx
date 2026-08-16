import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/gui/analytics')({
  component: GuiAnalyticsPage,
})

function GuiAnalyticsPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Analytics</h1>
      <p className="mb-6 text-koma-fg">
        The Analytics panel provides a visual dashboard for tracking token
        usage, costs, and model performance across sessions.
      </p>

      <div className="space-y-4 text-sm leading-relaxed text-koma-dim">
        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Sidebar Usage Summary</h3>
          <p>
            The usage footer at the bottom of the GUI shows current session
            token counts and cost in real time. The Analytics panel expands
            this into a full dashboard.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Dashboard Ranges &amp; Scope</h3>
          <p>
            Select a time range: today, last 7 days, last 30 days, or a
            custom range. Scope the view to a single session or view all
            sessions combined.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Metric Filters</h3>
          <p>
            Filter the dashboard by model, role (main, sub-agent, awareness,
            planner), and provider. The charts update to reflect the
            selected filters.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">KPIs</h3>
          <p>
            Key performance indicators show total tokens, total cost, average
            response time, and cache hit rate for the selected range and
            filters.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Model Table</h3>
          <p>
            A sortable table lists each model with its token count, cost,
            average latency, and request count. Sort by any column to find
            the most used or most expensive model.
          </p>
        </div>

        <div>
          <h3 className="mb-1 text-base font-semibold text-koma-fg">Main vs. Sub-Agent Breakdown</h3>
          <p>
            A chart compares token usage between the main coding agent and
            sub-agents. This helps identify whether sub-agents are consuming
            a disproportionate share of the budget.
          </p>
        </div>
      </div>
    </article>
  )
}
