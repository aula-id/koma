import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/tui/commands-new')({
  component: () => (
    <CommandPage
      name="/new"
      description="Spawn a new session and swap to it. The current session keeps running in the background."
      details={<>
        <p>By default, <code className="text-koma-fg">/new</code> creates a fresh session with a clean conversation history and swaps to it. Your previous session continues running — you can return to it with <code className="text-koma-fg">/resume</code>.</p>
        <p>If you want to close the current session instead, use <code className="text-koma-fg">/new kill</code>.</p>
        <p>To start a session on a remote host, use <code className="text-koma-fg">/new remote</code>.</p>
      </>}
      variants={[
        { label: '/new', desc: 'Spawn session, keep current running' },
        { label: '/new kill', desc: 'Spawn session and close current one' },
        { label: '/new remote', desc: 'Start session on a saved remote host' },
      ]}
    />
  ),
})
