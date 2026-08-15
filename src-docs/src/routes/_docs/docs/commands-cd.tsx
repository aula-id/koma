import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/docs/commands-cd')({
  component: () => (
    <CommandPage
      name="/cd"
      description="Change the session working directory."
      details={
        <p>Usage: <code className="text-koma-fg">/cd {'<path>'}</code>. Changes the working directory for the current session. All subsequent file operations and commands will use this directory as their base.</p>
      }
    />
  ),
})
