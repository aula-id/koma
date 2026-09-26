// Repository-bound request/reply transport. It does not depend on the store.
export type RebaseStep = { oid: string; action: 'pick' | 'reword' | 'squash' | 'drop'; message: string }
export type GitAction =
  | { kind: 'diff'; path: string; staged: boolean; commit?: string; oldPath?: string }
  | { kind: 'stageLines'; path: string; staged: boolean; token: string; lines: number[] }
  | { kind: 'conflict' | 'blame'; path: string }
  | { kind: 'resolve'; path: string; token: string; content?: string; choice?: string; stage: boolean }
  | { kind: 'commitMessage' | 'stashes' | 'remotes' }
  | { kind: 'amend'; message: string; head: string }
  | { kind: 'branchRename'; name: string; newName: string }
  | { kind: 'branchDelete'; name: string; force: boolean; remote?: string }
  | { kind: 'tagCreate'; name: string; target: string; message?: string }
  | { kind: 'tagDelete'; name: string; remote?: string }
  | { kind: 'tagPush'; name: string; remote: string }
  | { kind: 'stashCreate'; message: string; untracked: boolean }
  | { kind: 'stashInspect'; oid: string }
  | { kind: 'stashAction'; oid: string; operation: 'apply' | 'pop' | 'drop' }
  | { kind: 'reflog'; skip: number }
  | { kind: 'recover'; oid: string; name: string }
  | { kind: 'remoteAdd'; name: string; url: string }
  | { kind: 'remoteEdit'; name: string; newName: string; url: string; pushUrl: string }
  | { kind: 'remoteRemove'; name: string }
  | { kind: 'rebasePlan'; base: string }
  | { kind: 'rebaseRun'; base: string; head: string; steps: RebaseStep[] }
export type GitRequest = { root: string; requestId: string; action: GitAction }
export type GitReply = { root: string; requestId: string; data: unknown; error?: string | null }
export type GitToolTab = {
  kind: 'gitTool'; id: string; root: string; title: string
  view: 'diff' | 'resolve' | 'rebase' | 'branches' | 'tags' | 'stashes' | 'reflog' | 'blame' | 'remotes'
  path?: string; target?: string; staged?: boolean; commit?: string; oldPath?: string; dirty?: boolean
}
const pending = new Map<string, { root: string; resolve: (data: unknown) => void; reject: (e: Error) => void; timer: ReturnType<typeof setTimeout> }>()
let sequence = 0
export function gitRequest<T>(root: string, action: GitAction, send: (r: GuiReq) => void): Promise<T> {
  const requestId = `workbench-${Date.now()}-${++sequence}`
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(requestId)
      reject(new Error('Git has not replied. Refresh repository status before retrying; the operation may still be running.'))
    }, 180_000)
    pending.set(requestId, { root, resolve: (data) => resolve(data as T), reject, timer })
    try { send({ r: 'GitWorkbench', request: { root, requestId, action } }) }
    catch (e) { clearTimeout(timer); pending.delete(requestId); reject(e) }
  })
}
export function receiveGitReply(reply: GitReply) {
  const waiter = pending.get(reply.requestId)
  if (!waiter || waiter.root !== reply.root) return
  clearTimeout(waiter.timer)
  pending.delete(reply.requestId)
  if (reply.error) waiter.reject(new Error(reply.error))
  else waiter.resolve(reply.data)
}
export function cancelGitRequests() {
  for (const waiter of pending.values()) {
    clearTimeout(waiter.timer)
    waiter.reject(new Error('Session changed. Refresh the repository before continuing.'))
  }
  pending.clear()
}
