import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/docs/commands-mode')({
  component: () => (
    <CommandPage
      name="/mode"
      description="Toggle between Normal and Auto tool approval modes."
      shortcut="N/A — type in chat"
      details={<>
        <p>In <strong className="text-koma-accent">Normal</strong> mode, the agent asks for your approval before running tools (file edits, shell commands, etc.).</p>
        <p>In <strong className="text-koma-accent">Auto</strong> mode, tools execute without confirmation — faster but less control.</p>
        <p>The current mode is shown in the header bar.</p>
      </>}
    />
  ),
})
