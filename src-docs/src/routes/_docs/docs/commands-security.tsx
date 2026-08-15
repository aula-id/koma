import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/docs/commands-security')({
  component: () => (
    <CommandPage
      name="/security"
      description="Open the security daemon control panel."
      details={
        <p>Manages the security daemon that monitors agent actions for safety violations. View alerts, adjust policies, and review the audit log from this panel.</p>
      }
    />
  ),
})
