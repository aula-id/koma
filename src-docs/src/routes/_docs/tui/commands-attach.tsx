import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/tui/commands-attach')({
  component: () => (
    <CommandPage
      name="/attach"
      description="Attach a screenshot image to the next message."
      details={
        <p>Usage: <code className="text-koma-fg">/attach {'<path>'}</code> or <code className="text-koma-fg">/attach</code> (clipboard). Attaches a PNG image from the screenshots directory (or clipboard) to your next chat message. Useful for showing the agent a visual bug or UI state.</p>
      }
    />
  ),
})
