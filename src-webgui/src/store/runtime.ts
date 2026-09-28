import type { CodingSlice } from './coding'
import type { PushEnvelope } from './types/envelope'
import type { UiSlice } from './types/slices'

export type CodingHostView = { coding: CodingSlice; ui: UiSlice; replies: PushEnvelope[] }

/** Documents and editor layout retained per remote host, independent of chat. */
export const codingHostViews = new Map<string, CodingHostView>()
