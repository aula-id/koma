import { createFileRoute, Outlet } from '@tanstack/react-router'

/** Legacy /docs layout — pass-through for redirect catchers. */
function DocsRedirectLayout() {
  return <Outlet />
}

export const Route = createFileRoute('/_docs/docs')({
  component: DocsRedirectLayout,
})
