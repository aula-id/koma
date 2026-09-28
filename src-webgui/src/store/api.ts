import type { KomaState } from './state'

export type StoreGet = () => KomaState
export type StoreSet = {
  (partial: KomaState | Partial<KomaState> | ((state: KomaState) => KomaState | Partial<KomaState>), replace?: false): void
  (state: KomaState | ((state: KomaState) => KomaState), replace: true): void
}
