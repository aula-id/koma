import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/tui/commands-adddir')({
  component: () => (
    <CommandPage
      name="/adddir"
      description="Add a directory to the workspace roots."
      details={
        <p>Usage: <code className="text-koma-fg">/adddir {'<path>'}</code>. Adds an additional directory to the session's workspace roots, allowing the agent to read and edit files outside the primary project directory.</p>
      }
    />
  ),
})
