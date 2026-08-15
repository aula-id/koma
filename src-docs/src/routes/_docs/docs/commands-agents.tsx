import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/docs/commands-agents')({
  component: () => (
    <CommandPage
      name="/agents"
      description="Create, modify, or delete agent definitions."
      details={<>
        <p>Opens the agents panel where you can define custom sub-agents with specific models, system prompts, and tool access. Each agent can be dispatched via <code className="text-koma-fg">/task {'<agent>'}</code>.</p>
      </>}
    />
  ),
})
