import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type ClipboardEvent,
  type DragEvent,
  type KeyboardEvent,
} from 'react'
import {
  ArrowUp,
  Bold,
  Code,
  Frame,
  Heading2,
  Italic,
  Layers,
  Link,
  List,
  ListChecks,
  ListOrdered,
  Paperclip,
  Quote,
  Search,
  Square,
  Strikethrough,
  Braces,
  X,
} from 'lucide-react'
import { safeNoteUrl } from '../lib/markdownNote'
import { useKoma } from '../store/koma'
import {
  readCodingPathDragData,
} from '../lib/codingRef'
import { diagramViewForMermaid, mermaidTitle, splitDiagramMessage } from '../lib/diagramMermaid'
import {
  assignFreshMarkerInserts,
  imageMarker,
  listedComposerAttachments,
  trailingAttachmentMarkers,
  type MarkerInsertRow,
} from '../lib/composerMarkers'
import {
  formatPasteFence,
  PASTE_SOFT_MAX_BYTES,
  pasteByteLength,
  pasteMarker,
  shouldCollapsePaste,
  splitPasteMessage,
  type PastedBlock,
} from '../lib/pasteText'
import type { DiagramDoc } from '../lib/diagram'
import { splitDesignMessage } from '../lib/design'
import { ModelPicker } from './ModelPicker'
import { EffortPicker } from './EffortPicker'
import { ModeSelector } from './ModeSelector'
import { CatMascot } from './CatMascot'
import { DiagramSketch } from './DiagramVisual'
import {
  chipPayloadForAttachMarker,
  chipPayloadForFileRef,
  COMPOSER_ATTACHMENT_MIME,
  composerAttachmentPlain,
  hasComposerAttachmentDrag,
  writeComposerAttachmentDrag,
} from '../lib/composerIpc'
import { looksLikeComposerMarkdown } from '../lib/composerMarkdownPaste'
import { parseFileRefWire } from '../lib/composerChipOpen'
import { ComposerPasteEditOverlay } from './ComposerPasteEditOverlay'
import { LexicalMarkdownEditor, type LexicalEditorHandle } from './lexical/LexicalMarkdownEditor'

type DiagramChip = { id: string; title: string; mermaid: string; doc: DiagramDoc }
type DesignChip = { id: string; title: string; text: string }

function mintDiagramChipId(): string {
  return `d${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`
}

type LocalPaste = PastedBlock & { id: string; markerN?: number; label?: string }

function steerPreview(text: string): string {
  const pasted = splitPasteMessage(text)
  const split = splitDiagramMessage(pasted.prose)
  const designed = splitDesignMessage(split.prose)
  return [
    designed.prose,
    ...split.diagrams.map((item) => mermaidTitle(item.mermaid)),
    ...designed.designs.map((item) => item.title),
    ...pasted.pastes.map((item) => `Pasted Text #${item.n}`),
  ]
    .map((part) => part.replace(/\s+/g, ' ').trim())
    .filter(Boolean)
    .join(' · ')
}

function chipsFromMessage(text: string): { prose: string; chips: DiagramChip[]; designs: DesignChip[]; pastes: LocalPaste[] } {
  const pasted = splitPasteMessage(text)
  const split = splitDiagramMessage(pasted.prose)
  const designed = splitDesignMessage(split.prose)
  return {
    prose: designed.prose,
    chips: split.diagrams.map((item) => {
      const view = diagramViewForMermaid(item.mermaid)
      return { id: mintDiagramChipId(), title: mermaidTitle(item.mermaid), mermaid: item.mermaid, doc: view.doc }
    }),
    designs: designed.designs.map((item) => ({ id: mintDiagramChipId(), title: item.title, text: item.text })),
    pastes: pasted.pastes.map((item) => ({ ...item, id: mintDiagramChipId() })),
  }
}
// Build-time JSON import: src-misc/ lives outside the vite root (src-webgui/)
// but is the Rust side's single source of truth for this word list, so it's
// imported directly rather than copied — vite/rollup inlines it into the
// bundle at build time (fs.allow only gates the dev-server's HTTP file
// serving, not the module graph read via Node fs during build).
import wanderWords from '../../../src-misc/wanderer.json'

// Reads a File's bytes and resolves to a bare base64 string (no `data:` URL
// prefix) for the AttachFile GuiReq.
function readFileAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const result = reader.result as string
      const comma = result.indexOf(',')
      resolve(comma >= 0 ? result.slice(comma + 1) : result)
    }
    reader.onerror = () => reject(reader.error)
    reader.readAsDataURL(file)
  })
}

const COMPOSER_FIELD_CLASS =
  'm-0 box-border w-full whitespace-pre-wrap break-words p-0 text-[14px] leading-[22px] [overflow-wrap:anywhere] [tab-size:4]'

