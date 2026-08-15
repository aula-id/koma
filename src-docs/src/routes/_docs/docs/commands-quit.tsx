import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/docs/commands-quit')({
  component: () => (
    <CommandPage
      name="/quit"
      description="Quit koma."
      shortcut="Ctrl+C"
      details={<>
        <p>Exits koma. If you have running sessions, they will be detached (kept alive in the background) and can be resumed later with <code className="text-koma-fg">/resume</code>.</p>
        <p>Aliases: <code className="text-koma-fg">/q</code>, <code className="text-koma-fg">/exit</code>.</p>
      </>}
    />
  ),
})
