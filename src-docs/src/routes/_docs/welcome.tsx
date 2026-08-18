import { createFileRoute, Outlet } from '@tanstack/react-router'

function WelcomeLayout() {
  return <Outlet />
}

export const Route = createFileRoute('/_docs/welcome')({
  component: WelcomeLayout,
})
