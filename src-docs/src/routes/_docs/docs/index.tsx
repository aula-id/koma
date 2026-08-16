import { createFileRoute, Navigate } from '@tanstack/react-router'

function DocsIndexRedirect() {
  return <Navigate to="/welcome" />
}

export const Route = createFileRoute('/_docs/docs/')({
  component: DocsIndexRedirect,
})
