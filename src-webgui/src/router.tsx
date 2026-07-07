import { createRouter, createHashHistory } from '@tanstack/react-router'
import { routeTree } from './routes'

export const router = createRouter({ routeTree, history: createHashHistory() })

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
