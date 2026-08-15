import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/docs/commands-remote')({
  component: () => (
    <CommandPage
      name="/remote"
      description="Manage remote SSH hosts and sessions."
      details={
        <p>Add, edit, or remove saved remote hosts. Connect to a remote host to run sessions there — the agent gets full access to the remote filesystem and tools over SSH.</p>
      }
    />
  ),
})
