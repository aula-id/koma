import { createFileRoute } from '@tanstack/react-router'
import { TuiTutorial } from '../../../components/TuiTutorial'
import { getAgentsSteps } from '../../../demos/agents-tutorial'

export const Route = createFileRoute('/_docs/docs/commands-agents')({ component: AgentsPage })

function AgentsPage() {
  return <article><h1 className="mb-4 text-2xl font-bold text-koma-accent">/agents</h1><p className="mb-6 text-koma-fg">Create, modify, and delete custom sub-agent definitions. <strong className="text-koma-accent">An active session is required</strong> to open this dashboard. Agents can be dispatched with <code className="text-koma-fg">/task {'<agent>'}</code>.</p><TuiTutorial steps={getAgentsSteps(24)} /></article>
}
