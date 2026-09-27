import { create } from 'zustand'

// Visibility is session scoped; hiding the preview never changes controller ownership.
export const useComputerPreview = create<{
  session: string | null
  requestedSession: string | null
  requestShow: (session: string) => void
  syncController: (status: { session: string; enabled: boolean }) => void
  show: (session: string) => void
  hide: () => void
}>((set) => ({
  session: null,
  requestedSession: null,
  requestShow: session => set({ session: null, requestedSession: session }),
  syncController: status => set(state => !status.enabled
    ? { session: null, requestedSession: null }
    : state.requestedSession === status.session
      ? { session: status.session, requestedSession: null }
      : state),
  show: session => set({ session, requestedSession: null }),
  hide: () => set({ session: null, requestedSession: null }),
}))
