import { createFileRoute, Outlet } from '@tanstack/react-router'

function GuiLayout() {
  return <Outlet />
}

export const Route = createFileRoute('/_docs/gui')({
  component: GuiLayout,
})
