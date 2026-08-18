import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/tui/commands-clear')({
  component: () => (
    <CommandPage
      name="/clear"
      description="Clear the chat history (keeps system prompt + archive)."
      details={
        <p>Wipes the visible conversation from the chat, giving you a fresh start. The system prompt, archived context, and session state are preserved. The original conversation is still accessible in the archive.</p>
      }
    />
  ),
})
