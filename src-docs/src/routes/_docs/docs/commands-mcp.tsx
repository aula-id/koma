import { createFileRoute } from '@tanstack/react-router'
import { TuiTutorial } from '../../../components/TuiTutorial'
import { getMcpSteps } from '../../../demos/mcp-tutorial'

export const Route = createFileRoute('/_docs/docs/commands-mcp')({ component: McpPage })

function McpPage() {
  return <article><h1 className="mb-4 text-2xl font-bold text-koma-accent">/mcp</h1><p className="mb-6 text-koma-fg">Add, edit, and remove global MCP (Model Context Protocol) servers. Unlike chat commands, <strong className="text-koma-accent">/mcp is available without an active session</strong>.</p><TuiTutorial steps={getMcpSteps(24)} /></article>
}
