import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/tui/commands-free')({
  component: () => (
    <CommandPage
      name="/free"
      description="Toggle this session to use koma-free (keyless models hosted by koma)."
      details={
        <p>Switches the session to use the free tier with no API key required. Models are hosted by koma. You can switch back to your paid provider anytime via <code className="text-koma-fg">/model</code> or <code className="text-koma-fg">/settings</code>.</p>
      }
    />
  ),
})
