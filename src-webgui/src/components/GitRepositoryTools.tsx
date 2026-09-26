import { useCallback, useEffect, useRef, useState } from 'react'
import { ArrowDown, ArrowUp, RefreshCw } from 'lucide-react'
import { useKoma, type BranchInfo } from '../store/koma'
import type { GitAction, GitToolTab, RebaseStep } from '../lib/gitWorkbench'
import {
  Actions,
  Button,
  Confirm,
  MessageInput,
  Note,
  openCommit,
  setGitDirty,
  type GitWork,
} from './gitWorkbenchShared'
import { Field, Select, TextInput } from './panels/form'
import { CodePane } from './GitCodePane'
import { ConflictBanner } from './ConflictBanner'
import { GraphContextMenu } from './GraphContextMenu'

type Remote = { name: string; url: string; pushUrl: string }
function ListRow({
  selected,
  children,
  onClick,
}: {
  selected: boolean
  children: React.ReactNode
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`flex w-full items-center gap-2 px-3 py-1.5 text-left font-mono text-[12px] ${selected ? 'bg-koma-hover text-koma-accent' : 'text-koma-fg hover:bg-koma-hover'}`}
    >
      {children}
    </button>
  )
}
function useRemotes(work: GitWork) {
  const [remotes, setRemotes] = useState<Remote[]>([])
  const { run } = work
  const reload = useCallback(async () => {
    const data = await run<{ entries: Remote[] }>({ kind: 'remotes' })
    if (data) setRemotes(data.entries)
  }, [run])
  useEffect(() => {
    void reload()
  }, [reload])
  return { remotes, reload }
}
function RemoteSelect({
  value,
  onChange,
  remotes,
}: {
  value: string
  onChange: (v: string) => void
  remotes: Remote[]
}) {
  return (
    <Select
      value={value}
      onChange={onChange}
      options={[
        { value: '', label: 'Choose remote…' },
        ...remotes.map((r) => ({ value: r.name, label: r.name })),
      ]}
    />
  )
}
export function GitBranchesView({
  tab,
  work,
}: {
  tab: GitToolTab
  work: GitWork
}) {
  const branches = useKoma((s) => s.branches)
  const refresh = useKoma((s) => s.refreshBranches)
  const [selection, setSelection] = useState(tab.target ?? '')
  const [newName, setNewName] = useState('')
  const [force, setForce] = useState(false)
  const [confirm, setConfirm] = useState<{
    action: GitAction
    message: string
  }>()
  const { remotes } = useRemotes(work)
  useEffect(() => {
    if (work.active) refresh()
  }, [refresh, work.active, tab.root])
  const entries = branches.filter((b) => b.kind !== 'tag')
  const selected = entries.find((b) => `${b.kind}:${b.name}` === selection)
  const pick = (b: BranchInfo) => {
    setSelection(`${b.kind}:${b.name}`)
    setNewName(b.name)
    setForce(false)
    setConfirm(undefined)
  }
  const remote =
    selected?.kind === 'remote'
      ? [...remotes]
          .sort((a, b) => b.name.length - a.name.length)
          .find((r) => selected.name.startsWith(r.name + '/'))?.name
      : undefined
  const remove = () => {
    if (!selected) return
    setConfirm({
      action: {
        kind: 'branchDelete',
        name: remote ? selected.name.slice(remote.length + 1) : selected.name,
        remote,
        force,
      },
      message: remote
        ? `Delete branch ${selected.name} from remote ${remote}? This changes the remote repository.`
        : `Delete local branch ${selected.name}${force ? ', including unmerged work' : ''}?`,
    })
  }
  return (
    <>
      <Actions>
        <Button disabled={work.busy || !work.active} onClick={refresh}>
          <RefreshCw size={12} />
          Refresh
        </Button>
      </Actions>
      <div className="flex min-h-0 flex-1 flex-wrap overflow-auto">
        <div className="min-w-48 flex-1 border-r border-koma-border">
          {entries.map((b) => (
            <ListRow
              key={`${b.kind}:${b.name}`}
              selected={selected === b}
              onClick={() => pick(b)}
            >
              <span className="min-w-0 flex-1 truncate">{b.name}</span>
              <span className="text-[10px] text-koma-dim">
                {b.isCurrent ? 'current' : b.worktreePath ? 'worktree' : b.kind}
              </span>
            </ListRow>
          ))}
        </div>
        <div className="min-w-56 flex-1">
          {!selected ? (
            <Note>Select a branch.</Note>
          ) : (
            <>
              <Note>
                {selected.name}
                {selected.worktreePath ? ` · ${selected.worktreePath}` : ''}
              </Note>
              {selected.kind === 'local' && (
                <>
                  <Field label="Rename branch">
                    <TextInput
                      value={newName}
                      disabled={work.busy}
                      onChange={(e) => setNewName(e.target.value)}
                    />
                  </Field>
                  <Actions>
                    <Button
                      disabled={
                        work.busy ||
                        !work.active ||
                        !newName.trim() ||
                        newName === selected.name
                      }
                      onClick={async () => {
                        if (
                          await work.run({
                            kind: 'branchRename',
                            name: selected.name,
                            newName,
                          })
                        ) {
                          setSelection(`local:${newName}`)
                          refresh()
                        }
                      }}
                    >
                      Rename
                    </Button>
                  </Actions>
                  <label className="flex gap-2 px-3 py-2 text-[11px] text-koma-dim">
                    <input
                      type="checkbox"
                      checked={force}
                      onChange={(e) => {
                        setForce(e.target.checked)
                        setConfirm(undefined)
                      }}
                      disabled={
                        work.busy ||
                        selected.isCurrent ||
                        !!selected.worktreePath
                      }
                    />
                    Force delete unmerged branch
                  </label>
                </>
              )}
              <Actions>
                <Button
                  disabled={
                    work.busy ||
                    !work.active ||
                    selected.isCurrent ||
                    !!selected.worktreePath ||
                    (selected.kind === 'remote' && !remote)
                  }
                  onClick={remove}
                >
                  Delete{' '}
                  {selected.kind === 'remote' ? 'on remote' : 'local branch'}
                </Button>
              </Actions>
              {(selected.isCurrent || selected.worktreePath) && (
                <Note>
                  Check out another branch, or release its worktree, before
                  deleting this branch.
                </Note>
              )}
            </>
          )}
        </div>
      </div>
      {confirm && (
        <Confirm
          message={confirm.message}
          busy={work.busy || !work.active}
          onCancel={() => setConfirm(undefined)}
          onConfirm={async () => {
            if (await work.run(confirm.action)) {
              setConfirm(undefined)
              setSelection('')
              refresh()
            }
          }}
        />
      )}
    </>
  )
}
export function GitTagsView({ tab, work }: { tab: GitToolTab; work: GitWork }) {
  const tags = useKoma((s) => s.branches).filter((b) => b.kind === 'tag')
  const refresh = useKoma((s) => s.refreshBranches)
  const { remotes } = useRemotes(work)
  const [tag, setTag] = useState(tab.target ?? '')
  const [name, setName] = useState('')
  const [target, setTarget] = useState(tab.commit ?? 'HEAD')
  const [annotated, setAnnotated] = useState(false)
  const [message, setMessage] = useState('')
  const [remote, setRemote] = useState('')
  const [confirm, setConfirm] = useState<{
    action: GitAction
    message: string
  }>()
  useEffect(() => {
    if (work.active) refresh()
  }, [refresh, work.active])
  const existing = tags.some((t) => t.name === tag)
  const act = (action: GitAction, message: string) =>
    setConfirm({ action, message })
  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <Actions>
        <Button disabled={work.busy || !work.active} onClick={refresh}>
          <RefreshCw size={12} />
          Refresh
        </Button>
      </Actions>
      <div className="flex flex-wrap border-b border-koma-border">
        <div className="max-h-64 min-w-48 flex-1 overflow-auto border-r border-koma-border">
          {tags.length ? (
            tags.map((t) => (
              <ListRow
                key={t.name}
                selected={tag === t.name}
                onClick={() => {
                  setTag(t.name)
                  setConfirm(undefined)
                }}
              >
                {t.name}
              </ListRow>
            ))
          ) : (
            <Note>No local tags.</Note>
          )}
        </div>
        <div className="min-w-56 flex-1">
          <Note>{existing ? tag : 'Select a tag to delete or push.'}</Note>
          <Actions>
            <Button
              disabled={work.busy || !work.active || !existing}
              onClick={() =>
                act(
                  { kind: 'tagDelete', name: tag },
                  `Delete local tag ${tag}?`,
                )
              }
            >
              Delete local tag
            </Button>
          </Actions>
          <Field label="Remote">
            <RemoteSelect
              value={remote}
              onChange={(v) => {
                setRemote(v)
                setConfirm(undefined)
              }}
              remotes={remotes}
            />
          </Field>
          <Actions>
            <Button
              disabled={work.busy || !work.active || !existing || !remote}
              onClick={() =>
                act(
                  { kind: 'tagPush', name: tag, remote },
                  `Push tag ${tag} to ${remote}?`,
                )
              }
            >
              Push tag
            </Button>
            <Button
              disabled={work.busy || !work.active || !existing || !remote}
              onClick={() =>
                act(
                  { kind: 'tagDelete', name: tag, remote },
                  `Delete tag ${tag} from ${remote}? The local tag remains.`,
                )
              }
            >
              Delete remote tag
            </Button>
          </Actions>
        </div>
      </div>
      {confirm && (
        <Confirm
          message={confirm.message}
          busy={work.busy || !work.active}
          onCancel={() => setConfirm(undefined)}
          onConfirm={async () => {
            if (await work.run(confirm.action)) {
              setConfirm(undefined)
              refresh()
            }
          }}
        />
      )}
      <Field label="Create tag">
        <TextInput
          placeholder="Tag name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          disabled={work.busy}
        />
      </Field>
      <Field label="Commit">
        <TextInput
          value={target}
          onChange={(e) => setTarget(e.target.value)}
          disabled={work.busy}
        />
      </Field>
      <label className="flex gap-2 px-3 py-2 text-[11px] text-koma-dim">
        <input
          type="checkbox"
          checked={annotated}
          onChange={(e) => setAnnotated(e.target.checked)}
          disabled={work.busy}
        />
        Annotated tag
      </label>
      {annotated && (
        <Field label="Annotation">
          <MessageInput
            value={message}
            onChange={setMessage}
            disabled={work.busy}
          />
        </Field>
      )}
      <Actions>
        <Button
          disabled={
            work.busy ||
            !work.active ||
            !name.trim() ||
            !target.trim() ||
            (annotated && !message.trim())
          }
          onClick={async () => {
            if (
              await work.run({
                kind: 'tagCreate',
                name,
                target,
                message: annotated ? message : undefined,
              })
            ) {
              setTag(name)
              setName('')
              setMessage('')
              refresh()
            }
          }}
        >
          Create {annotated ? 'annotated' : 'lightweight'} tag
        </Button>
      </Actions>
    </div>
  )
}
export function GitRemotesView({ work }: { work: GitWork }) {
  const { remotes, reload } = useRemotes(work)
  const [selected, setSelected] = useState<string>()
  const [name, setName] = useState('')
  const [url, setUrl] = useState('')
  const [pushUrl, setPushUrl] = useState('')
  const [remove, setRemove] = useState(false)
  const edit = (r?: Remote) => {
    setSelected(r?.name)
    setName(r?.name ?? '')
    setUrl(r?.url ?? '')
    setPushUrl(r?.pushUrl ?? '')
    setRemove(false)
  }
  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <Actions>
        <Button
          disabled={work.busy || !work.active}
          onClick={() => void reload()}
        >
          <RefreshCw size={12} />
          Refresh
        </Button>
        <Button disabled={work.busy} onClick={() => edit()}>
          Add remote
        </Button>
      </Actions>
      <div className="max-h-56 overflow-auto border-b border-koma-border">
        {remotes.map((r) => (
          <ListRow
            key={r.name}
            selected={selected === r.name}
            onClick={() => edit(r)}
          >
            <span>{r.name}</span>
            <span className="truncate text-koma-dim">{r.url}</span>
          </ListRow>
        ))}
      </div>
      <Field label={selected ? 'Remote name' : 'New remote name'}>
        <TextInput
          value={name}
          onChange={(e) => setName(e.target.value)}
          disabled={work.busy}
        />
      </Field>
      <Field label="Fetch URL">
        <TextInput
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          disabled={work.busy}
        />
      </Field>
      {selected && (
        <Field label="Push URL">
          <TextInput
            value={pushUrl}
            onChange={(e) => setPushUrl(e.target.value)}
            disabled={work.busy}
          />
        </Field>
      )}
      <Actions>
        <Button
          disabled={
            work.busy ||
            !work.active ||
            !name.trim() ||
            !url.trim() ||
            (!!selected && !pushUrl.trim())
          }
          onClick={async () => {
            const action: GitAction = selected
              ? {
                  kind: 'remoteEdit',
                  name: selected,
                  newName: name,
                  url,
                  pushUrl,
                }
              : { kind: 'remoteAdd', name, url }
            if (await work.run(action)) {
              await reload()
              setSelected(name)
              if (!selected) setPushUrl(url)
            }
          }}
        >
          {selected ? 'Save remote' : 'Add remote'}
        </Button>
        {selected && (
          <Button
            disabled={work.busy || !work.active}
            onClick={() => setRemove(true)}
          >
            Remove remote
          </Button>
        )}
      </Actions>
      {remove && selected && (
        <Confirm
          message={`Remove remote ${selected} and its local tracking refs? This does not delete the hosted repository.`}
          busy={work.busy || !work.active}
          onCancel={() => setRemove(false)}
          onConfirm={async () => {
            if (await work.run({ kind: 'remoteRemove', name: selected })) {
              edit()
              await reload()
            }
          }}
        />
      )}
      <Note>
        URLs are saved to this repository. Fetch runs only when you request it.
      </Note>
    </div>
  )
}

