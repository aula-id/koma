import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/docs/commands-internet')({
  component: () => (
    <CommandPage
      name="/internet"
      description="Toggle internet mode (simple or full)."
      shortcut="Ctrl+E"
      details={<>
        <p>In <strong className="text-koma-accent">simple</strong> mode, the agent can search the web and fetch URLs using basic tools.</p>
        <p>In <strong className="text-koma-accent">full</strong> mode, the agent gets a headless browser for JavaScript-rendered pages and Cloudflare-protected sites.</p>
      </>}
      variants={[
        { label: '/internet', desc: 'Toggle between simple and full mode' },
        { label: '/internet simple', desc: 'Set to simple mode explicitly' },
        { label: '/internet full', desc: 'Set to full mode explicitly' },
      ]}
    />
  ),
})
