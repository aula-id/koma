import { useKoma, type ToastEntry } from '../store/koma'

export function showToast(text: string, kind: ToastEntry['kind'] = 'info') {
  useKoma.setState((state) => {
    const id = state.ui.toastSeq + 1
    return { ui: { ...state.ui, toastSeq: id, toast: { id, text, kind } } }
  })
}
