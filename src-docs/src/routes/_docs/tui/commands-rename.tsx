import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/tui/commands-rename')({
  component: () => (
    <CommandPage
      name="/rename"
      description="Rename the current session."
      details={
        <p>Prompts for a new name for the current session. The name appears in the session hub and the header bar. Useful for organizing multiple concurrent sessions by topic.</p>
      }
    />
  ),
})