type Stash = { oid: string; ref: string; message: string }
export function GitStashesView({
  tab,
  work,
}: {
  tab: GitToolTab
  work: GitWork
}) {
  const [entries, setEntries] = useState<Stash[]>([])
  const [selected, setSelected] = useState<Stash>()
  const [patch, setPatch] = useState('')
  const [message, setMessage] = useState('')
  const [untracked, setUntracked] = useState(false)
  const [operation, setOperation] = useState<'apply' | 'pop' | 'drop'>()
  const detailSequence = useRef(0)
  const { run } = work
  const reload = useCallback(async () => {
    const data = await run<{ entries: Stash[] }>({ kind: 'stashes' })
    if (data) setEntries(data.entries)
  }, [run])
  useEffect(() => {
    void reload()
  }, [reload])
  const pick = async (stash: Stash) => {
    const seq = ++detailSequence.current
    setSelected(stash)
    setPatch('')
    setOperation(undefined)
    const data = await run<{ patch: string }>({
      kind: 'stashInspect',
      oid: stash.oid,
    })
    if (data && seq === detailSequence.current) setPatch(data.patch)
  }
  return (
    <>
      <div className="flex-none border-b border-koma-border">
        <Field label="New stash">
          <TextInput
            placeholder="Stash message (optional)"
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            disabled={work.busy}
          />
        </Field>
        <Actions>
          <label className="mr-auto flex items-center gap-2 text-[11px] text-koma-dim">
            <input
              type="checkbox"
              checked={untracked}
              onChange={(e) => setUntracked(e.target.checked)}
              disabled={work.busy}
            />
            Include untracked files
          </label>
          <Button
            disabled={work.busy || !work.active}
            onClick={async () => {
              if (await run({ kind: 'stashCreate', message, untracked })) {
                setMessage('')
                await reload()
              }
            }}
          >
            Create stash
          </Button>
          <Button
            disabled={work.busy || !work.active}
            onClick={() => void reload()}
          >
            <RefreshCw size={12} />
          </Button>
        </Actions>
      </div>
      <div className="max-h-48 flex-none overflow-auto border-b border-koma-border">
        {entries.length ? (
          entries.map((e) => (
            <ListRow
              key={`${e.ref}:${e.oid}`}
              selected={selected?.oid === e.oid}
              onClick={() => void pick(e)}
            >
              <span className="flex-none text-koma-dim">{e.ref}</span>
              <span className="truncate">{e.message}</span>
            </ListRow>
          ))
        ) : (
          <Note>No stashes.</Note>
        )}
      </div>
      {selected && (
        <>
          <Actions>
            <span className="mr-auto text-[11px] text-koma-dim">
              {selected.ref} · {selected.oid.slice(0, 8)}
            </span>
            {(['apply', 'pop', 'drop'] as const).map((op) => (
              <Button
                key={op}
                disabled={
                  work.busy ||
                  !work.active ||
                  !entries.some((e) => e.oid === selected.oid)
                }
                onClick={() => setOperation(op)}
              >
                {op === 'apply' ? 'Apply' : op === 'pop' ? 'Pop' : 'Drop'}
              </Button>
            ))}
          </Actions>
          {operation && (
            <Confirm
              message={`${operation === 'drop' ? 'Permanently drop' : operation === 'pop' ? 'Apply and remove' : 'Apply'} ${selected.ref} (${selected.oid.slice(0, 8)})?${operation === 'pop' ? ' Conflicts keep the stash.' : ''}`}
              busy={work.busy || !work.active}
              onCancel={() => setOperation(undefined)}
              onConfirm={async () => {
                if (
                  await run({
                    kind: 'stashAction',
                    oid: selected.oid,
                    operation,
                  })
                ) {
                  setOperation(undefined)
                  if (operation !== 'apply') {
                    setSelected(undefined)
                    setPatch('')
                  }
                  await reload()
                }
              }}
            />
          )}
          <div className="min-h-0 flex-1">
            <CodePane value={patch} path="stash.diff" tabId={tab.id} />
          </div>
        </>
      )}
    </>
  )
}

