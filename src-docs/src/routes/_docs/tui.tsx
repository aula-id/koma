import { createFileRoute, Outlet } from '@tanstack/react-router'

function TuiLayout() {
  return <Outlet />
}

export const Route = createFileRoute('/_docs/tui')({
  component: TuiLayout,
})
