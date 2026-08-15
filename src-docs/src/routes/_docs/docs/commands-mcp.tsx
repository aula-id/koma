import { createFileRoute } from '@tanstack/react-router'
import { CommandPage } from '../../../components/CommandPage'

export const Route = createFileRoute('/_docs/docs/commands-mcp')({
  component: () => (
    <CommandPage
      name="/mcp"
      description="Add, edit, or remove MCP (Model Context Protocol) servers."
      details={
        <p>MCP servers extend the agent's capabilities with custom tools. This command opens the MCP server management panel where you can configure server connections, test them, and manage which tools are available.</p>
      }
    />
  ),
})