type ReflogEntry = { oid: string; selector: string; message: string }
export function GitReflogView({ work }: { work: GitWork }) {
  const [entries, setEntries] = useState<ReflogEntry[]>([])
  const [more, setMore] = useState(false)
  const [selected, setSelected] = useState<ReflogEntry>()
  const [name, setName] = useState('')
  const [menu, setMenu] = useState<{ x: number; y: number; oid: string }>()
  const { run } = work
  const load = useCallback(
    async (skip: number) => {
      const data = await run<{ entries: ReflogEntry[]; hasMore: boolean }>({
        kind: 'reflog',
        skip,
      })
      if (data) {
        setEntries((s) => (skip ? [...s, ...data.entries] : data.entries))
        setMore(data.hasMore)
      }
    },
    [run],
  )
  useEffect(() => {
    void load(0)
  }, [load])
  return (
    <>
      <Actions>
        <Button
          disabled={work.busy || !work.active}
          onClick={() => void load(0)}
        >
          <RefreshCw size={12} />
          Refresh
        </Button>
      </Actions>
      <Note>
        Local history of reference movements. Recover a commit into a new
        branch, or open its existing graph actions to reset.
      </Note>
      {selected && (
        <div className="border-y border-koma-border">
          <Field label={`Recover ${selected.oid.slice(0, 8)} as new branch`}>
            <TextInput
              value={name}
              placeholder="Branch name"
              onChange={(e) => setName(e.target.value)}
              disabled={work.busy}
            />
          </Field>
          <Actions>
            <Button
              disabled={work.busy || !work.active || !name.trim()}
              onClick={async () => {
                if (await run({ kind: 'recover', oid: selected.oid, name })) {
                  setName('')
                  work.setError(null)
                }
              }}
            >
              Create recovery branch
            </Button>
            <Button
              disabled={!work.active}
              onClick={() => openCommit(selected.oid)}
            >
              Open commit
            </Button>
          </Actions>
        </div>
      )}
      <div className="min-h-0 flex-1 overflow-auto">
        {entries.map((e, i) => (
          <div
            key={`${e.selector}:${i}`}
            onContextMenu={(event) => {
              if (work.active) {
                event.preventDefault()
                setMenu({ x: event.clientX, y: event.clientY, oid: e.oid })
              }
            }}
          >
            <ListRow selected={selected === e} onClick={() => setSelected(e)}>
              <span className="flex-none text-koma-dim">
                {e.oid.slice(0, 8)}
              </span>
              <span className="min-w-0 flex-1 truncate">{e.message}</span>
              <span className="truncate text-[10px] text-koma-dim">
                {e.selector}
              </span>
            </ListRow>
          </div>
        ))}
        {more && (
          <Actions>
            <Button
              disabled={work.busy || !work.active}
              onClick={() => void load(entries.length)}
            >
              Load more
            </Button>
          </Actions>
        )}
      </div>
      {menu && work.active && (
        <GraphContextMenu
          x={menu.x}
          y={menu.y}
          target={{ kind: 'commit', sha: menu.oid }}
          onClose={() => setMenu(undefined)}
        />
      )}
    </>
  )
}

