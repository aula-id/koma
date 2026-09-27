import { create } from 'zustand'

// Visibility is session scoped; hiding the preview never changes controller ownership.
export const useComputerPreview = create<{
  session: string | null
  show: (session: string) => void
  hide: () => void
}>((set) => ({ session: null, show: session => set({ session }), hide: () => set({ session: null }) }))
