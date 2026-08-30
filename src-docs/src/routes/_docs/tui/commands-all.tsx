import { createFileRoute } from '@tanstack/react-router'

const commands = [
  { name: '/new', desc: 'Spawn a new session, swap to it' },
  { name: '/new kill', desc: 'Spawn and close current session' },
  { name: '/new remote', desc: 'Start session on remote host' },
  { name: '/resume', desc: 'Open session hub' },
  { name: '/resume remote', desc: 'Resume on remote host' },
  { name: '/mode', desc: 'Cycle or explicitly set agent mode' },
  { name: '/effort', desc: 'Set reasoning effort' },
  { name: '/free', desc: 'Toggle koma-free' },
  { name: '/internet', desc: 'Toggle simple/full internet' },
  { name: '/settings', desc: 'Edit settings' },
  { name: '/agents', desc: 'Agent definitions' },
  { name: '/mcp', desc: 'MCP server management' },
  { name: '/extension', desc: 'Manage extensions' },
  { name: '/store', desc: 'Extension marketplace' },
  { name: '/security', desc: 'Security daemon panel' },
  { name: '/remote', desc: 'Remote SSH hosts' },
  { name: '/task', desc: 'Sub-agents panel / task runner' },
  { name: '/model', desc: 'Switch session / agent model' },
  { name: '/bash', desc: 'Bash jobs (FG + background)' },
  { name: '/todo', desc: 'Session task list' },
  { name: '/skill', desc: 'Load/unload agent skills' },
  { name: '/attach', desc: 'Attach screenshot' },
  { name: '/cd', desc: 'Change working directory' },
  { name: '/adddir', desc: 'Add workspace root' },
  { name: '/compact', desc: 'Summarize & compact conversation' },
  { name: '/clear', desc: 'Clear chat history' },
  { name: '/usage', desc: 'Cost & token dashboard' },
  { name: '/rename', desc: 'Rename session' },
  { name: '/select', desc: 'Dump history to terminal' },
  { name: '/help', desc: 'List commands' },
  { name: '/quit', desc: 'Quit koma' },
]

export const Route = createFileRoute('/_docs/tui/commands-all')({
  component: () => (
    <article>
      <h1 className="mb-2 text-2xl font-bold text-koma-accent">All Commands</h1>
      <p className="mb-6 text-koma-dim">Every slash command available in koma, in display order.</p>
      <div className="overflow-hidden rounded-md border border-koma-border">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-koma-border bg-koma-panel text-left">
              <th className="px-4 py-2 font-semibold text-koma-fg">Command</th>
              <th className="px-4 py-2 font-semibold text-koma-fg">Description</th>
            </tr>
          </thead>
          <tbody>
            {commands.map((c) => (
              <tr key={c.name} className="border-b border-koma-border/50 last:border-0 hover:bg-koma-hover">
                <td className="px-4 py-1.5 font-mono text-koma-accent">{c.name}</td>
                <td className="px-4 py-1.5 text-koma-dim">{c.desc}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </article>
  ),
})
