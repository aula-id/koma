import { createFileRoute, Outlet } from '@tanstack/react-router'

function DocsLayout() {
  return <Outlet />
}

export const Route = createFileRoute('/_docs/docs')({
  component: DocsLayout,
})
