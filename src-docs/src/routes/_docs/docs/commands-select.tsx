import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/docs/commands-select')({
  component: () => (
    <CommandPage
      name="/select"
      description="Dump full chat history to the terminal for native copy."
      details={
        <p>Outputs the entire conversation to the terminal so you can select and copy it using your terminal's native selection. This is useful when you need to copy a large code block or output that's hard to select in the chat UI.</p>
      }
    />
  ),
})