// Composer: message textarea + send, plus attach affordances (file-picker
// button, drag-drop onto the composer, clipboard-image paste) and the
// attached-file chip row. Split out of ChatView so the message-rendering
// region there (owned by a parallel branch) stays untouched by this work.
export function Composer() {
  const working = useKoma((s) => s.session.working)
  const attachments = useKoma((s) => s.session.attachments)
  const pendingSteer = useKoma((s) => s.session.pendingSteer)
  const refillComposer = useKoma((s) => s.refillComposer)
  const req = useKoma((s) => s.req)
  // Local selection index for the follow-ups list (client-only; daemon keeps
  // its own for the TUI). Clamped whenever the queue shrinks.
  const [steerSel, setSteerSel] = useState(0)
  const [steerFocus, setSteerFocus] = useState(false)
  const openOmniSearch = useKoma((s) => s.openOmniSearch)
  const omnisearchOpen = useKoma((s) => s.ui.omnisearchOpen)
  const composerInsert = useKoma((s) => s.ui.composerInsert)
  const consumeComposerInsert = useKoma((s) => s.consumeComposerInsert)
  const composerRefill = useKoma((s) => s.ui.composerRefill)
  const preserveTabsOnNextSession = useKoma((s) => s.ui.preserveTabsOnNextSession)
  const consumeComposerRefill = useKoma((s) => s.consumeComposerRefill)
  const pendingRewindIndex = useKoma((s) => s.ui.pendingRewindIndex)
  const clearRewind = useKoma((s) => s.clearRewind)
  const requestScrollBottom = useKoma((s) => s.requestScrollBottom)
  const [input, setInput] = useState('')
  const draftRef = useRef('')
  draftRef.current = input
  const [diagramChips, setDiagramChips] = useState<DiagramChip[]>([])
  const diagramChipsRef = useRef<DiagramChip[]>([])
  diagramChipsRef.current = diagramChips
  const [designChips, setDesignChips] = useState<DesignChip[]>([])
  const designChipsRef = useRef<DesignChip[]>([])
  designChipsRef.current = designChips
  const [localPastes, setLocalPastes] = useState<LocalPaste[]>([])
  const localPastesRef = useRef<LocalPaste[]>([])
  localPastesRef.current = localPastes
  // Fresh pastes wait here until the snapshot assigns `[Pasted Text #N]`.
  // Recalled fences are not queued: they already have a body to splice back.
  const markerInsertQueue = useRef<MarkerInsertRow[]>([])
  const seenAttachmentMarkers = useRef(new Set<string>())
  const submitArmed = useRef(false)
  const submitLock = useRef(false)
  const consumedAttachKeys = useRef(new Set<string>())
  const submitRef = useRef<() => void>(() => {})
  const [pasteTexts, setPasteTexts] = useState<Record<number, string>>({})
  const pasteTextsRef = useRef(pasteTexts)
  pasteTextsRef.current = pasteTexts
  const dirtyPastes = useRef(new Set<number>())
  const pasteBody = useKoma((s) => s.ui.pasteBody)
  const consumePasteBody = useKoma((s) => s.consumePasteBody)
  const sessionId = useKoma((s) => s.session.id)
  const diagramChatQueue = useKoma((s) => s.ui.diagramChatQueue)
  const consumeDiagramChatQueue = useKoma((s) => s.consumeDiagramChatQueue)
  const pendingComposerAttachmentInserts = useKoma((s) => s.ui.pendingComposerAttachmentInserts)
  const consumePendingComposerAttachmentInserts = useKoma((s) => s.consumePendingComposerAttachmentInserts)
  const designChatQueue = useKoma((s) => s.ui.designChatQueue)
  const consumeDesignChatQueue = useKoma((s) => s.consumeDesignChatQueue)
  const [dragOver, setDragOver] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const editorApi = useRef<LexicalEditorHandle | null>(null)
  const echoDraft = useRef(false)
  // Tokens this session has inserted via the omnisearch picker (bare, e.g.
  // "@downloads/file.pdf" or "@[1]downloads/file.pdf" — never with the
  // trailing space) — read by the overlay renderer + atomic chip-delete below
  // to decide which whitespace-delimited draft tokens are chip-eligible.
  // Add-only: stale entries that no longer appear in the text are harmless,
  // this is only ever membership-tested, never iterated positionally.
  const [pasteEditMarker, setPasteEditMarker] = useState<number | null>(null)
  // Mascot swap-on-send: bumped once per submit, telling CatMascot to pick a
  // different random cat. Otherwise it just keeps looping the current one.
  const [mascotSwap, setMascotSwap] = useState(0)
  // Thinking-bubble word, re-randomized every 1s while `working` is true (see
  // effect below). Empty when idle; the bubble itself is hidden via
  // `working` so a stale word never flashes on the next turn.
  const [thinkingWord, setThinkingWord] = useState('')
  const caretToEndRef = useRef(false)
  const caretTargetRef = useRef<number | null>(null)

  useEffect(() => {
    if (echoDraft.current) {
      echoDraft.current = false
      return
    }
    editorApi.current?.setMarkdown(input, caretToEndRef.current ? 'end' : undefined)
    caretToEndRef.current = false
  }, [input])

  // Thinking-bubble: while working, pick a fresh word immediately, then
  // re-randomize every 1000ms (avoiding an immediate repeat) until working
  // flips false — at which point the interval is torn down and the bubble
  // fades out via its own `working`-gated styling. Also cleaned up on unmount.
  useEffect(() => {
    if (!working) return
    const pick = (current: string) => {
      if (wanderWords.length <= 1) return wanderWords[0] ?? ''
      let next = current
      while (next === current) {
        next = wanderWords[Math.floor(Math.random() * wanderWords.length)]
      }
      return next
    }
    setThinkingWord((prev) => pick(prev))
    const id = window.setInterval(() => {
      setThinkingWord((prev) => pick(prev))
    }, 1000)
    return () => window.clearInterval(id)
  }, [working])

  // Consume one-shot omnisearch-pick signals: append the picked path into
  // the draft text (not an attachment — the daemon's ingest is image-only,
  // see attachFiles below), then ack so it doesn't re-fire on rerender.
  useEffect(() => {
    if (composerInsert === null) return
    const prefix = draftRef.current.length > 0 ? ' ' : ''
    const piece = `${prefix}${composerInsert}`
    const wire = piece.trim()
    if (wire.startsWith('@')) {
      editorApi.current?.insertChip(chipPayloadForFileRef(wire), {
        atEnd: true,
        trailingSpace: piece.endsWith(' '),
      })
    } else if (wire) {
      editorApi.current?.appendText(piece)
    }
    consumeComposerInsert()
    editorApi.current?.focus()
  }, [composerInsert, consumeComposerInsert])

  // A diagram reference is a drawing chip. The Mermaid stays on the chip until
  // send, which is the text the model actually receives.
  useEffect(() => {
    if (!diagramChatQueue.length) return
    setDiagramChips((prev) => [
      ...prev,
      ...diagramChatQueue.map((item) => ({ id: mintDiagramChipId(), ...item })),
    ])
    consumeDiagramChatQueue()
    editorApi.current?.focus()
  }, [diagramChatQueue, consumeDiagramChatQueue])

  const flushAttachmentInserts = () => {
    const assigned = assignFreshMarkerInserts(markerInsertQueue.current, seenAttachmentMarkers.current, attachments)
    const removals = assigned.filter((row) => row.cancelled)
    const keeps = assigned.filter((row) => !row.cancelled)
    for (const row of removals) {
      req({ r: 'RemoveAttachment', markerN: row.markerN, kind: row.kind })
    }
    if (keeps.length) {
      setLocalPastes((prev) =>
        prev.map((item) => {
          const q = markerInsertQueue.current.find((row) => row.id === item.id)
          if (!q || q.markerN == null) return item
          return { ...item, markerN: q.markerN, n: q.markerN }
        }),
      )
    }
  }

  useEffect(() => {
    if (!pendingComposerAttachmentInserts.length) return
    const pastes: LocalPaste[] = []
    for (const row of pendingComposerAttachmentInserts) {
      markerInsertQueue.current.push({ id: row.id, kind: row.kind, markerN: null, cancelled: false })
      if (row.kind === 'pasted_text' && row.text) {
        pastes.push({ id: row.id, n: 0, path: row.path ?? '', text: row.text, label: row.name })
      }
    }
    if (pastes.length) setLocalPastes((prev) => [...prev, ...pastes])
    consumePendingComposerAttachmentInserts()
    flushAttachmentInserts()
  }, [pendingComposerAttachmentInserts, consumePendingComposerAttachmentInserts, attachments, req])

  // A design reference is a chip. The html fence stays on the chip until send.
  useEffect(() => {
    if (!designChatQueue.length) return
    setDesignChips((prev) => [
      ...prev,
      ...designChatQueue.map((item) => ({ id: mintDiagramChipId(), ...item })),
    ])
    consumeDesignChatQueue()
    editorApi.current?.focus()
  }, [designChatQueue, consumeDesignChatQueue])

  useEffect(() => {
    if (!pasteBody) return
    if (!dirtyPastes.current.has(pasteBody.markerN)) {
      setPasteTexts((prev) => ({ ...prev, [pasteBody.markerN]: pasteBody.text }))
    }
    consumePasteBody()
  }, [pasteBody, consumePasteBody])

  // First mount must not wipe in-flight diagram attaches. Only a real session
  // change clears the draft. Run BEFORE the refill effect: the guided first-chat
  // template should be applied after the new session has cleared the old draft.
  const sessionSeenRef = useRef<string | null | undefined>(undefined)
  useEffect(() => {
    if (sessionSeenRef.current === undefined) {
      sessionSeenRef.current = sessionId
      return
    }
    if (sessionSeenRef.current === sessionId) return
    sessionSeenRef.current = sessionId
    setPasteTexts({})
    setLocalPastes([])
    setDiagramChips([])
    setDesignChips([])
    setInput('')
    dirtyPastes.current.clear()
    markerInsertQueue.current = []
    submitArmed.current = false
    submitLock.current = false
    consumedAttachKeys.current = new Set()
    consumePendingComposerAttachmentInserts()
    seenAttachmentMarkers.current = new Set()
  }, [sessionId, consumePendingComposerAttachmentInserts])

  // Rewind refills apply immediately. A guided Skills draft must wait for the
  // requested chat's Snapshot; otherwise the session-change reset erases it.
  useEffect(() => {
    if (composerRefill === null || preserveTabsOnNextSession) return
    const draft = chipsFromMessage(composerRefill)
    setDiagramChips(draft.chips)
    setDesignChips(draft.designs)
    setLocalPastes(draft.pastes)
    setInput(draft.prose)
    consumeComposerRefill()
  }, [composerRefill, consumeComposerRefill, preserveTabsOnNextSession])

  // Steer cap: the daemon queues at most 5 pending mid-turn submits; the 6th is
  // dropped host-side with a toast, so gate send at the cap.
  const atSteerCap = pendingSteer.length >= 5

  // Keep list selection in range as the queue shrinks (drain / remove / clear).
  useEffect(() => {
    if (pendingSteer.length === 0) {
      setSteerSel(0)
      setSteerFocus(false)
      return
    }
    setSteerSel((s) => Math.min(s, pendingSteer.length - 1))
  }, [pendingSteer.length])

  const editSteerAt = (index: number) => {
    const text = pendingSteer[index]
    if (text === undefined) return
    // GUI composer does not reconcile InputChanged — refill locally, then tell
    // the daemon to drop that queue slot (mirrors TUI EditSteer).
    refillComposer(text)
    req({ r: 'EditSteer', index })
    setSteerFocus(false)
    editorApi.current?.focus()
  }

  const removeSteerAt = (index: number) => {
    if (index < 0 || index >= pendingSteer.length) return
    req({ r: 'RemoveSteer', index })
  }

  // Up/Down composer history recall (client-side only, no daemon round-trip —
  // mirrors the TUI's hist_idx + input_stash, state/runtime.rs:887-906).
  // histIdxRef is -1 when not currently recalling; stashRef holds the
  // in-progress draft, restored once the user walks back past the newest
  // recalled entry. Both reset on any user edit (onChange) or send (submit).
  const histIdxRef = useRef(-1)
  const stashRef = useRef('')
  const stashChipsRef = useRef<DiagramChip[]>([])
  const stashDesignsRef = useRef<DesignChip[]>([])
  const stashPastesRef = useRef<LocalPaste[]>([])

  const resetHistory = () => {
    histIdxRef.current = -1
    stashRef.current = ''
    stashChipsRef.current = []
    stashDesignsRef.current = []
    stashPastesRef.current = []
  }

  const showRecalled = (text: string) => {
    const draft = chipsFromMessage(text)
    // The recalled draft replaces chips that were still waiting for a marker.
    // Cancel those rows so the late number is dropped instead of appearing
    // beside the recalled message.
    for (const row of markerInsertQueue.current) {
      if (row.markerN == null) row.cancelled = true
    }
    submitArmed.current = false
    setDiagramChips(draft.chips)
    setDesignChips(draft.designs)
    setLocalPastes(draft.pastes)
    setInput(draft.prose)
  }

  // Read straight off the store (no subscription — this only runs on an
  // Up/Down keypress, not every render) for user-authored, plain messages:
  // role==='user', no `kind` (excludes 'shell'/'bashNudge' — recalling a
  // "$ cmd\noutput" blob into the composer is garbage, a deliberate deviation
  // from the TUI which has no such rows), non-empty content. Oldest-first.
  const recallCandidates = () =>
    useKoma
      .getState()
      .session.messages.filter((m) => m.role === 'user' && !m.kind && m.content.trim() !== '')

  const toastError = (text: string) => {
    const id = useKoma.getState().ui.toastSeq + 1
    useKoma.setState((s) => ({
      ui: { ...s.ui, toastSeq: id, toast: { id, text, kind: 'error' } },
    }))
  }

  const submit = () => {
    if (submitLock.current) return
    // Wait only for attaches that are actually in flight. Orphan queue rows
    // (failed diagram image reads, session leftovers) must not swallow send.
    if (markerInsertQueue.current.some((row) => !row.cancelled && row.markerN == null)) {
      submitArmed.current = true
      return
    }
    const prose = editorApi.current?.getMarkdown() ?? input
    const mermaid = diagramChips.map((chip) => chip.mermaid).join('\n\n')
    const designs = designChips.map((chip) => chip.text).join('\n\n')
    const locals = localPastesRef.current
    const linked = new Set(locals.flatMap((item) => (item.markerN != null ? [item.markerN] : [])))
    const recalled = locals.filter((item) => item.markerN == null)
    const staged = attachments.filter((item) => item.kind === 'pasted_text' && !linked.has(item.markerN))
    const fences = recalled.map((item) => formatPasteFence(item)).join('\n\n')
    const trailingMarkers = trailingAttachmentMarkers(prose, attachments, locals, consumedAttachKeys.current)
    const text = [prose.trim() ? prose : '', mermaid, designs, fences, trailingMarkers.join(' ')].filter(Boolean).join('\n\n')
    const stagedPaste = staged.length > 0 || locals.some((item) => item.markerN != null)
    if (!text && !stagedPaste) return
    const bodies = [
      ...locals.map((item) => item.text),
      ...staged.flatMap((item) => {
        const body = pasteTextsRef.current[item.markerN]
        return body == null ? [] : [body]
      }),
    ]
    if (bodies.some((body) => pasteByteLength(body) > PASTE_SOFT_MAX_BYTES)) {
      toastError('Paste is larger than 2 MB')
      return
    }
    for (const item of locals) {
      if (item.markerN == null) continue
      req({ r: 'UpdatePaste', markerN: item.markerN, text: item.text })
    }
    for (const markerN of dirtyPastes.current) {
      const body = pasteTextsRef.current[markerN]
      if (body == null) continue
      if (!staged.some((item) => item.markerN === markerN)) continue
      req({ r: 'UpdatePaste', markerN, text: body })
    }
    dirtyPastes.current.clear()
    // While working, a submit is QUEUED daemon-side as a steer (not a new turn);
    // block it at the cap so we don't fire a request the daemon will just drop.
    if (atSteerCap) return
    resetHistory()
    // Staged rewind (edit pencil): fire RewindTo FIRST so the daemon aborts the
    // in-flight turn + truncates messages.json to before the edited message, THEN
    // Submit carries the edited text as the fresh turn. The single ordered IPC
    // channel guarantees RewindTo (abort + truncate) runs before Submit starts.
    if (pendingRewindIndex !== null) {
      req({ r: 'RewindTo', index: pendingRewindIndex })
      clearRewind()
    }
    // `!<cmd>` composer shell shortcut (TUI parity, controller/input/chat.rs:
    // 418-442): route to a no-model-round-trip shell run instead of a chat
    // submit — but ONLY while idle and with no staged image attachment (an
    // attachment makes no sense on a shell line; fall through to a normal
    // Submit instead, same as an empty `!cmd`). Deliberate deviation from the
    // TUI: it no-ops a `!` line while busy, but here we let it fall through to
    // a normal Submit so it queues as a steer like any other composer send,
    // rather than silently dropping the keystroke.
    if (!working && attachments.length === 0 && diagramChips.length === 0 && designChips.length === 0 && localPastesRef.current.length === 0 && text.startsWith('!')) {
      const cmd = text.slice(1).trim()
      if (cmd) {
        req({ r: 'Shell', cmd })
        setInput('')
        setMascotSwap((t) => t + 1)
        requestScrollBottom()
        return
      }
    }
    submitLock.current = true
    submitArmed.current = false
    echoDraft.current = true
    for (const item of attachments) consumedAttachKeys.current.add(`${item.kind}:${item.markerN}`)
    for (const item of locals) {
      if (item.markerN != null) consumedAttachKeys.current.add(`pasted_text:${item.markerN}`)
    }
    req({ r: 'Submit', text })
    editorApi.current?.setMarkdown('')
    setInput('')
    setDiagramChips([])
    setDesignChips([])
    setLocalPastes([])
    window.setTimeout(() => {
      submitLock.current = false
    }, 300)
    // Keep cancelled rows that are still waiting for a marker so the late
    // snapshot can drop the chip the user already removed.
    markerInsertQueue.current = markerInsertQueue.current.filter((row) => row.cancelled && row.markerN == null)
    // Swap the mascot to a new random cat on every send.
    setMascotSwap((t) => t + 1)
    // Force the transcript back to the bottom on send (re-engages the W4
    // scroll-stick even if the user had scrolled up).
    requestScrollBottom()
  }
  submitRef.current = submit

  // Snapshot assigned marker numbers. Bind them onto queue rows so Send is
  // not blocked and cancelled in-flight attaches can still be dropped.
  useEffect(() => {
    flushAttachmentInserts()
  }, [attachments, req])

  // Flush a send that happened while the paste marker was still in flight.
  // Wait until the chip state shows the marker so the body edit is what we save.
  useEffect(() => {
    const live = new Set(attachments.map((item) => `${item.kind}:${item.markerN}`))
    for (const key of [...consumedAttachKeys.current]) {
      if (!live.has(key)) consumedAttachKeys.current.delete(key)
    }
  }, [attachments])

  useEffect(() => {
    if (!submitArmed.current || submitLock.current) return
    if (markerInsertQueue.current.some((row) => !row.cancelled && row.markerN == null)) return
    const unmarked = localPastes.some((item) => {
      const row = markerInsertQueue.current.find((queued) => queued.id === item.id)
      return !!row && !row.cancelled && row.markerN != null && item.markerN == null
    })
    if (unmarked) return
    submitArmed.current = false
    submitRef.current()
  }, [localPastes, attachments])

  // If the daemon never assigns a marker, stop blocking send. Covers image and
  // paste rows (diagram attach used to leave image rows waiting forever).
  useEffect(() => {
    const waiting = markerInsertQueue.current.some((row) => !row.cancelled && row.markerN == null)
    if (!waiting) return
    const timer = window.setTimeout(() => {
      const stuck = markerInsertQueue.current.filter((row) => row.markerN == null && !row.cancelled)
      if (!stuck.length) return
      markerInsertQueue.current = markerInsertQueue.current.filter((row) => !stuck.some((item) => item.id === row.id))
      if (submitArmed.current && !submitLock.current) {
        submitArmed.current = false
        submitRef.current()
        return
      }
    }, 2500)
    return () => window.clearTimeout(timer)
  }, [localPastes, attachments, pendingComposerAttachmentInserts])

  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    // Follow-ups list focus: when the queue owns keys, Enter edits, arrows move,
    // Delete removes, Esc unfocuses (does not clear). Ctrl+X clears all below.
    if (steerFocus && pendingSteer.length > 0) {
      if (e.key === 'ArrowUp') {
        e.preventDefault()
        setSteerSel((s) => Math.max(0, s - 1))
        return
      }
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        setSteerSel((s) => {
          if (s + 1 < pendingSteer.length) return s + 1
          setSteerFocus(false)
          return s
        })
        return
      }
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault()
        editSteerAt(steerSel)
        return
      }
      if (e.key === 'Delete' || e.key === 'Backspace') {
        e.preventDefault()
        removeSteerAt(steerSel)
        return
      }
      if (e.key === 'Escape') {
        e.preventDefault()
        setSteerFocus(false)
        return
      }
      // Any printable char drops focus and falls through to the textarea.
      if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
        setSteerFocus(false)
      }
    }

    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey) && !e.shiftKey) {
      e.preventDefault()
      e.stopPropagation()
      submit()
      return
    }
    // Ctrl/Cmd+X clears every queued follow-up (TUI Ctrl+X parity).
    if ((e.ctrlKey || e.metaKey) && (e.key === 'x' || e.key === 'X') && pendingSteer.length > 0) {
      e.preventDefault()
      req({ r: 'CancelSteers' })
      setSteerFocus(false)
      return
    }
    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      // Omnisearch owns Up/Down for its own result-list navigation while open.
      if (omnisearchOpen) return
      const firstLine = editorApi.current?.isAtStart() ?? false
      const lastLine = editorApi.current?.isAtEnd() ?? false
      // From the first composer line, ↑ enters the follow-ups list when non-empty.
      if (e.key === 'ArrowUp' && firstLine && pendingSteer.length > 0) {
        e.preventDefault()
        setSteerFocus(true)
        setSteerSel(pendingSteer.length - 1)
        return
      }
      if (e.key === 'ArrowUp' && firstLine) {
        const history = recallCandidates()
        if (histIdxRef.current === -1) {
          if (history.length === 0) return
          stashRef.current = input
          stashChipsRef.current = diagramChipsRef.current
          stashDesignsRef.current = designChipsRef.current
          stashPastesRef.current = localPastesRef.current
          histIdxRef.current = history.length - 1
        } else if (histIdxRef.current > 0) {
          histIdxRef.current -= 1
        } else {
          return // already at the oldest entry — nothing further to recall
        }
        e.preventDefault()
        caretToEndRef.current = true
        showRecalled(history[histIdxRef.current].content)
      } else if (e.key === 'ArrowDown' && lastLine) {
        if (histIdxRef.current === -1) return // nothing recalled yet
        const history = recallCandidates()
        if (histIdxRef.current < history.length - 1) {
          histIdxRef.current += 1
          e.preventDefault()
          caretToEndRef.current = true
          showRecalled(history[histIdxRef.current].content)
        } else {
          // Walked past the newest recalled entry — restore the stashed draft.
          histIdxRef.current = -1
          e.preventDefault()
          caretToEndRef.current = true
          setDiagramChips(stashChipsRef.current)
          setDesignChips(stashDesignsRef.current)
          setLocalPastes(stashPastesRef.current)
          setInput(stashRef.current)
        }
      }
    }
  }

  // Draft change: clearing the composer to empty CANCELS a staged rewind (edit
  // pencil) — the user backed out, so the next send must NOT truncate. Only a
  // user edit fires onChange; programmatic refills (rewind/omnisearch/history
  // recall) go through setInput directly, so staging a rewind never
  // self-cancels here. A user edit also resets any in-progress history walk.
  const cancelMarkerQueueRows = (removed: Set<string>) => {
    for (const key of removed) {
      const colon = key.indexOf(':')
      if (colon < 0) continue
      const kind = key.slice(0, colon)
      if (kind !== 'image' && kind !== 'pasted_text') continue
      const markerN = Number(key.slice(colon + 1))
      if (!Number.isFinite(markerN)) continue
      for (const row of markerInsertQueue.current) {
        if (row.kind === kind && row.markerN === markerN) row.cancelled = true
      }
      if (kind === 'pasted_text') {
        dirtyPastes.current.delete(markerN)
        setLocalPastes((prev) => prev.filter((item) => item.markerN !== markerN))
      }
    }
  }

  const onDraft = (val: string) => {
    if (submitLock.current) return
    echoDraft.current = val !== input
    draftRef.current = val
    setInput(val)
    if (val.trim() === '' && pendingRewindIndex !== null) clearRewind()
    resetHistory()
  }

  const attachFiles = async (files: FileList | File[]) => {
    for (const file of Array.from(files)) {
      // Images only — the daemon's attachment ingest (Paste{path}) only
      // ingests image extensions; a non-image file falls through to
      // inserting its raw path into the shared composer buffer, silently
      // corrupting the session. Silently skip non-image files here; use
      // omnisearch to reference non-image workspace files by path instead.
      if (!file.type.startsWith('image/')) continue
      const id = mintDiagramChipId()
      markerInsertQueue.current.push({ id, kind: 'image', markerN: null, cancelled: false })
      try {
        const bytesB64 = await readFileAsBase64(file)
        req({ r: 'AttachFile', name: file.name, bytesB64, mime: file.type || undefined })
      } catch {
        markerInsertQueue.current = markerInsertQueue.current.filter((row) => row.id !== id)
      }
    }
  }

  const onPaste = (e: ClipboardEvent<HTMLElement>): boolean => {
    const items = Array.from(e.clipboardData?.items ?? [])
    const imageItem = items.find((item) => item.type.startsWith('image/'))
    if (imageItem) {
      const file = imageItem.getAsFile()
      if (file) {
        e.preventDefault()
        void attachFiles([file])
        return true
      }
    }
    const files = Array.from(e.clipboardData?.files ?? [])
    if (files.length > 0) {
      e.preventDefault()
      void attachFiles(files)
      return true
    }
    const raw = e.clipboardData?.getData('text/plain') ?? ''
    if (!raw) return false
    const text = raw.replace(/\r\n/g, '\n').replace(/\r/g, '\n')
    if (pasteByteLength(text) > PASTE_SOFT_MAX_BYTES) {
      e.preventDefault()
      toastError('Paste is larger than 2 MB')
      return true
    }
    if (looksLikeComposerMarkdown(text)) return false
    if (!shouldCollapsePaste(text)) return false
    e.preventDefault()
    const id = mintDiagramChipId()
    markerInsertQueue.current.push({ id, kind: 'pasted_text', markerN: null, cancelled: false })
    setLocalPastes((prev) => [...prev, { id, n: 0, path: '', text }])
    req({ r: 'AttachPaste', text })
    return true
  }

  const onDrop = (e: DragEvent<HTMLDivElement>) => {
    const types = Array.from(e.dataTransfer.types)
    if (hasComposerAttachmentDrag(types)) {
      e.preventDefault()
      setDragOver(false)
      return
    }
    e.preventDefault()
    setDragOver(false)
    // Coding tree path reference (not a file upload).
    const coding = readCodingPathDragData(e.dataTransfer)
    if (coding) {
      useKoma.getState().putCodingPathInChat(coding.root, coding.path, {
        isDir: coding.isDir,
      })
      return
    }
    if (e.dataTransfer.files.length > 0) void attachFiles(e.dataTransfer.files)
  }

  const onDragOver = (e: DragEvent<HTMLDivElement>) => {
    const types = Array.from(e.dataTransfer.types)
    if (hasComposerAttachmentDrag(types)) {
      e.preventDefault()
      try {
        e.dataTransfer.dropEffect = 'copy'
      } catch {
        /* ignore */
      }
      return
    }
    // Accept coding-tree path drags and external image files.
    e.preventDefault()
    setDragOver(true)
    try {
      e.dataTransfer.dropEffect = 'copy'
    } catch {
      /* ignore */
    }
  }

  const onDragLeave = () => setDragOver(false)

  const onFilePicked = (e: ChangeEvent<HTMLInputElement>) => {
    if (e.target.files && e.target.files.length > 0) void attachFiles(e.target.files)
    e.target.value = ''
  }

  const removeAttachment = (markerN: number, kind: 'image' | 'file' | 'pasted_text') => {
    if (kind === 'image' || kind === 'pasted_text') {
      cancelMarkerQueueRows(new Set([`${kind}:${markerN}`]))
      const chipKind = kind === 'image' ? 'image' : 'paste'
      editorApi.current?.removeAttachMarkerChip(chipKind, markerN)
      const marker = kind === 'image' ? imageMarker(markerN) : pasteMarker(markerN)
      setInput((prev) => (prev.includes(marker) ? prev.replace(marker, '') : prev))
    }
    req({ r: 'RemoveAttachment', markerN, kind })
  }

  const stripAttachments = useMemo(
    () => listedComposerAttachments(input, attachments, localPastes),
    [input, attachments, localPastes],
  )

  const removeListedAttachment = (item: (typeof stripAttachments)[number]) => {
    if (item.kind === 'pasted_text' && item.markerN == null && item.id) {
      const row = markerInsertQueue.current.find((queued) => queued.id === item.id)
      if (row) row.cancelled = true
      setLocalPastes((prev) => prev.filter((paste) => paste.id !== item.id))
      return
    }
    if (item.markerN != null) removeAttachment(item.markerN, item.kind)
  }

  const editPaste = (markerN: number, text: string) => {
    dirtyPastes.current.add(markerN)
    setPasteTexts((prev) => ({ ...prev, [markerN]: text }))
  }

  const openPasteEditor = (markerN: number) => {
    setPasteEditMarker(markerN)
    if (pasteTextsRef.current[markerN] == null) req({ r: 'ReadPaste', markerN })
  }

  const savePasteFromOverlay = (markerN: number, text: string) => {
    editPaste(markerN, text)
    req({ r: 'UpdatePaste', markerN, text })
  }

  const insertAttachmentIntoDraft = (kind: 'image' | 'pasted_text', markerN: number) => {
    const marker = kind === 'image' ? imageMarker(markerN) : pasteMarker(markerN)
    if (draftRef.current.includes(marker)) return
    editorApi.current?.insertChip(chipPayloadForAttachMarker(kind, markerN), { trailingSpace: true })
    editorApi.current?.focus()
  }

  const chipActions = useMemo(
    () => ({
      onPasteChipDoubleClick: (markerN: number) => openPasteEditor(markerN),
      onImageChipDoubleClick: (markerN: number) => {
        req({ r: 'ReadAttachment', markerN })
      },
      onFileChipDoubleClick: (wireText: string) => {
        const workdirs = (useKoma.getState().settingsValues?.workdir ?? []).filter(Boolean)
        const resolved = parseFileRefWire(wireText, workdirs)
        if (!resolved) return
        useKoma.getState().openCodingFile(resolved.root, resolved.path, { preview: false })
      },
      onRemoveChip: ({ nodeKey }) => {
        editorApi.current?.removeComposerChip(nodeKey)
      },
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [req],
  )

  const removeDiagramChip = (id: string) => {
    const next = diagramChips.filter((chip) => chip.id !== id)
    setDiagramChips(next)
    if (next.length === 0 && designChips.length === 0 && input.trim() === '' && pendingRewindIndex !== null) clearRewind()
  }

  const removeDesignChip = (id: string) => {
    const next = designChips.filter((chip) => chip.id !== id)
    setDesignChips(next)
    if (next.length === 0 && diagramChips.length === 0 && input.trim() === '' && pendingRewindIndex !== null) clearRewind()
  }

  const canSend =
    (input.trim() !== '' ||
      diagramChips.length > 0 ||
      designChips.length > 0 ||
      localPastes.length > 0 ||
      attachments.some((item) => item.kind === 'pasted_text' || item.kind === 'image')) &&
    !atSteerCap
  const [linkDraft, setLinkDraft] = useState<string | null>(null)

  const applyFormat = (kind: 'bold' | 'italic' | 'code' | 'strikethrough') => {
    editorApi.current?.format(kind)
  }

  const applyLink = () => {
    const href = linkDraft?.trim() ?? ''
    setLinkDraft(null)
    if (!href || !safeNoteUrl(href)) return
    editorApi.current?.insertLink(href)
    editorApi.current?.focus()
  }

  const formatButton =
    'flex h-7 w-7 flex-none items-center justify-center rounded-md text-koma-dim hover:bg-koma-hover hover:text-koma-fg'

  return (
    // claude.ai-style composer pinned at the bottom: a single rounded card
    // (textarea on top, an action bar below) that grows with its content. Drag
    // a file anywhere over the card to attach; the card rings on drag-over.
    <div className="px-2 pb-3 pt-1" data-tour="composer">
      {/* Follow-ups queue: submits made while the turn is cooking are queued
          daemon-side (cap 5). Selectable list — click or ↑ from composer. */}
      {pendingSteer.length > 0 && (
        <div
          className={`mb-1.5 flex flex-col gap-1 rounded-xl border bg-koma-panel px-2.5 py-2 ${
            steerFocus ? 'border-koma-accent' : 'border-koma-border'
          }`}
        >
          <div className="flex items-center gap-1.5 text-[11px] text-koma-dim">
            <Layers size={12} className="flex-none" />
            <span>
              follow-ups {pendingSteer.length}/5
            </span>
            <button
              onClick={() => {
                req({ r: 'CancelSteers' })
                setSteerFocus(false)
              }}
              aria-label="Clear all follow-ups"
              title="Clear all follow-ups"
              className="ml-auto flex-none opacity-60 transition-opacity hover:text-koma-fg hover:opacity-100"
            >
              <X size={12} />
            </button>
          </div>
          <div className="flex flex-col gap-0.5">
            {pendingSteer.map((s, i) => {
              const selected = steerFocus && i === steerSel
              const oneLine = steerPreview(s) || s.replace(/\s+/g, ' ').trim()
              return (
                <div
                  key={i}
                  role="button"
                  tabIndex={0}
                  onClick={() => {
                    setSteerFocus(true)
                    setSteerSel(i)
                    editSteerAt(i)
                  }}
                  onKeyDown={(ev) => {
                    if (ev.key === 'Enter' || ev.key === ' ') {
                      ev.preventDefault()
                      setSteerFocus(true)
                      setSteerSel(i)
                      editSteerAt(i)
                    }
                  }}
                  className={`flex cursor-pointer items-center gap-1.5 rounded-md px-1 py-0.5 text-[11.5px] ${
                    selected
                      ? 'bg-koma-hover text-koma-accent'
                      : 'text-koma-fg opacity-80 hover:bg-koma-hover/60'
                  }`}
                  title="Click to edit in composer"
                >
                  <span className="flex-none text-[10px]">{selected ? '●' : '○'}</span>
                  <span className="truncate">{oneLine}</span>
                  <button
                    type="button"
                    onClick={(ev) => {
                      ev.stopPropagation()
                      removeSteerAt(i)
                    }}
                    aria-label="Remove follow-up"
                    title="Remove"
                    className="ml-auto flex-none opacity-50 transition-opacity hover:opacity-100"
                  >
                    <X size={11} />
                  </button>
                </div>
              )
            })}
          </div>
          <div className="text-[10px] text-koma-dim">
            {steerFocus
              ? 'enter edit · ↑↓ select · del remove · esc unfocus · ctrl+x clear'
              : '↑ or click to select · ctrl+x clear all'}
          </div>
        </div>
      )}
      {pasteEditMarker != null && (
        <ComposerPasteEditOverlay
          markerN={pasteEditMarker}
          title={
            attachments.find((item) => item.kind === 'pasted_text' && item.markerN === pasteEditMarker)?.name ??
            `Pasted Text #${pasteEditMarker}`
          }
          initialText={pasteTexts[pasteEditMarker] ?? ''}
          loading={pasteTexts[pasteEditMarker] == null}
          onSave={savePasteFromOverlay}
          onClose={() => setPasteEditMarker(null)}
        />
      )}
      <div
        onDrop={onDrop}
        onDragOver={onDragOver}
        onDragLeave={onDragLeave}
        className={`relative flex flex-col gap-2 rounded-2xl border bg-koma-panel px-3 py-2.5 shadow-sm transition-colors @max-xs/chat:gap-1.5 @max-xs/chat:rounded-xl @max-xs/chat:px-2 @max-xs/chat:py-2 @max-[14rem]/chat:px-1.5 @max-[14rem]/chat:py-1.5 ${
          dragOver ? 'border-koma-accent bg-koma-hover' : 'border-koma-border'
        }`}
      >
        {/* Persistent mascot: a small always-looping cat perched on the card's
            top-right corner. Purely decorative — not gated on `working` — and
            swaps to a different random cat on every send (see submit above). */}
        <CatMascot swapTrigger={mascotSwap} />

        {/* Thinking bubble: floats ABOVE the cat (not beside it), only while
            `working`. The cat sits at -top-3/right-3 (h-12), so its top edge is
            12px above the card and its box is 48px tall; anchoring the bubble at
            -top-11 (-44px) puts its bottom edge ~8px above the cat's top edge (a
            small gap), while `right-3` matches the cat's right edge exactly.
            Only `right` is set (no `left`), so the pill still grows
            leftward/from-the-right as its word content needs. The nearest
            overflow-hidden ancestor is the shell's main content region
            (routes/index.tsx `top-8 flex overflow-hidden`), which spans nearly
            the full window height above the composer — the extra ~32px of
            poke-up room this needs (vs. the cat's 12px) stays well inside that
            box in any normal window size, so no portal is needed here (unlike
            e.g. Select/Combobox menus, which portal to <body> because they can
            open far down an overflow:auto list). Kept mounted (not conditionally
            rendered) so opacity/translate can transition instead of popping. */}
        <div
          className={`pointer-events-none absolute -top-11 right-3 z-10 transition-all duration-300 ${
            working ? 'translate-y-0 opacity-100' : 'translate-y-1 opacity-0'
          }`}
          aria-hidden="true"
        >
          <span className="whitespace-nowrap rounded-full border border-koma-border bg-koma-panel2 px-2.5 py-1 text-[11px] text-koma-dim shadow-sm">
            {thinkingWord.toLowerCase()}…
          </span>
        </div>

        {designChips.length > 0 && (
          <div className="flex flex-wrap gap-1">
            {designChips.map((chip) => (
              <span
                key={chip.id}
                className="flex items-center gap-1.5 rounded-lg border border-koma-border bg-koma-panel2 py-1 pl-2 pr-2 text-[11px] text-koma-fg"
              >
                <Frame size={14} className="flex-none text-koma-accent" />
                <span className="max-w-[140px] truncate">{chip.title}</span>
                <button
                  type="button"
                  onClick={() => removeDesignChip(chip.id)}
                  aria-label={`Remove ${chip.title}`}
                  className="flex-none opacity-60 transition-opacity hover:opacity-100"
                >
                  <X size={11} />
                </button>
              </span>
            ))}
          </div>
        )}

        {diagramChips.length > 0 && (
          <div className="flex flex-wrap gap-1">
            {diagramChips.map((chip) => (
              <span
                key={chip.id}
                className="flex items-center gap-1.5 rounded-lg border border-koma-border bg-koma-panel2 py-1 pl-1 pr-2 text-[11px] text-koma-fg"
              >
                <span className="h-8 w-12 flex-none overflow-hidden rounded border border-koma-border bg-koma-bg">
                  <DiagramSketch doc={chip.doc} />
                </span>
                <span className="max-w-[140px] truncate">{chip.title}</span>
                <button
                  type="button"
                  onClick={() => removeDiagramChip(chip.id)}
                  aria-label={`Remove ${chip.title}`}
                  className="flex-none opacity-60 transition-opacity hover:opacity-100"
                >
                  <X size={11} />
                </button>
              </span>
            ))}
          </div>
        )}

        {stripAttachments.length > 0 && (
          <div className="flex flex-col gap-1">
            <div className="flex flex-wrap gap-1">
              {stripAttachments.map((item) => {
                const canPlace = (item.kind === 'image' || item.kind === 'pasted_text') && item.markerN != null
                return (
                <span
                  key={item.key}
                  tabIndex={0}
                  role="group"
                  aria-label={item.name}
                  draggable={canPlace}
                  title={canPlace ? 'Drag into the message to place. Click to insert at the caret.' : undefined}
                  onDragStart={(event) => {
                    if (!canPlace || item.markerN == null) {
                      event.preventDefault()
                      return
                    }
                    const payload = { kind: item.kind as 'image' | 'pasted_text', markerN: item.markerN }
                    event.dataTransfer.setData(COMPOSER_ATTACHMENT_MIME, writeComposerAttachmentDrag(payload))
                    event.dataTransfer.setData('text/plain', composerAttachmentPlain(payload))
                    event.dataTransfer.effectAllowed = 'copy'
                  }}
                  onClick={(event) => {
                    if ((event.target as HTMLElement).closest('button')) return
                    if (canPlace && item.markerN != null) insertAttachmentIntoDraft(item.kind as 'image' | 'pasted_text', item.markerN)
                  }}
                  onKeyDown={(event) => {
                    if (event.key === 'Delete' || event.key === 'Backspace') {
                      event.preventDefault()
                      removeListedAttachment(item)
                      return
                    }
                    if ((event.key === 'Enter' || event.key === ' ') && canPlace && item.markerN != null) {
                      event.preventDefault()
                      insertAttachmentIntoDraft(item.kind as 'image' | 'pasted_text', item.markerN)
                    }
                  }}
                  className={`flex items-center gap-1 rounded-lg border border-koma-border bg-koma-panel2 px-2 py-1 text-[11px] text-koma-fg opacity-90 focus:outline-none focus-visible:ring-1 focus-visible:ring-koma-accent ${canPlace ? 'cursor-grab' : ''}`}
                >
                  {item.kind === 'pasted_text' && item.markerN != null ? (
                    <button
                      type="button"
                      className="max-w-[180px] truncate text-left"
                      onClick={() => openPasteEditor(item.markerN!)}
                    >
                      {item.name}
                    </button>
                  ) : (
                    <span className="max-w-[140px] truncate">{item.name}</span>
                  )}
                  <button
                    type="button"
                    onClick={() => removeListedAttachment(item)}
                    aria-label={`Remove ${item.name}`}
                    className="flex-none opacity-60 transition-opacity hover:opacity-100"
                  >
                    <X size={11} />
                  </button>
                </span>
                )
              })}
            </div>
          </div>
        )}
        <div className="flex flex-wrap items-center gap-0.5">
          <button type="button" className={formatButton} title="Bold" aria-label="Bold" onMouseDown={(event) => event.preventDefault()} onClick={() => applyFormat('bold')}>
            <Bold size={14} />
          </button>
          <button type="button" className={formatButton} title="Italic" aria-label="Italic" onMouseDown={(event) => event.preventDefault()} onClick={() => applyFormat('italic')}>
            <Italic size={14} />
          </button>
          <button type="button" className={formatButton} title="Strikethrough" aria-label="Strikethrough" onMouseDown={(event) => event.preventDefault()} onClick={() => applyFormat('strikethrough')}>
            <Strikethrough size={14} />
          </button>
          <button type="button" className={formatButton} title="Inline code" aria-label="Inline code" onMouseDown={(event) => event.preventDefault()} onClick={() => applyFormat('code')}>
            <Code size={14} />
          </button>
          <button
            type="button"
            className={formatButton}
            title="Link"
            aria-label="Link"
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => setLinkDraft((current) => (current == null ? 'https://' : null))}
          >
            <Link size={14} />
          </button>
          <button type="button" className={formatButton} title="Heading" aria-label="Heading" onMouseDown={(event) => event.preventDefault()} onClick={() => editorApi.current?.toggleHeading()}>
            <Heading2 size={14} />
          </button>
          <button type="button" className={formatButton} title="Bullet list" aria-label="Bullet list" onMouseDown={(event) => event.preventDefault()} onClick={() => editorApi.current?.toggleBullet()}>
            <List size={14} />
          </button>
          <button type="button" className={formatButton} title="Numbered list" aria-label="Numbered list" onMouseDown={(event) => event.preventDefault()} onClick={() => editorApi.current?.toggleNumber()}>
            <ListOrdered size={14} />
          </button>
          <button type="button" className={formatButton} title="Checklist" aria-label="Checklist" onMouseDown={(event) => event.preventDefault()} onClick={() => editorApi.current?.toggleCheckList()}>
            <ListChecks size={14} />
          </button>
          <button type="button" className={formatButton} title="Quote" aria-label="Quote" onMouseDown={(event) => event.preventDefault()} onClick={() => editorApi.current?.toggleQuote()}>
            <Quote size={14} />
          </button>
          <button type="button" className={formatButton} title="Code block" aria-label="Code block" onMouseDown={(event) => event.preventDefault()} onClick={() => editorApi.current?.toggleCodeBlock()}>
            <Braces size={14} />
          </button>
        </div>
        {linkDraft != null ? (
          <form
            className="flex gap-1 py-0.5"
            onSubmit={(event) => {
              event.preventDefault()
              applyLink()
            }}
          >
            <input
              autoFocus
              value={linkDraft}
              aria-label="Link address"
              placeholder="https://…"
              onChange={(event) => setLinkDraft(event.target.value)}
              className="h-7 min-w-0 flex-1 rounded-md border border-koma-border bg-koma-bg px-2 text-[11px] text-koma-fg outline-none focus:border-koma-accent"
            />
            <button type="submit" className="rounded-md bg-koma-accent/20 px-2 text-[11px] text-koma-fg hover:bg-koma-accent/30">
              Add
            </button>
            <button type="button" className="rounded-md px-2 text-[11px] text-koma-dim hover:bg-koma-hover hover:text-koma-fg" onClick={() => setLinkDraft(null)}>
              Cancel
            </button>
          </form>
        ) : null}

        <LexicalMarkdownEditor
          profile="composer"
          markdown={input}
          onMarkdown={onDraft}
          chipActions={chipActions}
          placeholder="Message koma…"
          ariaLabel="Message"
          apiRef={editorApi}
          onKeyDown={onKeyDown}
          onSubmit={submit}
          onPaste={onPaste}
          onPasteFiles={(files) => {
            void attachFiles(files)
          }}
          className={`relative z-0 max-h-[200px] min-h-[22px] overflow-y-auto text-koma-fg caret-koma-fg ${COMPOSER_FIELD_CLASS}`}
        />

        <div className="flex min-w-0 items-center justify-between gap-1">
          <div className="flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden @max-xs/chat:gap-0">
            <input
              ref={fileInputRef}
              type="file"
              accept="image/*"
              multiple
              className="hidden"
              onChange={onFilePicked}
            />
            <button
              onClick={() => fileInputRef.current?.click()}
              aria-label="Attach file"
              title="Attach file"
              className="flex h-8 w-8 flex-none items-center justify-center rounded-lg text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100 @max-[14rem]/chat:h-7 @max-[14rem]/chat:w-7"
            >
              <Paperclip size={16} />
            </button>
            <button
              onClick={openOmniSearch}
              aria-label="Search workspace files"
              title="Search workspace files"
              className="flex h-8 w-8 flex-none items-center justify-center rounded-lg text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100 @max-[14rem]/chat:h-7 @max-[14rem]/chat:w-7"
            >
              <Search size={16} />
            </button>
            {/* Session model quick-picker — compact, drops UP above the composer. */}
            <ModelPicker />
            {/* Reasoning effort (TUI /effort parity) — compact, drops UP above the composer. */}
            <EffortPicker />
            {/* Agent mode (Auto/Plan/Normal) — compact, drops UP above the composer. */}
            <ModeSelector />
          </div>

          <div className="flex flex-none items-center gap-1.5 @max-xs/chat:gap-1">
            {/* STOP is a SEPARATE control from send (not a morph): while the turn
                runs, both are shown — send stays LIVE so a submit QUEUES as a
                steer, and stop aborts the in-flight turn (GuiReq Interrupt). */}
            {working && (
              <button
                onClick={() => req({ r: 'Interrupt' })}
                aria-label="Stop"
                title="Stop"
                className="flex h-8 w-8 flex-none items-center justify-center rounded-full border border-koma-border text-koma-fg opacity-80 transition-colors hover:bg-koma-hover hover:opacity-100 @max-[14rem]/chat:h-7 @max-[14rem]/chat:w-7"
              >
                <Square size={13} className="fill-current" />
              </button>
            )}
            <button
              onClick={submit}
              disabled={!canSend}
              aria-label={working ? 'Queue message (Ctrl+Enter)' : 'Send (Ctrl+Enter)'}
              title={
                atSteerCap
                  ? '5 pending steers max'
                  : working
                    ? 'Queue while working (Ctrl+Enter)'
                    : 'Send (Ctrl+Enter)'
              }
              className={`flex h-8 w-8 flex-none items-center justify-center rounded-full transition-colors @max-[14rem]/chat:h-7 @max-[14rem]/chat:w-7 ${
                canSend
                  ? 'bg-koma-accent text-koma-bg hover:opacity-90'
                  : 'bg-koma-hover text-koma-fg opacity-40'
              }`}
            >
              <ArrowUp size={16} />
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
