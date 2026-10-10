import { create } from 'zustand'

// Visibility is session scoped. Hiding the preview never changes controller ownership.
// An explicit dismiss stays closed across later enabled statuses; a new grant opens it.
export const useComputerPreview = create<{
  session: string | null
  requestedSession: string | null
  dismissedSession: string | null
  requestShow: (session: string) => void
  syncController: (status: { session: string; enabled: boolean }) => void
  show: (session: string) => void
  hide: () => void
  dismiss: (session: string) => void
}>((set) => ({
  session: null,
  requestedSession: null,
  dismissedSession: null,
  requestShow: session => set({ session: null, requestedSession: session, dismissedSession: null }),
  syncController: status => set(state => {
    if (!status.enabled) return { session: null, requestedSession: null, dismissedSession: null }
    if (state.dismissedSession === status.session) {
      return state.requestedSession === null ? state : { requestedSession: null }
    }
    if (state.session === status.session && state.requestedSession === null) return state
    return { session: status.session, requestedSession: null }
  }),
  show: session => set({ session, requestedSession: null, dismissedSession: null }),
  hide: () => set({ session: null, requestedSession: null }),
  dismiss: session => set({ session: null, requestedSession: null, dismissedSession: session }),
}))