type RebasePlan = {
  base: string
  head: string
  branch: string
  steps: RebaseStep[]
}
export function GitRebaseView({
  tab,
  work,
}: {
  tab: GitToolTab
  work: GitWork
}) {
  const [plan, setPlan] = useState<RebasePlan>()
  const [steps, setSteps] = useState<RebaseStep[]>([])
  const [confirm, setConfirm] = useState(false)
  const [finished, setFinished] = useState(false)
  const [submitted, setSubmitted] = useState(false)
  const [attempted, setAttempted] = useState(false)
  const [reloadConfirm, setReloadConfirm] = useState(false)
  const inProgress = useKoma((s) => s.git.inProgress)
  const paused = work.active && inProgress === 'rebase'
  const editingDisabled = work.busy || !work.active || paused || submitted
  const [backup, setBackup] = useState('')
  const { run } = work
  const dirty =
    !finished &&
    !submitted &&
    !!plan &&
    JSON.stringify(plan.steps) !== JSON.stringify(steps)
  useEffect(() => {
    setGitDirty(tab.id, dirty)
  }, [tab.id, dirty])
  const reload = useCallback(async () => {
    const data = await run<RebasePlan>({
      kind: 'rebasePlan',
      base: tab.target!,
    })
    if (data) {
      setPlan(data)
      setSteps(data.steps)
      setFinished(false)
      setSubmitted(false)
      setAttempted(false)
      setConfirm(false)
    }
    setReloadConfirm(false)
  }, [run, tab.target])
  useEffect(() => {
    void reload()
  }, [reload])
  useEffect(() => {
    if (attempted && paused) {
      setSubmitted(true)
      setConfirm(false)
    }
  }, [attempted, paused])
  const move = (index: number, delta: number) => {
    const next = [...steps]
    ;[next[index], next[index + delta]] = [next[index + delta], next[index]]
    setSteps(next)
    setConfirm(false)
  }
  const edit = (index: number, values: Partial<RebaseStep>) => {
    setSteps((s) => s.map((v, i) => (i === index ? { ...v, ...values } : v)))
    setConfirm(false)
  }
  const invalid =
    steps.find((s) => s.action !== 'drop')?.action === 'squash' ||
    steps.some(
      (s) =>
        (s.action === 'reword' || s.action === 'squash') && !s.message.trim(),
    )
  return (
    <>
      {work.active && <ConflictBanner />}
      <Actions>
        <Button
          disabled={work.busy || !work.active || paused}
          onClick={() => (dirty ? setReloadConfirm(true) : void reload())}
        >
          <RefreshCw size={12} />
          Reload plan
        </Button>
      </Actions>
      {reloadConfirm && (
        <Confirm
          message="Discard the edited plan and reload this commit range?"
          busy={work.busy}
          onCancel={() => setReloadConfirm(false)}
          onConfirm={() => void reload()}
        />
      )}
      {submitted && (
        <Note>
          {paused
            ? 'The plan is in Git’s sequencer. Resolve the conflicts, then Continue or Abort above.'
            : 'The sequence has ended. Open the graph to review the result, or reload to prepare another plan.'}
        </Note>
      )}
      <Note>
        {plan
          ? `${plan.branch}: ${plan.base.slice(0, 8)} (excluded) → ${plan.head.slice(0, 8)}. Oldest commit first.`
          : 'Loading linear commit range…'}{' '}
        A recovery reference is created before rewriting.
      </Note>
      {finished ? (
        <Note>
          Rebase completed. {backup && `Recovery reference: ${backup}`}
        </Note>
      ) : (
        <>
          <div className="min-h-0 flex-1 overflow-auto">
            {steps.map((step, i) => (
              <div
                key={step.oid}
                className="border-b border-koma-border py-1.5"
              >
                <Actions>
                  <Button
                    title="Move earlier"
                    disabled={editingDisabled || i === 0}
                    onClick={() => move(i, -1)}
                  >
                    <ArrowUp size={12} />
                  </Button>
                  <Button
                    title="Move later"
                    disabled={editingDisabled || i === steps.length - 1}
                    onClick={() => move(i, 1)}
                  >
                    <ArrowDown size={12} />
                  </Button>
                  <Select
                    disabled={editingDisabled}
                    value={step.action}
                    options={['pick', 'reword', 'squash', 'drop'].map(
                      (value) => ({ value, label: value }),
                    )}
                    onChange={(v) => {
                      if (!editingDisabled)
                        edit(i, { action: v as RebaseStep['action'] })
                    }}
                  />
                  <span className="font-mono text-[11px] text-koma-dim">
                    {step.oid.slice(0, 8)}
                  </span>
                  <span
                    className={`min-w-0 flex-1 truncate text-[12px] text-koma-fg ${step.action === 'drop' ? 'line-through opacity-40' : ''}`}
                  >
                    {step.message.split('\n')[0]}
                  </span>
                </Actions>
                {(step.action === 'reword' || step.action === 'squash') && (
                  <Field
                    label={
                      step.action === 'squash'
                        ? 'Message for combined commit'
                        : 'New commit message'
                    }
                  >
                    <MessageInput
                      value={step.message}
                      onChange={(message) => edit(i, { message })}
                      disabled={editingDisabled}
                    />
                  </Field>
                )}
              </div>
            ))}
          </div>
          {invalid && (
            <Note>
              The first retained commit cannot be squashed. Reword and squash
              require a message.
            </Note>
          )}
          <Actions>
            <Button
              disabled={editingDisabled || !plan || invalid}
              onClick={() => setConfirm(true)}
            >
              Start rebase
            </Button>
          </Actions>
          {confirm && plan && (
            <Confirm
              message={`Rewrite ${plan.branch} with these ${steps.length} steps? Published commits will get new identities. Dropped commits remain available through the recovery reference.`}
              label="Rebase"
              busy={work.busy || !work.active}
              onCancel={() => setConfirm(false)}
              onConfirm={async () => {
                setAttempted(true)
                const data = await run<{ backup?: string }>({
                  kind: 'rebaseRun',
                  base: plan.base,
                  head: plan.head,
                  steps,
                })
                setConfirm(false)
                if (data) {
                  setFinished(true)
                  setBackup(data.backup ?? '')
                  setGitDirty(tab.id, false)
                }
              }}
            />
          )}
        </>
      )}
    </>
  )
}
