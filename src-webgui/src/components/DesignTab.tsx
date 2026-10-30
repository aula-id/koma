import { canvasPoint } from '../lib/uiScale'
import { CODING_AUTOSAVE_MS } from '../lib/coding-autosave'
import { useEffect, useMemo, useRef, useState, useSyncExternalStore, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react'
import { ChevronRight, Frame, Hand, Minus, MousePointer2, PenTool, Plus, Type, PanelRightClose, PanelRightOpen, X } from 'lucide-react'
import { KomaSelect } from './KomaSelect'
import { TokenEditor } from './panels/DesignPanel'
import { DesignMenu, type DesignMenuItem } from './DesignMenu'
import { getDesignHover, getDesignUi, publishDesignUi, setDesignHover, subscribeDesignHover, type DesignLayerOp } from '../lib/designUi'
import { isTerminalInputTarget } from '../lib/terminalInput'
import {
  COMPONENT_MIME,
  DESIGN_MIME,
  alignDesignNodes,
  addComponentVariant,
  autoLayoutDesign,
  canLeaveParent,
  canvasDeltaToSpace,
  designAxisAligned,
  designChildIds,
  designDistances,
  designParentIds,
  designTurnSign,
  canvasToContent,
  cornerPixels,
  componentView,
  contentAngle,
  copyTree,
  createComponentFromFrame,
  createNode,
  designChatText,
  designChatTitle,
  applyDesignStyle,
  designCanvasBox,
  designRoots,
  designSelectionCanvasBox,
  designDrop,
  designLayerName,
  designObjectSnap,
  designResizeSnap,
  designSnapScene,
  designStyle,
  duplicateDesignNodes,
  flowBreakBar,
  frameDesignView,
  flowInsertIndex,
  inFlowBand,
  designPath,
  designQueryNode,
  deleteDesignNode,
  flipDesignNode,
  frameAtPoint,
  findDesignNode,
  fitVectorNode,
  insertDesignNode,
  layoutDesign,
  locateDesign,
  makeInstance,
  effectiveInstanceChild,
  mergeDesignOverride,
  pickVariant,
  moveDesignNodes,
  stepDesignSibling,
  nextVariantProps,
  nodeChrome,
  nodeFromPen,
  hitDesignDeepest,
  nodeBoxOrigin,
  nodeCanvasPoint,
  nodeCenter,
  nodeOrigin,
  nudgeDesignNodes,
  orderDesignNode,
  parseDesign,
  placeDesignNode,
  renameComponent,
  reorderDesignNode,
  resetInstanceOverrides,
  resizeDesignNode,
  resolveInstanceTree,
  resolveRef,
  selectAllDesign,
  canEnterDesignContainer,
  isBooleanNode,
  designEnterScopeForLayerSelect,
  exitDesignContainer,
  hitDesignInScope,
  resolveDesignSelectHit,
  validateDesignEnteredContainer,
  selectDesignHit,
  selectDesignRect,
  sharedValue,
  sameDesign,
  setDesignLocked,
  setDesignMode,
  setDesignVisible,
  setInstanceVariant,
  setVariantProps,
  snapDesign,
  stackDesign,
  textStyle,
  updateDesignNode,
  vectorSvgPath,
  writeComponentView,
  unwrapDesignNode,
  wrapDesignNodes,
  applyDesignToken,
  applyImageFill,
  applyDesignBoolean,
  createPolygonNode,
  createStarNode,
  rotationFromCenter,
  snapRotation,
  outlineStrokeNode,
  detachInstance,
  retargetTextRuns,
  firstVisiblePaint,
  nodePaints,
  panImageCrop,
  resizeImageCrop,
  setNodePaints,
  isImageCropPaint,
  openVectorEndpoints,
  paintGradientAngle,
  paintGradientCenter,
  parseDesignSlice,
  serializeDesignSlice,
  visibleDesignScreens,
  cacheDesignImage,
  createImageRect,
  hashBytes,
  hydrateDesignImages,
  imageNaturalSize,
  rememberImageSize,
  mimeForName,
  putDesignImage,
  writeDesignAsset,
  designAssetPath,
  type DesignAlignAxis,
  type DesignAlignEdge,
  type DesignDoc,
  type DesignFlowBar,
  type DesignGuide,
  type DesignHandle,
  type DesignMeasure,
  type DesignOrder,
  type DesignRect,
  type DesignStyle,
  type DesignPenPoint,
  type DesignQuery,
  type DesignNode,
  type DesignVector,
  type DesignPaint,
  type DesignWeight,
} from '../lib/design'
import type { FramePreset } from '../lib/design/presets'
import { DesignCursorGhost, type CursorGhostShape } from './design/DesignCursorGhost'
import {
  appendVector,
  bendSegment,
  closestOnSegment,
  deleteSegments,
  deleteVertices,
  duplicateVertices,
  extractVertices,
  fixAngle,
  flipVertices,
  handleModeOf,
  joinVertices,
  mergeOnDrop,
  mergeVertices,
  moveHandle,
  moveVertices,
  nodeType,
  pointOnSegment,
  pullSmoothPair,
  segmentSvgPath,
  separateVertices,
  setNodeType,
  snapToPoints,
  splitSegment,
  verticesInRect,
  type HandleMode,
} from '../lib/design/vectorEdit'
import { DesignPathBar, type PathBarAction } from './design/DesignPathBar'
import { installDesignTextMeasurer } from './design/textMeasure'

installDesignTextMeasurer()
import { designPngBase64 } from '../lib/designRender'
import { fileKey } from '../store/coding'
import { isTabVisible } from '../store/editorGroups'
import { useKoma } from '../store/koma'
import type { Tab } from '../store/types/tabs'
import { showCodingHistory } from './CodingHistory'
import { EditorChrome } from './EditorChrome'

import {
  drawnBox,
  getCopiedShape,
  getCopiedStyle,
  setCopiedShape,
  setCopiedStyle,
  editingDoc,
  paintOnlyDoc,
  deepSelectKey,
  instanceAxes,
  lineBox,
  lineEnds,
  mintId,
  projectDoc,
  shapeCorners,
  UNDO_CAP,
  clearDesignHistory,
  designHistory,
  type DesignHistoryEntry,
  ZOOM_KEY_STEP,
  ZOOM_MAX,
  ZOOM_PER_PIXEL,
  ZOOM_SETTLE_MS,
  ZOOM_MIN,
  type DesignCommands,
  DESIGN_PATCH_DEBOUNCE_MS,
  HANDLES,
  SELECTION,
  SHAPE_FILL,
  type Drag,
  type DrawShape,
  type Ghost,
  type PenDraft,
  type RadiusCorner,
  type Tool,
  type View,
} from './design/tabShared'
import { canvasBackdrop, DesignRulers, designMenuItems, ToolButton } from './design/DesignRulers'
import { DesignNodeView, stableNodeHandlers, type DesignNodeHandlers } from './design/DesignNodeView'
import { DesignSelectionChrome } from './design/DesignSelectionChrome'
import { FrameToolFlyout, ShapeToolFlyout } from './design/DesignTools'
import { NodeSettings } from './design/DesignNodeSettings'
import { PenOverlay } from './design/DesignPropertyFields'

/** Nodes copied in path edit mode. Shared across designer tabs, like the shape clipboard. */
let vectorClipboard: DesignVector | null = null

/** The cursor miniature for a drawing tool, sized like the shape a click would place. */
function cursorGhostFor(
  tool: Tool,
  preset: FramePreset | null,
): { shape: CursorGhostShape; width: number; height: number; label: string } | null {
  const sized = (shape: CursorGhostShape, name: string, w: number, h: number) => ({
    shape,
    width: w,
    height: h,
    label: shape === 'line' ? `${name} ${w}` : `${name} ${w}×${h}`,
  })
  if (tool === 'frame' || tool === 'section') {
    if (preset) return sized('frame', preset.name, preset.width, preset.height)
    const frame = createNode('frame', 'ghost', 0, 0)
    return sized('frame', tool === 'section' ? 'Section' : 'Frame', frame.w, frame.h)
  }
  if (tool === 'rect' || tool === 'ellipse' || tool === 'line') {
    const node = createNode(tool, 'ghost', 0, 0)
    return sized(tool, node.name ?? tool, node.w, node.h)
  }
  if (tool === 'triangle') return sized('triangle', 'Triangle', 100, 100)
  if (tool === 'polygon') return sized('polygon', 'Polygon', 100, 100)
  if (tool === 'star') return sized('star', 'Star', 100, 100)
  if (tool === 'pen') return { shape: 'pen', width: 16, height: 16, label: 'Click to add points, drag for curves' }
  if (tool === 'text') return { shape: 'text', width: 16, height: 16, label: 'Click to type, drag for a text box' }
  return null
}

export function DesignTab({ tab }: { tab: Extract<Tab, { kind: 'design' }> }) {
  const key = fileKey(tab.root, tab.path)
  const file = useKoma((s) => s.design?.docs?.[key])
  const updateDesign = useKoma((s) => s.updateDesign)
  const saveDesign = useKoma((s) => s.saveDesign)
  const setDesignFileUi = useKoma((s) => s.setDesignFileUi)
  const setDesignPanelTab = useKoma((s) => s.setDesignPanelTab)
  const panelTabId = useKoma((s) => s.design?.panelTabId)
  const active = useKoma((s) => s.ui.activeTabId === tab.id && isTabVisible(s.ui, tab.id))
  const canvasRef = useRef<HTMLDivElement>(null)
  const nodeHandlersRef = useRef<DesignNodeHandlers | null>(null)
  const nodeHandlers = useMemo(() => stableNodeHandlers(nodeHandlersRef), [])
  const paintDocRef = useRef<DesignDoc | null>(null)
  /** Pointer held by the canvas while a drag runs, so a release outside the window still ends it. */
  const captureRef = useRef<number | null>(null)
  const historyRef = useRef(designHistory(key))
  const pastRef = useRef(historyRef.current.past)
  const futureRef = useRef(historyRef.current.future)
  const nudgeOpenRef = useRef(false)
  const nudgeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const dragRef = useRef<Drag | null>(null)
  const cropEditRef = useRef(false)
  const labelNoted = useRef(false)
  const patchDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const pendingTypePatchRef = useRef<((node: DesignNode) => DesignNode) | null>(null)
  const spaceRef = useRef(false)
  const toolRef = useRef<Tool>('select')
  const viewRef = useRef<View>({ panX: 40, panY: 40, zoom: 1 })
  const updateRef = useRef(updateDesign)
  updateRef.current = updateDesign
  const [view, setView] = useState<View>(viewRef.current)
  /** Zoom the node layer is laid out at. A gesture scales that layout with a transform and settles here when it pauses. */
  const [layerZoom, setLayerZoom] = useState(viewRef.current.zoom)
  const settleRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const hoverId = useSyncExternalStore(subscribeDesignHover, getDesignHover)
  const hoverFrameRef = useRef<{ frame: number; clientX: number; clientY: number; deep: boolean }>({ frame: 0, clientX: 0, clientY: 0, deep: false })
  const [altDown, setAltDown] = useState(false)
  const [tool, setTool] = useState<Tool>('select')
  /** Preset picked from the frame menu: the next click places a frame of this size. */
  const [framePreset, setFramePreset] = useState<FramePreset | null>(null)
  const framePresetRef = useRef(framePreset)
  framePresetRef.current = framePreset
  useEffect(() => {
    if (tool !== 'frame') setFramePreset(null)
  }, [tool])
  const [selection, setSelection] = useState<string[]>([])
  const [editing, setEditing] = useState<string | null>(null)
  const [propsOpen, setPropsOpen] = useState(true)
  const [openFillToken, setOpenFillToken] = useState(0)
  const [inspectorPaint, setInspectorPaint] = useState<DesignPaint | null>(null)
  const [ghost, setGhost] = useState<Ghost | null>(null)
  const [flowBar, setFlowBar] = useState<DesignFlowBar | null>(null)
  const [snapMarks, setSnapMarks] = useState<{ guides: DesignGuide[]; measures: DesignMeasure[] } | null>(null)
  const [pen, setPen] = useState<PenDraft | null>(null)
  const [penHover, setPenHover] = useState<{ x: number; y: number } | null>(null)
  const [penHandle, setPenHandle] = useState<number | null>(null)
  const [menu, setMenu] = useState<{ x: number; y: number; canvasX: number; canvasY: number } | null>(null)
  const commandsRef = useRef<DesignCommands | null>(null)
  const pasteTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const stepOutRef = useRef<() => void>(() => {})
  const layerOpsRef = useRef<(action: DesignLayerOp) => void>(() => {})
  const penApplyRef = useRef<(draft: PenDraft, closed?: boolean) => void>(() => {})
  const finishPenRef = useRef<(closed: boolean) => void>(() => {})
  const cancelPenRef = useRef<() => void>(() => {})
  const chooseToolRef = useRef<(next: Tool) => void>(() => {})
  const penRef = useRef<PenDraft | null>(null)
  const penNoted = useRef(false)
  /** Last pen click, so a second click on the same spot soon after ends the path. */
  const penClickRef = useRef<{ time: number; x: number; y: number } | null>(null)
  const penTargetRef = useRef<(raw: { x: number; y: number }, shift: boolean) => { x: number; y: number }>((raw) => raw)
  const penBackRef = useRef<() => void>(() => {})
  const selectionRef = useRef<string[]>([])
  selectionRef.current = selection
  penRef.current = pen
  const [spaceDown, setSpaceDown] = useState(false)
  const [dragCursor, setDragCursor] = useState<string | null>(null)
  const [rev, setRev] = useState(0)
  const [focusId, setFocusId] = useState<string | null>(null)
  const [overrideTargetId, setOverrideTargetId] = useState<string | null>(null)
  const [enteredContainerId, setEnteredContainerId] = useState<string | null>(null)
  const [vectorEditId, setVectorEditId] = useState<string | null>(null)
  /** Selected nodes of the vector being edited, and the node-edit options. */
  const [vectorSel, setVectorSelState] = useState<number[]>([])
  const vectorSelRef = useRef<number[]>([])
  const [vectorSnap, setVectorSnap] = useState(true)
  /** Figma's bend tool: while on, dragging a segment curves it and clicking a node toggles its curve. */
  const [vectorBend, setVectorBend] = useState(false)
  const vectorBendRef = useRef(false)
  vectorBendRef.current = vectorBend
  const vectorSnapRef = useRef(true)
  vectorSnapRef.current = vectorSnap
  const [vectorMarquee, setVectorMarquee] = useState<{ x: number; y: number; w: number; h: number } | null>(null)
  useEffect(() => {
    vectorSelRef.current = []
    setVectorSelState([])
    setVectorBend(false)
  }, [vectorEditId])
  const [textRange, setTextRange] = useState<{ id: string; start: number; end: number } | null>(null)
  const [palette, setPalette] = useState(false)
  const vectorEditRef = useRef<string | null>(null)
  const imageInputRef = useRef<HTMLInputElement | null>(null)
  const focusRef = useRef<string | null>(null)
  const overrideTargetRef = useRef<string | null>(null)
  const enteredContainerRef = useRef<string | null>(null)
  toolRef.current = tool
  focusRef.current = focusId
  overrideTargetRef.current = overrideTargetId
  enteredContainerRef.current = enteredContainerId

  useEffect(() => {
    // History outlives the tab. A document that was reloaded underneath it starts fresh.
    const history = designHistory(key)
    const current = useKoma.getState().design?.docs?.[key]?.doc ?? null
    if (history.head && current !== history.head) clearDesignHistory(history)
    historyRef.current = history
    pastRef.current = history.past
    futureRef.current = history.future
    setSelection([])
    setEnteredContainerId(null)
    setEditing(null)
    setFocusId(null)
    setOverrideTargetId(null)
    setPen(null)
    setPenHover(null)
    setPenHandle(null)
    setMenu(null)
    if (patchDebounceRef.current) {
      clearTimeout(patchDebounceRef.current)
      patchDebounceRef.current = null
    }
    pendingTypePatchRef.current = null
    const next = useKoma.getState().design?.fileUi?.[key]?.view ?? { panX: 40, panY: 40, zoom: 1 }
    viewRef.current = next
    setView(next)
    setLayerZoom(next.zoom)
    setRev((value) => value + 1)
    return () => setDesignFileUi(tab.root, tab.path, { view: viewRef.current })
  }, [key])

  useEffect(() => {
    historyRef.current.head = file?.doc ?? null
  }, [file?.doc])

  // Autosave follows the coding autosave setting, and waits for a drag to end.
  const autosave = useKoma((s) => !!s.settingsValues?.codingAutosave)
  useEffect(() => {
    if (!autosave || !file?.dirty || file.saving || file.loading || file.error) return
    let timer: ReturnType<typeof setTimeout>
    const attempt = () => {
      if (dragRef.current) {
        timer = setTimeout(attempt, CODING_AUTOSAVE_MS)
        return
      }
      const current = useKoma.getState().design?.docs?.[key]
      if (current?.dirty && !current.saving && !current.loading && !current.error) saveDesign(tab.root, tab.path)
    }
    timer = setTimeout(attempt, CODING_AUTOSAVE_MS)
    return () => clearTimeout(timer)
  }, [autosave, file?.dirty, file?.doc, file?.error, file?.loading, file?.saving, key, saveDesign, tab.path, tab.root])

  useEffect(() => {
    const onFocus = (event: Event) => {
      const detail = (event as CustomEvent<{ root: string; path: string; componentId: string | null }>).detail
      if (!detail || detail.root !== tab.root || detail.path !== tab.path) return
      setFocusId(detail.componentId)
      setSelection([])
      setEditing(null)
      setPen(null)
      setPenHover(null)
      setPenHandle(null)
    }
    window.addEventListener('koma-design-focus', onFocus)
    return () => window.removeEventListener('koma-design-focus', onFocus)
  }, [tab.path, tab.root])

  useEffect(() => {
    if (active) setDesignPanelTab(tab.id)
  }, [active, setDesignPanelTab, tab.id])

  const imageSig = file?.doc.images ? Object.keys(file.doc.images).sort().join('|') : ''
  useEffect(() => {
    const doc = useKoma.getState().design?.docs?.[key]?.doc
    if (!doc?.images) return
    let cancelled = false
    void hydrateDesignImages(doc, useKoma.getState().req, tab.root).then((changed) => {
      if (!cancelled && changed) setRev((value) => value + 1)
    })
    return () => {
      cancelled = true
    }
  }, [imageSig, key, tab.root])

  useEffect(() => {
    setDesignFileUi(tab.root, tab.path, { selection, focusId, overrideTargetId })
  }, [focusId, overrideTargetId, selection, setDesignFileUi, tab.path, tab.root])

  useEffect(() => {
    if (panelTabId !== tab.id) return
    publishDesignUi({ root: tab.root, path: tab.path, selection, focusId, overrideTargetId, enteredContainerId })
  }, [enteredContainerId, focusId, overrideTargetId, panelTabId, selection, tab.id, tab.path, tab.root])

  useEffect(() => {
    return () => {
      if (nudgeTimerRef.current) clearTimeout(nudgeTimerRef.current)
      const current = getDesignUi()
      if (current && current.root === tab.root && current.path === tab.path) publishDesignUi(null)
    }
  }, [tab.path, tab.root])

  useEffect(() => {
    const onLayer = (event: Event) => {
      const detail = (event as CustomEvent<{ root: string; path: string; action: DesignLayerOp }>).detail
      if (!detail || detail.root !== tab.root || detail.path !== tab.path) return
      layerOpsRef.current(detail.action)
    }
    window.addEventListener('koma-design-layer', onLayer)
    return () => window.removeEventListener('koma-design-layer', onLayer)
  }, [tab.path, tab.root])

  useEffect(() => {
    if (tool === 'pen') return
    penRef.current = null
    penNoted.current = false
    setPen(null)
    setPenHover(null)
    setPenHandle(null)
  }, [tool])

  useEffect(() => {
    if (!focusId || !file) return
    if (!file.doc.components.some((component) => component.id === focusId)) setFocusId(null)
  }, [file, focusId])

  useEffect(() => {
    if (!overrideTargetId) return
    if (selection.length !== 1) setOverrideTargetId(null)
  }, [overrideTargetId, selection])

  useEffect(() => {
    if (!file?.doc) return
    setEnteredContainerId((current) => validateDesignEnteredContainer(file.doc, current))
  }, [file?.doc])

  const closeNudge = () => {
    if (nudgeTimerRef.current) {
      clearTimeout(nudgeTimerRef.current)
      nudgeTimerRef.current = null
    }
    nudgeOpenRef.current = false
  }

  const note = (doc: DesignDoc) => {
    closeNudge()
    pastRef.current.push({ doc, selection: selectionRef.current })
    if (pastRef.current.length > UNDO_CAP) pastRef.current.shift()
    futureRef.current.length = 0
    setRev((value) => value + 1)
  }
  const noteRef = useRef(note)
  noteRef.current = note

  const commit = (view: DesignDoc) => {
    const stored = useKoma.getState().design?.docs?.[key]?.doc
    if (!stored) return
    const next = projectDoc(stored, focusRef.current, view)
    if (sameDesign(stored, next)) return
    note(stored)
    updateDesign(tab.root, tab.path, next)
  }
  const importImages = async (files: File[], point?: { x: number; y: number }) => {
    const stored = useKoma.getState().design?.docs?.[key]?.doc
    if (!stored || !files.length) return
    const req = useKoma.getState().req
    let next = stored
    const ids: string[] = []
    let cursor = point ?? { x: 40, y: 40 }
    for (const file of files) {
      const mime = file.type || mimeForName(file.name)
      if (!mime?.startsWith('image/')) continue
      const bytes = new Uint8Array(await file.arrayBuffer())
      const hash = await hashBytes(bytes)
      const path = designAssetPath(hash, mime)
      cacheDesignImage(hash, bytes, mime)
      writeDesignAsset(req, tab.root, path, bytes)
      const size = await imageNaturalSize(bytes, mime)
      rememberImageSize(hash, size.w, size.h)
      next = putDesignImage(next, hash, mime, path, size)
      const selected = selectionRef.current[0]
      const target = selected ? findDesignNode(next, selected) : null
      if (target && (target.kind === 'rect' || target.kind === 'frame' || target.kind === 'ellipse')) {
        next = updateDesignNode(next, target.id, (node) => applyImageFill(node, hash, 'fill'))
        ids.push(target.id)
      } else {
        const node = createImageRect(mintId('img'), cursor.x, cursor.y, size.w, size.h, hash)
        next = insertDesignNode(next, null, node)
        ids.push(node.id)
        cursor = { x: cursor.x + 24, y: cursor.y + 24 }
      }
    }
    if (ids.length) {
      commitStored(next)
      setSelection(ids)
      setTool('select')
    }
  }

  const importImagesRef = useRef(importImages)
  importImagesRef.current = importImages

  // Images pasted from the system clipboard, such as a screenshot, land in the middle of the view.
  useEffect(() => {
    if (!active) return
    const onPaste = (event: ClipboardEvent) => {
      const target = event.target as HTMLElement | null
      if (isTerminalInputTarget(event.target) || target?.closest?.('input, textarea, [contenteditable="true"], [contenteditable="plaintext-only"]')) return
      const files = [...(event.clipboardData?.files ?? [])].filter((item) => (item.type || mimeForName(item.name))?.startsWith('image/'))
      if (!files.length) return
      event.preventDefault()
      if (pasteTimerRef.current) clearTimeout(pasteTimerRef.current)
      pasteTimerRef.current = null
      const view = viewportBox()
      void importImagesRef.current(files, view ? { x: view.x + view.w / 2, y: view.y + view.h / 2 } : undefined)
    }
    window.addEventListener('paste', onPaste)
    return () => window.removeEventListener('paste', onPaste)
  }, [active])

  const commitStored = (next: DesignDoc, noteOnce = false) => {
    const stored = useKoma.getState().design?.docs?.[key]?.doc
    if (!stored) return
    const laid = layoutDesign(next)
    if (sameDesign(stored, laid)) return
    if (noteOnce) {
      if (!labelNoted.current) {
        note(stored)
        labelNoted.current = true
      }
    } else note(stored)
    updateDesign(tab.root, tab.path, laid)
  }
  const commitRef = useRef(commit)
  commitRef.current = commit

  /** Swap the document for a history entry and bring back the selection it had. */
  const travel = (from: DesignHistoryEntry[], to: DesignHistoryEntry[]) => {
    closeNudge()
    const entry = from.pop()
    if (!entry) return
    const current = useKoma.getState().design?.docs?.[key]?.doc
    if (current) to.push({ doc: current, selection: selectionRef.current })
    setRev((value) => value + 1)
    setEditing(null)
    updateDesign(tab.root, tab.path, entry.doc)
    const view = editingDoc(entry.doc, focusRef.current)
    const selection = entry.selection.filter((id) => locateDesign(view, id))
    selectionRef.current = selection
    setSelection(selection)
  }
  const undo = () => travel(pastRef.current, futureRef.current)
  const redo = () => travel(futureRef.current, pastRef.current)
  const undoRef = useRef(undo)
  const redoRef = useRef(redo)
  undoRef.current = undo
  redoRef.current = redo

  const revert = () => {
    const saved = useKoma.getState().design?.docs?.[key]?.savedText
    if (!saved) return
    const parsed = parseDesign(saved)
    if (parsed.error) return
    clearDesignHistory(historyRef.current)
    setRev((value) => value + 1)
    setEditing(null)
    updateDesign(tab.root, tab.path, parsed.doc)
  }

  /**
   * A gesture step only moves the cheap transform. CSS zoom re-lays out every node,
   * so the layer catches up once the gesture pauses.
   */
  const applyView = (next: View, gesture = false) => {
    viewRef.current = next
    setView(next)
    if (settleRef.current) clearTimeout(settleRef.current)
    settleRef.current = null
    if (!gesture) {
      setLayerZoom(next.zoom)
      return
    }
    settleRef.current = setTimeout(() => {
      settleRef.current = null
      setLayerZoom(viewRef.current.zoom)
    }, ZOOM_SETTLE_MS)
  }

  const toDoc = (clientX: number, clientY: number) => {
    const rect = canvasRef.current?.getBoundingClientRect()
    if (!rect) return null
    const current = viewRef.current
    return {
      x: (canvasPoint(clientX - rect.left) - current.panX) / current.zoom,
      y: (canvasPoint(clientY - rect.top) - current.panY) / current.zoom,
    }
  }

  /** The visible canvas in document coordinates, so snapping ignores what is off screen. */
  const viewportBox = (): DesignRect | undefined => {
    const rect = canvasRef.current?.getBoundingClientRect()
    if (!rect) return undefined
    const current = viewRef.current
    return { x: -current.panX / current.zoom, y: -current.panY / current.zoom, w: canvasPoint(rect.width) / current.zoom, h: canvasPoint(rect.height) / current.zoom }
  }

  const zoomTo = (ids: string[] | null) => {
    const stored = useKoma.getState().design?.docs?.[key]?.doc
    const canvas = canvasRef.current
    if (!stored || !canvas) return
    const open = editingDoc(stored, focusRef.current)
    const targets = ids?.length ? ids : open.screens.map((screen) => screen.id)
    const boxes = targets.flatMap((id) => {
      const box = designCanvasBox(open, id)
      return box ? [box] : []
    })
    const framed = frameDesignView(boxes, canvas.clientWidth, canvas.clientHeight, ZOOM_MIN, ZOOM_MAX)
    if (framed) applyView(framed)
  }

  const zoomAt = (clientX: number, clientY: number, nextZoom: number, gesture = false) => {
    const rect = canvasRef.current?.getBoundingClientRect()
    if (!rect) return
    const current = viewRef.current
    const zoom = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, nextZoom))
    const cx = canvasPoint(clientX - rect.left)
    const cy = canvasPoint(clientY - rect.top)
    const docX = (cx - current.panX) / current.zoom
    const docY = (cy - current.panY) / current.zoom
    applyView({ zoom, panX: cx - docX * zoom, panY: cy - docY * zoom }, gesture)
  }

  useEffect(() => {
    if (!dragCursor) return
    const previous = document.body.style.cursor
    document.body.style.cursor = dragCursor
    return () => {
      document.body.style.cursor = previous
    }
  }, [dragCursor])

  useEffect(() => {
    const onRestore = (event: Event) => {
      const detail = (event as CustomEvent<{ root: string; path: string }>).detail
      if (!detail || detail.root !== tab.root || detail.path !== tab.path) return
      clearDesignHistory(historyRef.current)
      setEditing(null)
      setRev((value) => value + 1)
    }
    const commitDrawn = (doc: DesignDoc, drag: Extract<Drag, { kind: 'draw' }>) => {
      const stored = useKoma.getState().design?.docs?.[key]?.doc
      if (!stored) return
      const tiny = Math.hypot(drag.x1 - drag.x0, drag.y1 - drag.y0) < 4
      const id = mintId(drag.shape[0])
      let node = createNode(drag.shape, id, drag.x0, drag.y0)
      if (!tiny && drag.shape === 'line') {
        const line = lineBox(drag.x0, drag.y0, drag.x1, drag.y1)
        node = { ...node, x: line.x, y: line.y, w: line.w, h: line.h, rotation: contentAngle(doc, drag.parentId, line.rotation) }
      } else if (!tiny) {
        const box = drawnBox(drag.x0, drag.y0, drag.x1, drag.y1, doc.grid, doc.snap)
        node = { ...node, x: box.x, y: box.y, w: Math.max(1, box.w), h: Math.max(1, box.h) }
      } else {
        node = { ...node, x: snapDesign(drag.x0, doc.grid, doc.snap), y: snapDesign(drag.y0, doc.grid, doc.snap) }
        const preset = toolRef.current === 'frame' ? framePresetRef.current : null
        if (preset) node = { ...node, name: preset.name, w: preset.width, h: preset.height }
      }
      const local = canvasToContent(doc, drag.parentId, node.x, node.y)
      if (!local) return
      node = { ...node, x: snapDesign(local.x, doc.grid, doc.snap), y: snapDesign(local.y, doc.grid, doc.snap) }
      if (toolRef.current === 'section') node = { ...node, section: true, name: 'Section' }
      // A click types on one growing line; a dragged box keeps its width and wraps.
      if (drag.shape === 'text') node = { ...node, text: '', textHug: tiny ? 'width' : 'height' }
      let next = doc
      // A shape drawn into an auto-layout frame stays where it was drawn instead of
      // joining the flow and jumping to the frame's start.
      if (drag.parentId && findDesignNode(doc, drag.parentId)?.layout) node = { ...node, absolute: true }
      if (drag.parentId) next = insertDesignNode(doc, drag.parentId, node)
      else if (focusRef.current && node.kind !== 'frame' && node.kind !== 'group') {
        const screen = createNode('frame', mintId('f'), node.x, node.y)
        node.x = 16
        node.y = 16
        screen.children = [node]
        next = insertDesignNode(doc, null, screen)
      } else next = insertDesignNode(doc, null, node)
      const laid = projectDoc(stored, focusRef.current, next)
      if (sameDesign(laid, stored)) return
      noteRef.current(stored)
      updateRef.current(tab.root, tab.path, laid)
      setSelection([node.id])
      setPropsOpen(true)
      setTool('select')
      if (drag.shape === 'text') {
        // Typing folds into the creation's undo step.
        labelNoted.current = true
        setEditing(node.id)
      }
    }
    const move = (event: PointerEvent) => {
      const drag = dragRef.current
      if (toolRef.current === 'pen' && drag?.kind !== 'pen') {
        const hover = toDoc(event.clientX, event.clientY)
        if (hover) setPenHover(penTargetRef.current(hover, event.shiftKey))
      }
      if (!drag) return
      setDesignHover(null)
      if (captureRef.current == null && (drag.kind !== 'move' || drag.moved)) {
        try {
          canvasRef.current?.setPointerCapture(event.pointerId)
          captureRef.current = event.pointerId
        } catch {
          // The pointer is already gone; pointerup or blur ends the drag.
        }
      }
      if (drag.kind === 'pan') {
        const dx = canvasPoint(event.clientX - drag.lastX)
        const dy = canvasPoint(event.clientY - drag.lastY)
        drag.lastX = event.clientX
        drag.lastY = event.clientY
        const current = viewRef.current
        applyView({ ...current, panX: current.panX + dx, panY: current.panY + dy })
        return
      }
      const point = toDoc(event.clientX, event.clientY)
      if (drag.kind === 'marquee' && point) {
        drag.x1 = point.x
        drag.y1 = point.y
        const box = drawnBox(drag.x0, drag.y0, drag.x1, drag.y1, 1, false)
        setGhost({ kind: 'marquee', ...box })
        return
      }
      if (drag.kind === 'draw' && point) {
        if (drag.shape === 'line') {
          const ends = lineEnds(drag.cx, drag.cy, point.x, point.y, event.shiftKey, event.altKey)
          drag.x0 = ends.x0
          drag.y0 = ends.y0
          drag.x1 = ends.x1
          drag.y1 = ends.y1
          const line = lineBox(drag.x0, drag.y0, drag.x1, drag.y1)
          setGhost({ kind: 'shape', shape: 'line', ...line })
        } else {
          const corners = shapeCorners(drag.cx, drag.cy, point.x, point.y, event.shiftKey, event.altKey)
          drag.x0 = corners.x0
          drag.y0 = corners.y0
          drag.x1 = corners.x1
          drag.y1 = corners.y1
          const stored = useKoma.getState().design?.docs?.[key]?.doc
          const doc = stored ? editingDoc(stored, focusRef.current) : null
          const box = drawnBox(drag.x0, drag.y0, drag.x1, drag.y1, doc?.grid ?? 8, !!doc?.snap)
          setGhost({ kind: 'shape', shape: drag.shape, ...box, rotation: 0 })
        }
        return
      }
      if (drag.kind === 'pen' && point) {
        const draft = penRef.current
        if (!draft) return
        const points = draft.points.slice()
        const current = points[drag.index]
        if (!current) return
        if (drag.space || spaceRef.current) {
          current.x = point.x
          current.y = point.y
        } else {
          // Shift keeps the handle on 45° steps; Alt breaks the pair so only the outgoing side moves.
          const tip = event.shiftKey ? fixAngle(current, point) : point
          current.outgoing = { x: tip.x - current.x, y: tip.y - current.y }
          if (!event.altKey) current.incoming = { x: current.x - tip.x, y: current.y - tip.y }
        }
        const next = { ...draft, points }
        penRef.current = next
        setPen(next)
        penApplyRef.current(next)
        return
      }
      if ((drag.kind === 'vnodes' || drag.kind === 'vcurve' || drag.kind === 'vseg' || drag.kind === 'vmarquee') && point) {
        vectorDragRef.current.move(drag, point, event)
        return
      }
      if (drag.kind === 'rotate') {
        const stored = useKoma.getState().design?.docs?.[key]?.doc
        if (!stored) return
        const focus = focusRef.current
        const doc = editingDoc(stored, focus)
        const located = locateDesign(doc, drag.id)
        if (!located || !point) return
        const current = rotationFromCenter(drag.cx, drag.cy, point.x, point.y)
        const rotation = snapRotation(drag.rotation + drag.sign * (current - drag.startAngle), event.shiftKey)
        const nextNode = { ...drag.node }
        if (rotation) nextNode.rotation = rotation
        else delete nextNode.rotation
        const next = projectDoc(stored, focus, updateDesignNode(doc, drag.id, () => nextNode))
        if (sameDesign(next, stored)) return
        if (!drag.remembered) {
          noteRef.current(stored)
          drag.remembered = true
        }
        updateRef.current(tab.root, tab.path, next)
        return
      }
      if (drag.kind === 'handle' && point) {
        const stored = useKoma.getState().design?.docs?.[key]?.doc
        if (!stored) return
        const focus = focusRef.current
        const doc = editingDoc(stored, focus)
        const origin = nodeOrigin(doc, drag.id)
        if (!origin) return
        // Plain keeps the node's own type; ⌘ mirrors, Alt frees, ⌘+Alt aligns (Penpot). Shift snaps to 45°.
        const meta = event.metaKey || event.ctrlKey
        const mode: HandleMode = meta && event.altKey ? 'aligned' : meta ? 'mirror' : event.altKey ? 'independent' : drag.mode
        const next = projectDoc(stored, focus, updateDesignNode(doc, drag.id, (node) => {
          const vector = node.vector
          const segment = vector?.segments[drag.segment]
          const vertex = segment && vector?.vertices[drag.end === 'start' ? segment.start : segment.end]
          if (!vector || !vertex) return node
          const raw = { x: point.x - origin.x, y: point.y - origin.y }
          const tip = event.shiftKey ? fixAngle(vertex, raw) : raw
          return fitVectorNode({ ...node, vector: moveHandle(vector, drag.segment, drag.end, tip, mode) })
        }))
        if (sameDesign(next, stored)) return
        if (!drag.remembered) {
          noteRef.current(stored)
          drag.remembered = true
        }
        updateRef.current(tab.root, tab.path, next)
        return
      }
      if (drag.kind === 'crop') {
        const stored = useKoma.getState().design?.docs?.[key]?.doc
        if (!stored) return
        const focus = focusRef.current
        const doc = editingDoc(stored, focus)
        const zoom = viewRef.current.zoom || 1
        const screenDx = canvasPoint(event.clientX - drag.startX) / zoom
        const screenDy = canvasPoint(event.clientY - drag.startY) / zoom
        const located = locateDesign(doc, drag.id)
        if (!located) return
        const delta = canvasDeltaToSpace(doc, located.parentId, screenDx, screenDy)
        const nextPaint = panImageCrop(drag.paint, delta.x, delta.y)
        const next = projectDoc(stored, focus, updateDesignNode(doc, drag.id, (node) => {
          const paints = nodePaints(node, 'fill')
          return setNodePaints(node, 'fill', paints.map((paint) => (isImageCropPaint(paint) && paint.hash === drag.paint.hash ? nextPaint : paint)))
        }))
        if (sameDesign(next, stored)) return
        if (!drag.remembered) {
          noteRef.current(stored)
          drag.remembered = true
        }
        updateRef.current(tab.root, tab.path, next)
        return
      }
      if (drag.kind !== 'move' && drag.kind !== 'resize' && drag.kind !== 'radius') return
      const stored = useKoma.getState().design?.docs?.[key]?.doc
      if (!stored) return
      const focus = focusRef.current
      const doc = editingDoc(stored, focus)
      const zoom = viewRef.current.zoom || 1
      const screenDx = canvasPoint(event.clientX - drag.startX) / zoom
      const screenDy = canvasPoint(event.clientY - drag.startY) / zoom
      if (drag.kind === 'radius') {
        const located = locateDesign(doc, drag.id)
        if (!located) return
        const delta = canvasDeltaToSpace(doc, drag.id, screenDx, screenDy)
        const inward = drag.corner === 'tl' ? delta.x + delta.y : drag.corner === 'tr' ? -delta.x + delta.y : drag.corner === 'bl' ? delta.x - delta.y : -delta.x - delta.y
        const limit = Math.min(drag.node.w, drag.node.h) / 2
        const radius = Math.round(Math.min(limit, Math.max(0, drag.radius + inward / 2)))
        const nextNode = { ...drag.node }
        const cornerKey = drag.corner === 'tl' ? 'radiusTL' : drag.corner === 'tr' ? 'radiusTR' : drag.corner === 'bl' ? 'radiusBL' : 'radiusBR'
        const uniform = typeof drag.node.radius === 'number' ? drag.node.radius : 0
        if (radius === uniform) delete nextNode[cornerKey]
        else nextNode[cornerKey] = radius
        const next = projectDoc(stored, focus, updateDesignNode(doc, drag.id, () => nextNode))
        if (sameDesign(next, stored)) return
        if (!drag.remembered) {
          noteRef.current(stored)
          drag.remembered = true
        }
        updateRef.current(tab.root, tab.path, next)
        return
      }
      if (drag.kind === 'resize') {
        const located = locateDesign(doc, drag.id)
        if (!located) return
        const delta = canvasDeltaToSpace(doc, drag.id, screenDx, screenDy)
        if (cropEditRef.current) {
          const paint = nodePaints(drag.node, 'fill').find(isImageCropPaint)
          if (paint) {
            const nextPaint = resizeImageCrop(paint, drag.handle, delta.x, delta.y, event.shiftKey)
            const next = projectDoc(stored, focus, updateDesignNode(doc, drag.id, (node) => (
              setNodePaints(node, 'fill', nodePaints(node, 'fill').map((item) => (isImageCropPaint(item) && item.hash === paint.hash ? nextPaint : item)))
            )))
            if (sameDesign(next, stored)) return
            if (!drag.remembered) {
              noteRef.current(stored)
              drag.remembered = true
            }
            updateRef.current(tab.root, tab.path, next)
            return
          }
        }
        const center = event.altKey
        let dx = delta.x
        let dy = delta.y
        let snapX = false
        let snapY = false
        setSnapMarks(null)
        // Object snap moves the pointer first, so a Shift lock still holds the ratio afterwards.
        // A rotated or flipped box has no canvas-aligned edges, so it skips object snap.
        // Cmd or Ctrl held during the drag turns object snapping off.
        const currentBox = !(event.metaKey || event.ctrlKey) && designAxisAligned(doc, drag.id) ? designCanvasBox(doc, drag.id) : null
        const scene = currentBox ? designSnapScene(doc, [drag.id], viewportBox()) : null
        if (currentBox && scene) {
          const free = resizeDesignNode(drag.node, drag.handle, dx, dy, doc.grid, doc.snap, false, { center })
          if (!free.flipX && !free.flipY) {
            const proposed = { x: currentBox.x + (free.x - located.node.x), y: currentBox.y + (free.y - located.node.y), w: free.w, h: free.h }
            const snap = designResizeSnap(proposed, scene.targets, drag.handle, 5 / zoom)
            if (snap.snappedX) {
              dx += drag.handle.includes('w') ? snap.box.x - proposed.x : snap.box.x + snap.box.w - (proposed.x + proposed.w)
              snapX = true
            }
            if (snap.snappedY) {
              dy += drag.handle.includes('n') ? snap.box.y - proposed.y : snap.box.y + snap.box.h - (proposed.y + proposed.h)
              snapY = true
            }
            setSnapMarks(snap.guides.length || snap.measures.length ? { guides: snap.guides, measures: snap.measures } : null)
          }
        }
        const resized = resizeDesignNode(drag.node, drag.handle, dx, dy, doc.grid, doc.snap, event.shiftKey, { center, gridX: !snapX, gridY: !snapY })
        const next = projectDoc(stored, focus, updateDesignNode(doc, drag.id, () => resized))
        if (sameDesign(next, stored)) return
        if (!drag.remembered) {
          noteRef.current(stored)
          drag.remembered = true
        }
        updateRef.current(tab.root, tab.path, next)
        return
      }
      if (!drag.moved && Math.hypot(canvasPoint(event.clientX - drag.startX), canvasPoint(event.clientY - drag.startY)) < 4) return
      drag.moved = true
      let live = doc
      if (drag.alt) {
        const duplicated = duplicateDesignNodes(live, drag.ids, 0, 0, () => mintId('n'))
        if (duplicated) {
          drag.alt = false
          drag.ids = duplicated.ids
          const origins: Record<string, { x: number; y: number }> = {}
          const laid = layoutDesign(duplicated.doc)
          for (const id of duplicated.ids) {
            const row = locateDesign(laid, id)
            if (row) origins[id] = { x: row.node.x, y: row.node.y }
          }
          drag.origins = origins
          const copied = projectDoc(stored, focus, laid)
          if (!drag.remembered) {
            noteRef.current(stored)
            drag.remembered = true
          }
          updateRef.current(tab.root, tab.path, copied)
          selectionRef.current = duplicated.ids
          setSelection(duplicated.ids)
          live = laid
        }
      }
      const flowId = drag.ids.length === 1 ? drag.ids[0] : null
      const flowLocated = flowId ? locateDesign(live, flowId) : null
      const flowParent = flowLocated?.parentId ? findDesignNode(live, flowLocated.parentId) : null
      const flowing = !!(flowId && flowLocated && flowParent?.layout && !flowLocated.node.absolute)
      if (flowing && flowId && flowParent && point && !drag.broke && inFlowBand(live, flowParent.id, point.x, point.y)) {
        setSnapMarks(null)
        setFlowBar(flowBreakBar(live, flowParent.id, flowId, point.x, point.y))
        return
      }
      if (flowing && point && !drag.broke) drag.broke = true
      setFlowBar(null)
      const moveIds = designRoots(live, drag.ids)
      if (!drag.scene) drag.scene = designSnapScene(live, drag.ids, viewportBox())
      const snap = drag.scene && !(event.metaKey || event.ctrlKey)
        ? designObjectSnap(drag.scene.moving, drag.scene.targets, screenDx, screenDy, 5 / zoom)
        : { dx: screenDx, dy: screenDy, snappedX: false, snappedY: false, guides: [], measures: [] }
      setSnapMarks(snap.guides.length || snap.measures.length ? { guides: snap.guides, measures: snap.measures } : null)
      let view = live
      let frozen: string | undefined
      for (const id of moveIds) {
        const located = locateDesign(view, id)
        const origin = drag.origins[id]
        if (!located || !origin) continue
        const parent = located.parentId ? findDesignNode(view, located.parentId) : null
        if (parent?.layout && !located.node.absolute) frozen = id
        const delta = canvasDeltaToSpace(view, located.parentId, snap.dx, snap.dy)
        const x = snap.snappedX ? origin.x + delta.x : snapDesign(origin.x + delta.x, doc.grid, doc.snap)
        const y = snap.snappedY ? origin.y + delta.y : snapDesign(origin.y + delta.y, doc.grid, doc.snap)
        view = updateDesignNode(view, id, (node) => ({ ...node, x, y }))
      }
      const next = projectDoc(stored, focus, view, frozen)
      if (sameDesign(next, stored)) return
      if (!drag.remembered) {
        noteRef.current(stored)
        drag.remembered = true
      }
      updateRef.current(tab.root, tab.path, next)
    }
    const release = () => {
      const drag = dragRef.current
      dragRef.current = null
      if (captureRef.current != null) {
        try {
          canvasRef.current?.releasePointerCapture(captureRef.current)
        } catch {
          // Capture already ended with the pointer.
        }
        captureRef.current = null
      }
      setDragCursor(null)
      setGhost(null)
      setFlowBar(null)
      setSnapMarks(null)
      if (drag?.kind === 'pen') setPenHandle(null)
      return drag
    }
    /** Losing the window ends a drag where it is and lets go of Space, since its keyup never arrives. */
    const abandon = () => {
      if (document.visibilityState === 'hidden' || !document.hasFocus()) {
        release()
        spaceRef.current = false
        setSpaceDown(false)
        setAltDown(false)
        setDesignHover(null)
      }
    }
    const up = (event: PointerEvent) => {
      const drag = release()
      if (!drag || drag.kind === 'pan' || drag.kind === 'pen') return
      if (drag.kind === 'vnodes' || drag.kind === 'vcurve' || drag.kind === 'vseg' || drag.kind === 'vmarquee') {
        vectorDragRef.current.up(drag)
        return
      }
      const stored = useKoma.getState().design?.docs?.[key]?.doc
      if (!stored) return
      const focus = focusRef.current
      const doc = editingDoc(stored, focus)
      if (drag.kind === 'marquee') {
        const box = drawnBox(drag.x0, drag.y0, drag.x1, drag.y1, 1, false)
        if (box.w >= 2 || box.h >= 2) setSelection(selectDesignRect(doc, box.x, box.y, box.w, box.h, enteredContainerRef.current))
        return
      }
      if (drag.kind === 'draw') {
        commitDrawn(doc, drag)
        return
      }
      if (drag.kind !== 'move') {
        if (drag.kind === 'resize' && drag.remembered) return
        return
      }
      if (!drag.moved) return
      const point = toDoc(event.clientX, event.clientY)
      const dragId = drag.ids[0]
      if (!dragId || !point) return
      const located = locateDesign(doc, dragId)
      let next = doc
      if (located && drag.ids.length === 1 && !(focus && located.parentId == null)) {
        const drop = designDrop(doc, dragId, point.x, point.y)
        const parent = located.parentId ? findDesignNode(doc, located.parentId) : null
        const localPoint = parent ? canvasToContent(doc, parent.id, point.x, point.y) : null
        if (drop.kind === 'stay' && parent?.layout && !located.node.absolute && localPoint && inFlowBand(doc, parent.id, point.x, point.y)) {
          const along = parent.layout === 'row' ? localPoint.x : localPoint.y
          next = reorderDesignNode(doc, dragId, flowInsertIndex(parent, dragId, along))
        } else if (drop.kind === 'move' && !(focus && drop.parentId == null)) {
          next = placeDesignNode(doc, dragId, drop.parentId, snapDesign(drop.x, doc.grid, doc.snap), snapDesign(drop.y, doc.grid, doc.snap))
        }
      }
      const laid = projectDoc(stored, focus, next)
      if (sameDesign(laid, stored)) return
      if (!drag.remembered) noteRef.current(stored)
      updateRef.current(tab.root, tab.path, laid)
    }
    const onCommit = (event: Event) => {
      const detail = (event as CustomEvent<{ root: string; path: string; doc: DesignDoc }>).detail
      if (!detail || detail.root !== tab.root || detail.path !== tab.path) return
      event.preventDefault()
      const stored = useKoma.getState().design?.docs?.[key]?.doc
      if (!stored || sameDesign(stored, detail.doc)) return
      noteRef.current(stored)
      updateRef.current(tab.root, tab.path, detail.doc)
    }
    const onApplyToken = (event: Event) => {
      const detail = (event as CustomEvent<{ root: string; path: string; name: string }>).detail
      if (!detail || detail.root !== tab.root || detail.path !== tab.path) return
      const stored = useKoma.getState().design?.docs?.[key]?.doc
      if (!stored || !selectionRef.current.length) return
      const next = applyDesignToken(stored, selectionRef.current, detail.name)
      if (sameDesign(next, stored)) return
      noteRef.current(stored)
      updateRef.current(tab.root, tab.path, next)
    }
    window.addEventListener('koma-design-commit', onCommit)
    window.addEventListener('koma-design-apply-token', onApplyToken)
    window.addEventListener('koma-design-restore', onRestore)
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
    window.addEventListener('pointercancel', up)
    window.addEventListener('blur', abandon)
    document.addEventListener('visibilitychange', abandon)
    return () => {
      window.removeEventListener('blur', abandon)
      document.removeEventListener('visibilitychange', abandon)
      window.removeEventListener('koma-design-commit', onCommit)
      window.removeEventListener('koma-design-apply-token', onApplyToken)
      window.removeEventListener('koma-design-restore', onRestore)
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      window.removeEventListener('pointercancel', up)
    }
  }, [key, tab.path, tab.root])

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    // Wheel events arrive faster than frames. Deltas pile up and apply once per frame.
    const pending = { panX: 0, panY: 0, zoom: 1, clientX: 0, clientY: 0, frame: 0 }
    const flush = () => {
      pending.frame = 0
      if (pending.zoom !== 1) zoomAt(pending.clientX, pending.clientY, viewRef.current.zoom * pending.zoom, true)
      if (pending.panX || pending.panY) {
        const current = viewRef.current
        applyView({ ...current, panX: current.panX - pending.panX, panY: current.panY - pending.panY }, pending.zoom !== 1)
      }
      pending.panX = 0
      pending.panY = 0
      pending.zoom = 1
    }
    const onWheel = (event: WheelEvent) => {
      event.preventDefault()
      const unit = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 16 : event.deltaMode === WheelEvent.DOM_DELTA_PAGE ? canvas.clientHeight : 1
      let dx = event.deltaX * unit
      let dy = event.deltaY * unit
      if (event.ctrlKey || event.metaKey) {
        // A trackpad pinch sends many small deltas and a mouse notch one large one; both scale smoothly.
        pending.zoom *= Math.exp(-dy * ZOOM_PER_PIXEL)
        pending.clientX = event.clientX
        pending.clientY = event.clientY
      } else {
        if (event.shiftKey && !dx) {
          dx = dy
          dy = 0
        }
        pending.panX += dx
        pending.panY += dy
      }
      if (!pending.frame) pending.frame = requestAnimationFrame(flush)
    }
    canvas.addEventListener('wheel', onWheel, { passive: false })
    return () => {
      canvas.removeEventListener('wheel', onWheel)
      if (pending.frame) cancelAnimationFrame(pending.frame)
    }
  }, [file?.loading])

  useEffect(() => {
    if (!active) return
    const onKey = (event: KeyboardEvent) => {
      if (isTerminalInputTarget(event.target)) return
      const typing = !!(event.target as HTMLElement | null)?.closest('input, textarea, [contenteditable="true"], [contenteditable="plaintext-only"]')
      if ((event.metaKey || event.ctrlKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 's') {
        event.preventDefault()
        saveDesign(tab.root, tab.path)
        return
      }
      if (event.key === 'Alt') setAltDown(event.type === 'keydown' && !typing)
      if (event.code === 'Space') {
        if (typing) return
        event.preventDefault()
        spaceRef.current = event.type === 'keydown'
        setSpaceDown(event.type === 'keydown')
        return
      }
      if (event.type !== 'keydown') return
      if (typing) return
      const meta = event.metaKey || event.ctrlKey
      if ((event.metaKey || event.ctrlKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'k') {
        event.preventDefault()
        setPalette((open) => !open)
        return
      }
      if (meta && !event.altKey && event.key.toLowerCase() === 'z') {
        event.preventDefault()
        if (event.shiftKey) redoRef.current()
        else undoRef.current()
        return
      }
      if (meta && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'y') {
        event.preventDefault()
        redoRef.current()
        return
      }
      if (vectorKeyRef.current(event)) return
      if (meta && event.altKey && !event.shiftKey && event.key.toLowerCase() === 'c') {
        event.preventDefault()
        commandsRef.current?.copyStyle()
        return
      }
      if (meta && event.altKey && !event.shiftKey && event.key.toLowerCase() === 'v') {
        event.preventDefault()
        commandsRef.current?.pasteStyle()
        return
      }
      if (meta && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'c') {
        event.preventDefault()
        commandsRef.current?.copy()
        return
      }
      if (meta && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'v') {
        // The browser's paste event comes next and may carry an image. The shape paste waits one turn for it.
        if (pasteTimerRef.current) clearTimeout(pasteTimerRef.current)
        pasteTimerRef.current = setTimeout(() => {
          pasteTimerRef.current = null
          commandsRef.current?.paste()
        }, 0)
        return
      }
      if (meta && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'd') {
        event.preventDefault()
        commandsRef.current?.duplicate()
        return
      }
      if (meta && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'a') {
        event.preventDefault()
        commandsRef.current?.selectAll()
        return
      }
      if (meta && !event.altKey && event.key.toLowerCase() === 'g') {
        event.preventDefault()
        if (event.shiftKey) commandsRef.current?.unwrap()
        else commandsRef.current?.wrap('group')
        return
      }
      if (meta && event.altKey && !event.shiftKey && event.key.toLowerCase() === 'g') {
        event.preventDefault()
        commandsRef.current?.wrap('frame')
        return
      }
      if (meta && event.shiftKey && event.key.toLowerCase() === 'h') {
        event.preventDefault()
        commandsRef.current?.hide()
        return
      }
      if (meta && event.shiftKey && event.key.toLowerCase() === 'l') {
        event.preventDefault()
        commandsRef.current?.lock()
        return
      }
      if (meta && event.altKey && event.key.toLowerCase() === 'b') {
        event.preventDefault()
        commandsRef.current?.detach()
        return
      }
      if (meta && event.altKey && event.key.toLowerCase() === 'k') {
        event.preventDefault()
        commandsRef.current?.component()
        return
      }
      if (!meta && event.altKey && event.shiftKey) {
        const booleanOp = event.code === 'KeyU' ? 'union' : event.code === 'KeyS' ? 'subtract' : event.code === 'KeyI' ? 'intersect' : event.code === 'KeyE' ? 'exclude' : event.code === 'KeyF' ? 'flatten' : null
        if (booleanOp) {
          event.preventDefault()
          commandsRef.current?.boolean(booleanOp)
          return
        }
      }
      if (meta && event.altKey && event.key.toLowerCase() === 'o') {
        event.preventDefault()
        commandsRef.current?.outline()
        return
      }
      if (!meta && event.shiftKey && event.key.toLowerCase() === 'a') {
        event.preventDefault()
        commandsRef.current?.auto()
        return
      }
      if (event.key === ']' || event.key === '[') {
        event.preventDefault()
        const order: DesignOrder = event.key === ']' ? (meta ? 'front' : 'forward') : meta ? 'back' : 'backward'
        commandsRef.current?.order(order)
        return
      }
      if (!meta && event.shiftKey && event.key.toLowerCase() === 'h') {
        event.preventDefault()
        commandsRef.current?.flip('x')
        return
      }
      if (!meta && event.shiftKey && event.key.toLowerCase() === 'v') {
        event.preventDefault()
        commandsRef.current?.flip('y')
        return
      }
      if (event.key === 'Delete' || event.key === 'Backspace') {
        event.preventDefault()
        if (toolRef.current === 'pen' && penRef.current) penBackRef.current()
        else commandsRef.current?.remove()
        return
      }
      if (!meta && !event.altKey && event.shiftKey && (event.code === 'Digit1' || event.code === 'Digit2')) {
        event.preventDefault()
        commandsRef.current?.zoom(event.code === 'Digit1' ? 'all' : 'selection')
        return
      }
      if (!meta && (event.key === 'ArrowLeft' || event.key === 'ArrowRight' || event.key === 'ArrowUp' || event.key === 'ArrowDown')) {
        event.preventDefault()
        const step = event.shiftKey ? 10 : 1
        const dx = event.key === 'ArrowLeft' ? -step : event.key === 'ArrowRight' ? step : 0
        const dy = event.key === 'ArrowUp' ? -step : event.key === 'ArrowDown' ? step : 0
        commandsRef.current?.nudge(dx, dy, event.altKey && !event.repeat)
        return
      }
      if (!meta && !event.shiftKey && !event.altKey) {
        const tools: Partial<Record<string, Tool>> = { v: 'select', r: 'rect', o: 'ellipse', l: 'line', f: 'frame', s: 'section', t: 'text', p: 'pen', h: 'pan' }
        const next = tools[event.key.toLowerCase()]
        if (next) {
          event.preventDefault()
          chooseToolRef.current(next)
          return
        }
      }
      if (event.key === 'Enter' && !meta) {
        event.preventDefault()
        if (toolRef.current === 'pen' && penRef.current) finishPenRef.current(false)
        else if (vectorEditRef.current) setVectorEditId(null)
        else if (event.shiftKey) commandsRef.current?.selectParent()
        else commandsRef.current?.selectChildren()
        return
      }
      if (event.key === 'Tab' && !meta && !event.altKey && selectionRef.current.length) {
        event.preventDefault()
        commandsRef.current?.step(event.shiftKey ? -1 : 1)
        return
      }
      if (!meta && !event.altKey && (event.key === '=' || event.key === '+' || event.key === '-')) {
        event.preventDefault()
        commandsRef.current?.zoomStep(event.key === '-' ? 1 / ZOOM_KEY_STEP : ZOOM_KEY_STEP)
        return
      }
      if (!meta && !event.altKey && event.shiftKey && event.code === 'Digit0') {
        event.preventDefault()
        commandsRef.current?.zoomStep(0)
        return
      }
      if (event.key === 'Escape') {
        setEditing(null)
        if (toolRef.current !== 'select' && toolRef.current !== 'pen' && toolRef.current !== 'pan') {
          setTool('select')
          return
        }
        if (toolRef.current === 'pen' && penRef.current) {
          // Escape keeps what was drawn (Penpot); a lone first point is dropped.
          if (penRef.current.points.length >= 2) finishPenRef.current(false)
          else cancelPenRef.current()
          return
        }
        if (vectorEditRef.current) {
          setVectorEditId(null)
          return
        }
        setPen(null)
        setPenHover(null)
        setPenHandle(null)
        penRef.current = null
        penNoted.current = false
        stepOutRef.current()
      }
    }
    window.addEventListener('keydown', onKey)
    window.addEventListener('keyup', onKey)
    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('keyup', onKey)
    }
  }, [active, file?.loading, key, saveDesign, selection, tab.path, tab.root])

  const placeAt = (kind: DrawShape, point: { x: number; y: number }) => {
    const stored = useKoma.getState().design?.docs?.[key]?.doc
    if (!stored) return
    const doc = editingDoc(stored, focusRef.current)
    const parent = kind === 'frame' || focusRef.current ? frameAtPoint(doc, point.x, point.y, '') : null
    const node = createNode(kind, mintId(kind[0]), 0, 0)
    let next = doc
    if (parent) {
      const local = canvasToContent(doc, parent, point.x, point.y)
      if (!local) return
      node.x = snapDesign(local.x, doc.grid, doc.snap)
      node.y = snapDesign(local.y, doc.grid, doc.snap)
      next = insertDesignNode(doc, parent, node)
    } else if (focusRef.current && kind !== 'frame') {
      const screen = createNode('frame', mintId('f'), snapDesign(point.x, doc.grid, doc.snap), snapDesign(point.y, doc.grid, doc.snap))
      node.x = 16
      node.y = 16
      screen.children = [node]
      next = insertDesignNode(doc, null, screen)
    } else {
      node.x = snapDesign(point.x, doc.grid, doc.snap)
      node.y = snapDesign(point.y, doc.grid, doc.snap)
      next = insertDesignNode(doc, null, node)
    }
    commit(next)
    setSelection([node.id])
    setEditing(null)
    setPropsOpen(true)
    setTool('select')
  }

  const applyPen = (draft: PenDraft, closed = false) => {
    const stored = useKoma.getState().design?.docs?.[key]?.doc
    if (!stored) return
    const focus = focusRef.current
    const doc = editingDoc(stored, focus)
    const points = draft.points.map((point) => {
      const at = canvasToContent(doc, draft.parentId, point.x, point.y)
      return {
        x: at?.x ?? point.x,
        y: at?.y ?? point.y,
        incoming: canvasDeltaToSpace(doc, draft.parentId, point.incoming.x, point.incoming.y),
        outgoing: canvasDeltaToSpace(doc, draft.parentId, point.outgoing.x, point.outgoing.y),
      }
    })
    const node = nodeFromPen(draft.id, points, closed)
    if (!node) return
    const existing = locateDesign(doc, draft.id)
    const view = existing
      ? updateDesignNode(doc, draft.id, (current) => ({
          ...current,
          x: node.x,
          y: node.y,
          w: node.w,
          h: node.h,
          vector: node.vector,
          fill: closed ? (current.fill && current.fill !== 'none' ? current.fill : '#d9d9d9') : 'none',
          stroke: current.stroke && current.stroke !== 'none' ? current.stroke : '#1c1c1c',
          strokeWidth: current.strokeWidth ?? 2,
        }))
      : insertDesignNode(doc, draft.parentId, node)
    const next = projectDoc(stored, focus, view)
    if (sameDesign(next, stored)) return
    if (!penNoted.current) {
      note(stored)
      penNoted.current = true
    }
    updateDesign(tab.root, tab.path, next)
    setSelection([draft.id])
  }
  penApplyRef.current = applyPen

  const finishPen = (closed: boolean) => {
    const draft = penRef.current
    if (!draft || draft.points.length < 2) {
      cancelPen()
      return
    }
    applyPen(draft, closed)
    penRef.current = null
    setPen(null)
    setPenHover(null)
    setPenHandle(null)
    penNoted.current = false
    setTool('select')
    setSelection([draft.id])
  }

  /**
   * Where a pen point lands: Shift locks it to 45° from the last point; otherwise it snaps
   * to the path's own points and, with snapping on, to the grid.
   */
  const penTarget = (raw: { x: number; y: number }, shift: boolean) => {
    const draft = penRef.current
    const last = draft?.points[draft.points.length - 1]
    if (shift && last) return fixAngle(last, raw)
    const others = draft ? draft.points.slice(0, -1) : []
    const near = snapToPoints(raw, others, 6 / viewRef.current.zoom)
    const doc = viewDoc()?.doc
    if (!doc?.snap) return near
    return {
      x: near.x === raw.x ? snapDesign(raw.x, doc.grid, true) : near.x,
      y: near.y === raw.y ? snapDesign(raw.y, doc.grid, true) : near.y,
    }
  }
  penTargetRef.current = penTarget

  /** Backspace while drawing takes back the last point; the last one left cancels the path. */
  const penBack = () => {
    const draft = penRef.current
    if (!draft) return
    if (draft.points.length <= 1) {
      cancelPen()
      return
    }
    const next = { ...draft, points: draft.points.slice(0, -1) }
    penRef.current = next
    setPen(next)
    setPenHandle(null)
    applyPen(next)
  }
  penBackRef.current = penBack

  const cancelPen = () => {
    const draft = penRef.current
    penRef.current = null
    setPen(null)
    setPenHover(null)
    setPenHandle(null)
    if (draft?.id && penNoted.current) {
      const stored = useKoma.getState().design?.docs?.[key]?.doc
      if (stored && locateDesign(stored, draft.id)) commitStored(deleteDesignNode(stored, draft.id))
    }
    penNoted.current = false
    setTool('select')
  }

  const chooseTool = (next: Tool) => {
    if (toolRef.current === 'pen' && next !== 'pen' && next !== 'pan' && penRef.current) finishPen(false)
    setTool(next)
  }
  finishPenRef.current = finishPen

  // --- Path editing (Penpot-style node editor) --------------------------------------------
  const setVectorSel = (ids: number[]) => {
    vectorSelRef.current = ids
    setVectorSelState(ids)
  }
  /** The vector being edited, in the editing view, with its canvas origin. */
  const editedVector = () => {
    const open = viewDoc()
    const id = vectorEditRef.current
    if (!open || !id) return null
    const node = findDesignNode(open.doc, id)
    const origin = nodeOrigin(open.doc, id)
    return node?.vector && origin ? { open, id, node, vector: node.vector, origin } : null
  }
  /** One undoable change to the edited vector; `select` replaces the node selection. */
  const commitVector = (change: (vector: DesignVector) => DesignVector | { vector: DesignVector; select: number[] }) => {
    const ctx = editedVector()
    if (!ctx) return
    const result = change(ctx.vector)
    const vector = 'select' in result ? result.vector : result
    if (vector !== ctx.vector) commit(updateDesignNode(ctx.open.doc, ctx.id, (node) => fitVectorNode({ ...node, vector })))
    if ('select' in result) setVectorSel(result.select)
  }
  /** Live update during a node drag, always from the vector as it was when the drag began. */
  const liveVector = (drag: { id: string; base: DesignNode; remembered: boolean }, vector: DesignVector) => {
    const stored = useKoma.getState().design?.docs?.[key]?.doc
    if (!stored) return
    const focus = focusRef.current
    const next = projectDoc(stored, focus, updateDesignNode(editingDoc(stored, focus), drag.id, () => fitVectorNode({ ...drag.base, vector })))
    if (sameDesign(next, stored)) return
    if (!drag.remembered) {
      noteRef.current(stored)
      drag.remembered = true
    }
    updateRef.current(tab.root, tab.path, next)
  }
  const nodeTolerance = () => 6 / viewRef.current.zoom

  const vectorDown = {
    vertex: (event: ReactPointerEvent, index: number) => {
      event.preventDefault()
      event.stopPropagation()
      const ctx = editedVector()
      if (!ctx || event.button !== 0) return
      const start = toDoc(event.clientX, event.clientY)
      if (!start) return
      const meta = event.metaKey || event.ctrlKey
      if (meta && event.altKey) {
        commitVector((vector) => ({ vector: deleteVertices(vector, [index], false), select: [] }))
        return
      }
      if (meta || vectorBendRef.current) {
        dragRef.current = { kind: 'vcurve', id: ctx.id, index, start, base: ctx.node, moved: false, remembered: false }
        return
      }
      const selected = vectorSelRef.current
      let ids = selected.includes(index) ? selected : [index]
      if (event.shiftKey) {
        ids = selected.includes(index) ? selected.filter((i) => i !== index) : [...selected, index]
        setVectorSel(ids)
        if (!ids.includes(index)) return
      } else if (!selected.includes(index)) setVectorSel(ids)
      dragRef.current = {
        kind: 'vnodes',
        id: ctx.id,
        ids,
        start,
        base: ctx.node,
        moved: false,
        remembered: false,
        pressed: index,
        narrow: !event.shiftKey && selected.includes(index) && selected.length > 1,
        alt: event.altKey,
      }
    },
    segment: (event: ReactPointerEvent, index: number) => {
      event.preventDefault()
      event.stopPropagation()
      const ctx = editedVector()
      if (!ctx || event.button !== 0) return
      const start = toDoc(event.clientX, event.clientY)
      if (!start) return
      if (event.altKey) {
        commitVector((vector) => ({ vector: deleteSegments(vector, [index]), select: [] }))
        return
      }
      const hit = closestOnSegment(ctx.vector, index, { x: start.x - ctx.origin.x, y: start.y - ctx.origin.y })
      dragRef.current = {
        kind: 'vseg',
        id: ctx.id,
        segment: index,
        t: hit?.t ?? 0.5,
        start,
        base: ctx.node,
        moved: false,
        remembered: false,
        bend: event.metaKey || event.ctrlKey || vectorBendRef.current,
      }
    },
    /** A press on empty canvas while editing: a node marquee, or leave edit mode for another object. */
    canvas: (event: ReactPointerEvent, point: { x: number; y: number }, hitId: string | null) => {
      const ctx = editedVector()
      if (!ctx) return false
      if (hitId && hitId !== ctx.id) {
        setVectorEditId(null)
        return false
      }
      event.preventDefault()
      const base = event.shiftKey ? vectorSelRef.current : []
      if (!event.shiftKey) setVectorSel([])
      dragRef.current = { kind: 'vmarquee', id: ctx.id, start: point, end: point, base }
      return true
    },
  }
  const vectorDownRef = useRef(vectorDown)
  vectorDownRef.current = vectorDown

  const vectorDrag = {
    move: (drag: Extract<Drag, { kind: 'vnodes' | 'vcurve' | 'vseg' | 'vmarquee' }>, point: { x: number; y: number }, event: PointerEvent) => {
      if (drag.kind === 'vmarquee') {
        drag.end = point
        const ctx = editedVector()
        if (!ctx) return
        const rect = { x: Math.min(drag.start.x, point.x), y: Math.min(drag.start.y, point.y), w: Math.abs(point.x - drag.start.x), h: Math.abs(point.y - drag.start.y) }
        setVectorMarquee(rect)
        const inside = verticesInRect(ctx.vector, { ...rect, x: rect.x - ctx.origin.x, y: rect.y - ctx.origin.y })
        setVectorSel([...new Set([...drag.base, ...inside])])
        return
      }
      const dx = point.x - drag.start.x
      const dy = point.y - drag.start.y
      if (!drag.moved) {
        if (Math.hypot(dx, dy) * viewRef.current.zoom < 3) return
        drag.moved = true
      }
      const base = drag.base.vector
      if (!base) return
      if (drag.kind === 'vcurve') {
        // The press was on the node, so the pointer's travel is the new handle.
        liveVector(drag, pullSmoothPair(base, drag.index, { x: dx, y: dy }))
        return
      }
      if (drag.kind === 'vseg') {
        if (drag.bend) {
          const segment = base.segments[drag.segment]
          const held = pointOnSegment(base, drag.segment, drag.t)
          if (!segment || !held) return
          // The curve point under the press follows the pointer.
          liveVector(drag, bendSegment(base, drag.segment, segment, drag.t, { x: held.x + dx, y: held.y + dy }))
          return
        }
        const segment = base.segments[drag.segment]
        if (!segment) return
        liveVector(drag, moveVertices(base, [segment.start, segment.end], dx, dy))
        return
      }
      // Moving nodes. Alt duplicates them on the first move, Penpot-style.
      if (drag.alt) {
        const copy = duplicateVertices(base, drag.ids)
        // The copy of the pressed node is the one that snaps and follows Shift.
        const pressedAt = drag.ids.indexOf(drag.pressed)
        drag.alt = false
        drag.base = { ...drag.base, vector: copy.vector }
        drag.pressed = copy.ids[pressedAt] ?? copy.ids[0] ?? drag.pressed
        drag.ids = copy.ids
        setVectorSel(copy.ids)
      }
      const vector = drag.base.vector as DesignVector
      const anchor = vector.vertices[drag.ids.includes(drag.pressed) ? drag.pressed : drag.ids[0] as number]
      let mx = dx
      let my = dy
      if (anchor && event.shiftKey) {
        const fixed = fixAngle(anchor, { x: anchor.x + dx, y: anchor.y + dy })
        mx = fixed.x - anchor.x
        my = fixed.y - anchor.y
      } else if (anchor && vectorSnapRef.current) {
        const moving = new Set(drag.ids)
        const others = vector.vertices.filter((_, i) => !moving.has(i))
        const snapped = snapToPoints({ x: anchor.x + dx, y: anchor.y + dy }, others, nodeTolerance())
        mx = snapped.x - anchor.x
        my = snapped.y - anchor.y
      }
      liveVector(drag, moveVertices(vector, drag.ids, mx, my))
    },
    up: (drag: Extract<Drag, { kind: 'vnodes' | 'vcurve' | 'vseg' | 'vmarquee' }>) => {
      if (drag.kind === 'vmarquee') {
        setVectorMarquee(null)
        return
      }
      if (drag.kind === 'vcurve') {
        if (!drag.moved) commitVector((vector) => setNodeType(vector, [drag.index], nodeType(vector, drag.index) === 'corner' ? 'mirror' : 'corner'))
        return
      }
      if (drag.kind === 'vseg') {
        if (drag.moved || drag.bend) return
        // A click on the outline adds a node exactly there.
        commitVector((vector) => {
          const split = splitSegment(vector, drag.segment, drag.t)
          return { vector: split.vector, select: split.vertex >= 0 ? [split.vertex] : [] }
        })
        return
      }
      if (!drag.moved) {
        if (drag.alt) commitVector((vector) => ({ vector: deleteVertices(vector, [drag.pressed], true), select: [] }))
        else if (drag.narrow) setVectorSel([drag.pressed])
        return
      }
      // A node dropped onto another merges into it (Penpot merge-dragged-on-drop).
      const ctx = editedVector()
      if (!ctx || !vectorSnapRef.current) return
      const merged = mergeOnDrop(ctx.vector, drag.ids, nodeTolerance())
      if (merged === ctx.vector) return
      const stored = useKoma.getState().design?.docs?.[key]?.doc
      if (!stored) return
      const focus = focusRef.current
      updateRef.current(tab.root, tab.path, projectDoc(stored, focus, updateDesignNode(ctx.open.doc, ctx.id, (node) => fitVectorNode({ ...node, vector: merged }))))
      setVectorSel([])
    },
  }
  const vectorDragRef = useRef(vectorDrag)
  vectorDragRef.current = vectorDrag

  /** Node edits from the toolbar or the keyboard. Returns false when the action needs nodes it does not have. */
  const vectorAction = (action: PathBarAction): boolean => {
    const ids = vectorSelRef.current
    if (action === 'draw') {
      setVectorBend(false)
      chooseTool('pen')
      return true
    }
    if (action === 'move' || action === 'bend') {
      setVectorBend(action === 'bend')
      setTool('select')
      return true
    }
    if (action === 'snap') {
      setVectorSnap((on) => !on)
      return true
    }
    if (!ids.length) return false
    if (typeof action === 'object') {
      commitVector((vector) => setNodeType(vector, ids, action.handles))
      return true
    }
    if (action === 'delete') commitVector((vector) => ({ vector: deleteVertices(vector, ids, true), select: [] }))
    else if (action === 'corner') commitVector((vector) => setNodeType(vector, ids, 'corner'))
    else if (action === 'curve') commitVector((vector) => setNodeType(vector, ids, 'mirror'))
    else if (action === 'separate') commitVector((vector) => separateVertices(vector, ids))
    else if (action === 'flipH') commitVector((vector) => flipVertices(vector, ids, 'horizontal'))
    else if (action === 'flipV') commitVector((vector) => flipVertices(vector, ids, 'vertical'))
    else if (action === 'merge') commitVector((vector) => ({ vector: mergeVertices(vector, ids), select: [Math.min(...ids)] }))
    else if (action === 'join') {
      if (ids.length !== 2) return false
      commitVector((vector) => joinVertices(vector, ids[0] as number, ids[1] as number))
    } else if (action === 'add') {
      // A node in the middle of every segment whose two ends are both selected.
      commitVector((vector) => {
        const picked = new Set(ids)
        const targets = vector.segments.flatMap((segment, index) => (picked.has(segment.start) && picked.has(segment.end) ? [index] : []))
        let next = vector
        const added: number[] = []
        for (const index of targets) {
          const split = splitSegment(next, index, 0.5)
          next = split.vector
          added.push(split.vertex)
        }
        return { vector: next, select: [...ids, ...added] }
      })
    }
    return true
  }

  /** Node-editing keys, ahead of the canvas shortcuts. Returns true when it handled the key. */
  const vectorKey = (event: KeyboardEvent): boolean => {
    if (!vectorEditRef.current || toolRef.current !== 'select') return false
    const meta = event.metaKey || event.ctrlKey
    const lower = event.key.toLowerCase()
    const run = (action: PathBarAction) => {
      event.preventDefault()
      vectorAction(action)
      return true
    }
    if (event.key === 'Delete' || event.key === 'Backspace') {
      event.preventDefault()
      const ids = vectorSelRef.current
      if (ids.length) commitVector((vector) => ({ vector: deleteVertices(vector, ids, !event.shiftKey), select: [] }))
      return true
    }
    if (event.key.startsWith('Arrow') && !meta) {
      event.preventDefault()
      const step = event.shiftKey ? 10 : 1
      const dx = event.key === 'ArrowLeft' ? -step : event.key === 'ArrowRight' ? step : 0
      const dy = event.key === 'ArrowUp' ? -step : event.key === 'ArrowDown' ? step : 0
      const ids = vectorSelRef.current
      if (ids.length) commitVector((vector) => moveVertices(vector, ids, dx, dy))
      return true
    }
    if (meta && !event.altKey && lower === 'a') {
      event.preventDefault()
      const ctx = editedVector()
      setVectorSel(event.shiftKey || !ctx ? [] : ctx.vector.vertices.map((_, i) => i))
      return true
    }
    if (meta && !event.shiftKey && !event.altKey && lower === 'j') return run('merge')
    if (meta && !event.altKey && event.key === "'") return run('snap')
    if (meta && !event.shiftKey && !event.altKey && (lower === 'c' || lower === 'x')) {
      const ctx = editedVector()
      const ids = vectorSelRef.current
      if (!ctx || !ids.length) return false
      event.preventDefault()
      vectorClipboard = extractVertices(ctx.vector, ids)
      if (lower === 'x') commitVector((vector) => ({ vector: deleteVertices(vector, ids, false), select: [] }))
      return true
    }
    if (meta && !event.shiftKey && !event.altKey && (lower === 'v' || lower === 'd')) {
      const piece = lower === 'v' ? vectorClipboard : (() => {
        const ctx = editedVector()
        return ctx && vectorSelRef.current.length ? extractVertices(ctx.vector, vectorSelRef.current) : null
      })()
      if (!piece) return false
      event.preventDefault()
      commitVector((vector) => {
        const pasted = appendVector(vector, piece, 10, 10)
        return { vector: pasted.vector, select: pasted.ids }
      })
      return true
    }
    if (meta || event.altKey) return false
    if (event.shiftKey && lower === 'h') return run('flipH')
    if (event.shiftKey && lower === 'v') return run('flipV')
    if (event.shiftKey && (event.key === '+' || event.key === '=')) return run('add')
    if (event.shiftKey) return false
    if (lower === 'j') return run('join')
    if (lower === 'k') return run('separate')
    if (lower === 'x') return run('corner')
    if (lower === 'c') return run('curve')
    if (lower === 'p') return run('draw')
    if (lower === 'm') return run('move')
    if (lower === 'b') return run('bend')
    return false
  }
  const vectorKeyRef = useRef(vectorKey)
  vectorKeyRef.current = vectorKey
  cancelPenRef.current = cancelPen
  chooseToolRef.current = chooseTool
  vectorEditRef.current = vectorEditId

  const viewDoc = () => {
    const stored = useKoma.getState().design?.docs?.[key]?.doc
    if (!stored) return null
    return { stored, doc: editingDoc(stored, focusRef.current) }
  }

  const mutate = (next: DesignDoc | null) => {
    if (!next) return
    commit(next)
  }

  commandsRef.current = {
    copy: () => {
      const open = viewDoc()
      if (!open || !selectionRef.current.length) return
      const rows = designRoots(open.doc, selectionRef.current).map((id) => locateDesign(open.doc, id)).filter((row) => row != null)
      if (!rows.length) return
      // Shapes from different parents travel in canvas space and land side by side.
      const shared = rows.every((row) => row.parentId === rows[0].parentId)
      const nodes = shared ? rows.map((row) => row.node) : rows.map((row) => {
        const origin = nodeBoxOrigin(open.doc, row.node.id)
        return origin ? { ...row.node, x: origin.x, y: origin.y } : row.node
      })
      const components = open.doc.components.filter((component) => rows.some((row) => row.node.kind === 'instance' && row.node.component === component.id))
      const images = open.doc.images
      const slice = { nodes, parentId: shared ? rows[0].parentId : null, components: components.length ? components : undefined, images }
      setCopiedShape(slice)
      void navigator.clipboard?.writeText(serializeDesignSlice(slice.nodes, { components, images })).catch(() => undefined)
    },
    paste: (at) => {
      const open = viewDoc()
      if (!open || file?.loading) return
      const clip = getCopiedShape()
      const sources = clip?.nodes.length ? clip.nodes : []
      const finish = (nodes: DesignNode[], extras?: { parentId?: string | null; components?: DesignDoc['components']; images?: DesignDoc['images'] }) => {
      const source = extras ? extras.parentId : clip?.parentId
      const lone = selectionRef.current.length === 1 ? findDesignNode(open.doc, selectionRef.current[0]) : null
      // Paste lands in a selected frame, unless it carries frames itself; otherwise next to where it was copied.
      const into = !at && lone?.kind === 'frame' && !nodes.some((node) => node.kind === 'frame') ? lone.id : null
      const parentId = at
        ? frameAtPoint(open.doc, at.x, at.y, '')
        : into ?? (source && locateDesign(open.doc, source) ? source : null)
      const minX = Math.min(...nodes.map((node) => node.x))
      const minY = Math.min(...nodes.map((node) => node.y))
      const spanW = Math.max(...nodes.map((node) => node.x + node.w)) - minX
      const spanH = Math.max(...nodes.map((node) => node.y + node.h)) - minY
      let shift = { x: 0, y: 0 }
      if (at) {
        const local = canvasToContent(open.doc, parentId, at.x, at.y)
        if (local) shift = { x: local.x - minX, y: local.y - minY }
      } else if (into && into !== source && lone) {
        shift = { x: (lone.w - spanW) / 2 - minX, y: (lone.h - spanH) / 2 - minY }
      } else {
        // In place while the copied spot is on screen; else in the middle of the view.
        const view = viewportBox()
        const corner = parentId ? nodeCanvasPoint(open.doc, parentId, minX, minY) : { x: minX, y: minY }
        const visible = !view || !corner || (corner.x < view.x + view.w && corner.x + spanW > view.x && corner.y < view.y + view.h && corner.y + spanH > view.y)
        const center = view && !visible ? canvasToContent(open.doc, parentId, view.x + view.w / 2, view.y + view.h / 2) : null
        if (center) shift = { x: center.x - spanW / 2 - minX, y: center.y - spanH / 2 - minY }
      }
      let next = open.doc
      const pasted: DesignNode[] = []
      for (const item of nodes) {
        const node = copyTree(item, () => mintId('n'))
        node.x = snapDesign(item.x + shift.x, open.doc.grid, open.doc.snap)
        node.y = snapDesign(item.y + shift.y, open.doc.grid, open.doc.snap)
        if (parentId) next = insertDesignNode(next, parentId, node)
        else if (focusRef.current && node.kind !== 'frame' && node.kind !== 'group' && next.screens[0]) next = insertDesignNode(next, next.screens[0].id, node)
        else if (focusRef.current && node.kind !== 'frame' && node.kind !== 'group') {
          const screen = createNode('frame', mintId('f'), node.x, node.y)
          node.x = 16
          node.y = 16
          screen.children = [node]
          next = insertDesignNode(next, null, screen)
        } else next = insertDesignNode(next, null, node)
        pasted.push(node)
      }
      const components = extras?.components ?? clip?.components
      const images = extras?.images ?? clip?.images
      if (components?.length) {
        const more = components.filter((component) => !next.components.some((item) => item.id === component.id))
        if (more.length) next = { ...next, components: [...next.components, ...more] }
      }
      if (images) next = { ...next, images: { ...images, ...next.images } }
      setCopiedShape({ nodes: pasted, parentId, components, images })
      commit(next)
      setSelection(pasted.map((node) => node.id))
      setPropsOpen(true)
      }
      if (sources.length) {
        finish(sources)
        return
      }
      void navigator.clipboard?.readText().then((text) => {
        const slice = parseDesignSlice(text)
        if (slice) finish(slice.nodes, { components: slice.components, images: slice.images })
      }).catch(() => undefined)
    },
    duplicate: () => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length || file?.loading) return
      const step = open.doc.snap && open.doc.grid > 0 ? open.doc.grid : 10
      const duplicated = duplicateDesignNodes(open.doc, ids, step, step, () => mintId('n'))
      if (!duplicated) return
      commit(duplicated.doc)
      setSelection(duplicated.ids)
      setPropsOpen(true)
    },
    copyStyle: () => {
      const open = viewDoc()
      const id = selectionRef.current[selectionRef.current.length - 1]
      if (!open || !id) return
      const located = locateDesign(open.doc, id)
      if (!located) return
      setCopiedStyle(designStyle(located.node))
    },
    pasteStyle: () => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length || !getCopiedStyle()) return
      const style = getCopiedStyle()
      if (!style) return
      commit(applyDesignStyle(open.doc, ids, style))
    },
    selectAll: () => {
      const open = viewDoc()
      if (!open) return
      setSelection(selectAllDesign(open.doc, selectionRef.current))
      setPropsOpen(true)
    },
    nudge: (dx, dy, duplicate) => {
      const open = viewDoc()
      if (!open || file?.loading) return
      let view = open.doc
      let ids = selectionRef.current
      if (!ids.length) return
      if (duplicate) {
        const copied = duplicateDesignNodes(view, ids, 0, 0, () => mintId('n'))
        if (!copied) return
        view = copied.doc
        ids = copied.ids
        selectionRef.current = ids
        setSelection(ids)
      }
      view = nudgeDesignNodes(view, ids, dx, dy)
      const stored = useKoma.getState().design?.docs?.[key]?.doc
      if (!stored) return
      const next = projectDoc(stored, focusRef.current, view)
      if (sameDesign(stored, next)) return
      if (!nudgeOpenRef.current) {
        note(stored)
        nudgeOpenRef.current = true
      }
      if (nudgeTimerRef.current) clearTimeout(nudgeTimerRef.current)
      nudgeTimerRef.current = setTimeout(() => {
        nudgeTimerRef.current = null
        nudgeOpenRef.current = false
      }, 300)
      updateDesign(tab.root, tab.path, next)
    },
    zoom: (scope) => {
      zoomTo(scope === 'all' ? null : selectionRef.current)
    },
    remove: () => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      const next = ids.reduce((doc, id) => deleteDesignNode(doc, id), open.doc)
      commit(next)
      setSelection([])
      setEditing(null)
    },
    wrap: (kind) => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      if (focusRef.current && ids.some((id) => locateDesign(open.doc, id)?.parentId == null)) return
      const id = mintId(kind[0])
      const next = wrapDesignNodes(open.doc, ids, kind, id)
      if (!next) return
      commit(next)
      setSelection([id])
    },
    unwrap: () => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      let next = open.doc
      const released: string[] = []
      for (const id of ids) {
        const children = findDesignNode(next, id)?.children?.map((child) => child.id) ?? []
        const unwrapped = unwrapDesignNode(next, id)
        if (!unwrapped) continue
        next = unwrapped
        released.push(...children)
      }
      if (!released.length || sameDesign(next, open.doc)) return
      commit(next)
      setSelection(released)
    },
    order: (order) => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      const ranked = ids.flatMap((id) => {
        const row = locateDesign(open.doc, id)
        if (!row) return []
        const siblings = row.parentId == null ? open.doc.screens : findDesignNode(open.doc, row.parentId)?.children ?? []
        const index = siblings.findIndex((item) => item.id === id)
        return index < 0 ? [] : [{ id, index }]
      }).sort((a, b) => a.index - b.index)
      const sequence = order === 'back' || order === 'backward' ? ranked.reverse() : ranked
      mutate(sequence.reduce((current, row) => orderDesignNode(current, row.id, order), open.doc))
    },
    flip: (axis) => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      mutate(ids.reduce((doc, id) => flipDesignNode(doc, id, axis), open.doc))
    },
    hide: () => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      const show = ids.some((id) => locateDesign(open.doc, id)?.node.visible === false)
      mutate(ids.reduce((doc, id) => setDesignVisible(doc, id, show), open.doc))
    },
    lock: () => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      const unlock = ids.every((id) => locateDesign(open.doc, id)?.node.locked)
      mutate(ids.reduce((doc, id) => setDesignLocked(doc, id, !unlock), open.doc))
    },
    auto: () => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      const next = autoLayoutDesign(open.doc, ids, mintId('f'))
      if (next) commit(next)
    },
    component: () => {
      const stored = useKoma.getState().design?.docs?.[key]?.doc
      const id = selectionRef.current.length === 1 ? selectionRef.current[0] : null
      if (!stored || !id || focusRef.current) return
      const componentId = mintId('c')
      const next = createComponentFromFrame(stored, id, componentId, () => mintId('n'))
      if (!next) return
      commitStored(next)
      setFocusId(componentId)
      setSelection([])
    },
    select: (id) => {
      setSelection([id])
      setPropsOpen(true)
    },
    step: (dir) => {
      const open = viewDoc()
      const id = selectionRef.current[selectionRef.current.length - 1]
      if (!open || !id) return
      setSelection([stepDesignSibling(open.doc, id, dir)])
      setPropsOpen(true)
    },
    selectChildren: () => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      const lone = ids.length === 1 ? findDesignNode(open.doc, ids[0]) : null
      if (lone?.kind === 'text' && !lone.locked) {
        setEditing(lone.id)
        labelNoted.current = false
        return
      }
      const children = designChildIds(open.doc, ids)
      if (ids.length === 1 && children[0] !== ids[0]) setEnteredContainerId(ids[0])
      setSelection(children)
    },
    selectParent: () => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      const parents = designParentIds(open.doc, ids)
      setSelection(parents)
      setEnteredContainerId(parents.length === 1 ? designEnterScopeForLayerSelect(open.doc, parents[0]) : null)
    },
    zoomStep: (factor) => {
      const rect = canvasRef.current?.getBoundingClientRect()
      if (!rect) return
      zoomAt(rect.left + rect.width / 2, rect.top + rect.height / 2, factor ? viewRef.current.zoom * factor : 1)
    },
    boolean: (op) => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      // Combining several shapes mints a new group, which should end up selected.
      let made: string | null = null
      const next = applyDesignBoolean(open.doc, ids, op, () => (made = mintId('b')))
      if (!next) return
      commit(next)
      if (made) setSelection([made])
    },
    outline: () => {
      const open = viewDoc()
      const ids = selectionRef.current
      if (!open || !ids.length) return
      mutate(ids.reduce((doc, id) => updateDesignNode(doc, id, outlineStrokeNode), open.doc))
    },
    detach: () => {
      const stored = useKoma.getState().design?.docs?.[key]?.doc
      const id = selectionRef.current[0]
      if (!stored || !id) return
      const next = detachInstance(stored, id, () => mintId('n'))
      if (next) commitStored(next)
    },
  }

  layerOpsRef.current = (action) => {
    const open = viewDoc()
    if (!open) return
    if (action.op === 'enter') {
      setEnteredContainerId(action.id)
      setSelection([action.id])
      setPropsOpen(true)
      setEditing(null)
      return
    }
    if (action.op === 'select') {
      const ids = selectionRef.current
      const last = action.ids[action.ids.length - 1]
      if (!last) return
      if (action.toggle) setSelection(ids.includes(last) ? ids.filter((item) => item !== last) : [...ids, last])
      else {
        setSelection(action.ids)
        setEnteredContainerId(designEnterScopeForLayerSelect(open.doc, last))
      }
      setPropsOpen(true)
      setEditing(null)
      return
    }
    if (action.op === 'rename') {
      const trimmed = action.name.trim()
      commit(updateDesignNode(open.doc, action.id, (node) => {
        const next = { ...node }
        if (trimmed) next.name = trimmed
        else delete next.name
        return next
      }))
      return
    }
    if (action.op === 'visible') {
      commit(setDesignVisible(open.doc, action.id, action.visible))
      return
    }
    if (action.op === 'locked') {
      commit(setDesignLocked(open.doc, action.id, action.locked))
      return
    }
    if (action.op === 'move') {
      const ids = action.ids.filter((id) => {
        if (focusRef.current && action.parentId == null && locateDesign(open.doc, id)?.parentId != null) return false
        return canLeaveParent(open.doc, id, action.parentId)
      })
      if (ids.length) commit(moveDesignNodes(open.doc, ids, action.parentId, action.targetId, action.place))
      return
    }
    if (!selectionRef.current.includes(action.id)) setSelection([action.id])
    const origin = nodeOrigin(open.doc, action.id)
    setMenu({ x: action.x, y: action.y, canvasX: origin?.x ?? 0, canvasY: origin?.y ?? 0 })
  }

  const beginPen = (point: { x: number; y: number }, time: number) => {
    const open = viewDoc()
    if (!open) return
    const draft = penRef.current
    const zero = { x: 0, y: 0 }
    const previous = penClickRef.current
    penClickRef.current = { time, x: point.x, y: point.y }
    if (draft && previous && time - previous.time < 400 && Math.hypot(previous.x - point.x, previous.y - point.y) <= 4 / viewRef.current.zoom) {
      // A double-click ends an open path where it is, like Penpot.
      penClickRef.current = null
      finishPen(false)
      return
    }
    if (draft && draft.points.length >= 3) {
      const first = draft.points[0]
      const threshold = 8 / viewRef.current.zoom
      if (Math.hypot(first.x - point.x, first.y - point.y) <= threshold) {
        applyPen(draft, true)
        penRef.current = null
        setPen(null)
        setPenHover(null)
        setPenHandle(null)
        penNoted.current = false
        setTool('select')
        return
      }
    }
    if (!draft) {
      penNoted.current = false
      const parentId = focusRef.current ? frameAtPoint(open.doc, point.x, point.y, '') ?? open.doc.screens.find((screen) => screen.kind === 'frame' || screen.kind === 'group')?.id ?? null : null
      const resume = selectionRef.current.map((id) => findDesignNode(open.doc, id)).find((node) => node?.kind === 'vector' && node.vector)
      if (resume?.vector) {
        const origin = nodeOrigin(open.doc, resume.id)
        const threshold = 8 / viewRef.current.zoom
        const end = origin ? openVectorEndpoints(resume).find((item) => Math.hypot(origin.x + item.x - point.x, origin.y + item.y - point.y) <= threshold) : null
        if (end && origin) {
          const points = resume.vector.vertices.map((vertex) => ({ x: origin.x + vertex.x, y: origin.y + vertex.y, incoming: zero, outgoing: zero }))
          if (end.index === 0) points.reverse()
          const next = { id: resume.id, parentId: locateDesign(open.doc, resume.id)?.parentId ?? parentId, points }
          penRef.current = next
          setPen(next)
          setPenHandle(points.length - 1)
          setVectorEditId(resume.id)
          dragRef.current = { kind: 'pen', index: points.length - 1, space: spaceRef.current }
          return
        }
      }
      const next = { id: mintId('v'), parentId, points: [{ x: point.x, y: point.y, incoming: zero, outgoing: zero }] }
      penRef.current = next
      setPen(next)
      setPenHandle(0)
      applyPen(next)
      dragRef.current = { kind: 'pen', index: 0, space: spaceRef.current }
      return
    }
    const points = [...draft.points, { x: point.x, y: point.y, incoming: zero, outgoing: zero }]
    const next = { ...draft, points }
    penRef.current = next
    setPen(next)
    setPenHandle(points.length - 1)
    applyPen(next)
    dragRef.current = { kind: 'pen', index: points.length - 1, space: spaceRef.current }
  }

  const startPan = (event: ReactPointerEvent | PointerEvent) => {
    dragRef.current = { kind: 'pan', lastX: event.clientX, lastY: event.clientY }
    setDragCursor('grabbing')
  }

  /** Outline what a click would select, once per frame. Cmd or Ctrl hovers the deepest shape. */
  const onCanvasHover = (event: ReactPointerEvent<HTMLDivElement>) => {
    const pending = hoverFrameRef.current
    pending.clientX = event.clientX
    pending.clientY = event.clientY
    pending.deep = deepSelectKey(event)
    if (pending.frame) return
    pending.frame = requestAnimationFrame(() => {
      pending.frame = 0
      const stored = useKoma.getState().design?.docs?.[key]?.doc
      const point = toDoc(pending.clientX, pending.clientY)
      if (!stored || !point || dragRef.current || toolRef.current !== 'select' || spaceRef.current) {
        setDesignHover(null)
        return
      }
      const current = editingDoc(stored, focusRef.current)
      const hit = pending.deep ? hitDesignDeepest(current, point.x, point.y)?.id : selectDesignHit(current, point.x, point.y, false, enteredContainerRef.current)
      setDesignHover(hit ?? null)
    })
  }

  const onCanvasPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button === 1 || spaceRef.current || tool === 'pan') {
      event.preventDefault()
      startPan(event)
      return
    }
    if (event.button !== 0) return
    setMenu(null)
    const point = toDoc(event.clientX, event.clientY)
    if (!point || !file) return
    if (tool === 'pen') {
      event.preventDefault()
      beginPen(penTargetRef.current(point, event.shiftKey), event.timeStamp)
      return
    }
    if (tool === 'polygon' || tool === 'star' || tool === 'triangle') {
      event.preventDefault()
      const stored = useKoma.getState().design?.docs?.[key]?.doc
      if (!stored) return
      const node =
        tool === 'star'
          ? createStarNode(mintId('s'), point.x, point.y)
          : tool === 'triangle'
            ? { ...createPolygonNode(mintId('p'), point.x, point.y, 100, 3), name: 'Triangle' }
            : createPolygonNode(mintId('p'), point.x, point.y)
      commit(insertDesignNode(editingDoc(stored, focusRef.current), null, node))
      setSelection([node.id])
      setTool('select')
      return
    }
    if (tool === 'frame' || tool === 'section' || tool === 'rect' || tool === 'ellipse' || tool === 'line' || tool === 'text') {
      event.preventDefault()
      const stored = useKoma.getState().design?.docs?.[key]?.doc
      const doc = stored ? editingDoc(stored, focusRef.current) : null
      dragRef.current = {
        kind: 'draw',
        shape: tool === 'section' ? 'frame' : tool,
        cx: point.x,
        cy: point.y,
        x0: point.x,
        y0: point.y,
        x1: point.x,
        y1: point.y,
        parentId: doc && (tool === 'frame' || focusRef.current) ? frameAtPoint(doc, point.x, point.y, '') : null,
      }
      return
    }
    const storedNow = useKoma.getState().design?.docs?.[key]?.doc
    const current = storedNow ? editingDoc(storedNow, focusRef.current) : null
    if (!current) return
    if (vectorEditRef.current && tool === 'select') {
      // Editing a path: empty canvas (or the path's own fill) starts a node marquee;
      // pressing another object leaves edit mode and selects it as usual.
      const under = resolveDesignSelectHit(current, point.x, point.y, enteredContainerRef.current)
      const hitId = under.kind === 'hit' || under.kind === 'exit-and-hit' ? under.id : null
      if (vectorDownRef.current.canvas(event, point, hitId)) return
    }
    const deepId = deepSelectKey(event) ? hitDesignDeepest(current, point.x, point.y)?.id : undefined
    const resolved = deepId ? { kind: 'hit' as const, id: deepId } : resolveDesignSelectHit(current, point.x, point.y, enteredContainerRef.current)
    if (resolved.kind === 'exit-and-hit') setEnteredContainerId(resolved.enteredContainerId)
    const target = resolved.kind === 'hit' ? resolved.id : resolved.kind === 'exit-and-hit' ? resolved.id : null
    if (!target) {
      if (resolved.kind === 'clear' && !enteredContainerRef.current) {
        setSelection([])
        dragRef.current = { kind: 'marquee', x0: point.x, y0: point.y, x1: point.x, y1: point.y }
      } else if (resolved.kind === 'clear' || resolved.kind === 'exit-and-hit') {
        setSelection([])
      }
      return
    }
    const previous = selectionRef.current
    const ids = event.shiftKey
      ? previous.includes(target) ? previous.filter((item) => item !== target) : [...previous, target]
      : previous.includes(target) ? previous : [target]
    const moveIds = designRoots(current, ids)
    selectionRef.current = ids
    setSelection(ids)
    setPropsOpen(true)
    if (event.shiftKey || locateDesign(current, target)?.node.locked) return
    if (cropEditRef.current && ids.length === 1 && ids[0] === target) {
      const paint = nodePaints(locateDesign(current, target)?.node, 'fill').find(isImageCropPaint)
      if (paint) {
        dragRef.current = { kind: 'crop', id: target, startX: event.clientX, startY: event.clientY, paint: { ...paint }, remembered: false }
        setDragCursor('move')
        return
      }
    }
    const origins: Record<string, { x: number; y: number }> = {}
    for (const item of moveIds) {
      const row = locateDesign(current, item)
      if (row) origins[item] = { x: row.node.x, y: row.node.y }
    }
    dragRef.current = {
      kind: 'move',
      ids: moveIds,
      startX: event.clientX,
      startY: event.clientY,
      origins,
      remembered: false,
      moved: false,
      broke: false,
      alt: event.altKey,
      scene: null,
    }
    setDragCursor('default')
  }

  const placeInstance = (componentId: string, point: { x: number; y: number }) => {
    focusRef.current = null
    setFocusId(null)
    const stored = useKoma.getState().design?.docs?.[key]?.doc
    if (!stored) return
    const node = makeInstance(stored, componentId, mintId('i'), 0, 0)
    if (!node) return
    const parent = frameAtPoint(stored, point.x, point.y, '')
    let next = stored
    if (parent) {
      const local = canvasToContent(stored, parent, point.x, point.y)
      if (!local) return
      node.x = snapDesign(local.x, stored.grid, stored.snap)
      node.y = snapDesign(local.y, stored.grid, stored.snap)
      next = insertDesignNode(stored, parent, node)
    } else if (focusRef.current) {
      const screen = createNode('frame', mintId('f'), snapDesign(point.x, stored.grid, stored.snap), snapDesign(point.y, stored.grid, stored.snap))
      node.x = 16
      node.y = 16
      screen.children = [node]
      next = insertDesignNode(stored, null, screen)
    } else {
      node.x = snapDesign(point.x, stored.grid, stored.snap)
      node.y = snapDesign(point.y, stored.grid, stored.snap)
      next = insertDesignNode(stored, null, node)
    }
    commitStored(next)
    setSelection([node.id])
    setEditing(null)
    setPropsOpen(true)
    setTool('select')
  }

  const onDrop = (event: React.DragEvent<HTMLDivElement>) => {
    event.preventDefault()
    const componentId = event.dataTransfer.getData(COMPONENT_MIME)
    const point = toDoc(event.clientX, event.clientY)
    if (!point) return
    if (componentId) {
      placeInstance(componentId, point)
      return
    }
    const files = [...(event.dataTransfer.files ?? [])]
    if (files.some((file) => (file.type || mimeForName(file.name))?.startsWith('image/'))) {
      void importImages(files, point)
      return
    }
    const kind = event.dataTransfer.getData(DESIGN_MIME)
    if (kind !== 'frame' && kind !== 'rect' && kind !== 'ellipse' && kind !== 'line' && kind !== 'text') return
    placeAt(kind, point)
  }

  if (!file) {
    return <div className="flex h-full items-center justify-center text-[12px] text-koma-dim">Loading…</div>
  }

  const storedDoc = file.doc
  const doc = editingDoc(storedDoc, focusId)
  const paintDoc = paintOnlyDoc(paintDocRef, doc)
  const selectedNodes = selection.flatMap((id) => {
    const hit = locateDesign(doc, id)
    return hit ? [hit.node] : []
  })
  const selectedId = selectedNodes[selectedNodes.length - 1]?.id ?? null
  const located = selectedId ? locateDesign(doc, selectedId) : null
  const selected = located?.node ?? null
  const multi = selectedNodes.length > 1
  // Alt over another shape shows the distances from the selection to it.
  const altFrom = altDown && hoverId && selection.length && !dragCursor && !selection.includes(hoverId) ? designSelectionCanvasBox(doc, selection) : null
  const altTo = altFrom && hoverId ? designCanvasBox(doc, hoverId) : null
  const altMeasures = altFrom && altTo ? designDistances(altFrom, altTo) : []
  const marks = snapMarks ?? (altMeasures.length ? { guides: [], measures: altMeasures } : null)
  const cropEdit = !multi && isImageCropPaint(inspectorPaint)
  cropEditRef.current = cropEdit
  const parentNode = located?.parentId ? findDesignNode(doc, located.parentId) : null
  const sizeModes = !multi && (parentNode?.layout === 'row' || parentNode?.layout === 'column')
  const everyParent = selectedNodes.length > 0 && selectedNodes.every((node) => locateDesign(doc, node.id)?.parentId != null)
  const focusedComponent = focusId ? storedDoc.components.find((component) => component.id === focusId) ?? null : null
  const selectedVariant = focusedComponent && located?.parentId == null
    ? focusedComponent.variants.find((variant) => variant.node.id === selected?.id) ?? null
    : null
  const chatQuery: DesignQuery | null = focusedComponent
    ? { component: focusedComponent.id, variant: selectedVariant?.props }
    : selected?.kind === 'instance' && selected.component
      ? { component: selected.component, variant: selected.variant }
      : selected && located?.parentId == null
        ? { screen: selected.id }
        : null
  const chain = selectedId ? designPath(doc, selectedId) ?? [] : []
  const enteredBox = enteredContainerId ? designCanvasBox(doc, enteredContainerId) : null
  const enterContainerAt = (containerId: string, clientX: number, clientY: number) => {
    const storedNow = useKoma.getState().design?.docs?.[key]?.doc
    const current = storedNow ? editingDoc(storedNow, focusRef.current) : doc
    const point = toDoc(clientX, clientY)
    setEnteredContainerId(containerId)
    if (!point) {
      setSelection([])
      return
    }
    const inner = hitDesignInScope(current, containerId, point.x, point.y, true)
    setSelection(inner && inner.id !== containerId ? [inner.id] : [])
    setPropsOpen(true)
  }
  const stepOut = () => {
    if (enteredContainerId) {
      const { nextEnteredId, selectId } = exitDesignContainer(doc, enteredContainerId)
      setEnteredContainerId(nextEnteredId)
      if (selectId) setSelection([selectId])
      return
    }
    if (selection.length) {
      const id = selection[selection.length - 1]
      const row = id ? locateDesign(doc, id) : null
      if (row?.parentId) {
        setSelection([row.parentId])
        return
      }
      setSelection([])
      return
    }
    if (focusId) {
      setFocusId(null)
      setSelection([])
    }
  }
  stepOutRef.current = stepOut
  const sendChat = () => {
    if (!chatQuery) return
    const text = designChatText(storedDoc, chatQuery, tab.root)
    const node = designQueryNode(storedDoc, chatQuery)
    if (!text || !node) return
    const png = designPngBase64(storedDoc, node)
    const title = designChatTitle(storedDoc, chatQuery)
    if (png) {
      const stem = title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 40) || 'design'
      useKoma.getState().req({ r: 'AttachFile', name: `${stem}.png`, bytesB64: png, mime: 'image/png' })
    }
    useKoma.getState().addDesignToChat({ title, text })
  }
  const status = file.saving
    ? 'Saving…'
    : file.loading && file.savedText == null
      ? 'Loading…'
      : file.error && !file.dirty
        ? file.error
        : file.dirty
          ? 'Modified'
          : 'Saved'
  const canSave = file.dirty && !file.saving && !file.loading
  const canRevert = file.dirty && !!file.savedText && !file.saving && !file.loading
  const grid = doc.grid > 0 ? doc.grid : 8
  void rev

  const patchSelected = (fn: (node: DesignNode) => DesignNode, noteOnce = false) => {
    if (!selection.length) return
    const stored = useKoma.getState().design?.docs?.[key]?.doc
    if (!stored) return
    let view = editingDoc(stored, focusRef.current)
    for (const id of selection) view = updateDesignNode(view, id, fn)
    const next = projectDoc(stored, focusRef.current, view)
    if (sameDesign(next, stored)) return
    if (noteOnce) {
      if (!labelNoted.current) {
        note(stored)
        labelNoted.current = true
      }
      updateDesign(tab.root, tab.path, next)
      setRev((value) => value + 1)
      return
    }
    commit(view)
  }

  const flushDebouncedTypePatch = () => {
    if (patchDebounceRef.current) {
      clearTimeout(patchDebounceRef.current)
      patchDebounceRef.current = null
    }
    const fn = pendingTypePatchRef.current
    pendingTypePatchRef.current = null
    if (fn) patchSelected(fn, true)
  }

  const debouncedTypePatch = (fn: (node: DesignNode) => DesignNode) => {
    pendingTypePatchRef.current = fn
    if (patchDebounceRef.current) clearTimeout(patchDebounceRef.current)
    patchDebounceRef.current = setTimeout(() => {
      patchDebounceRef.current = null
      const apply = pendingTypePatchRef.current
      pendingTypePatchRef.current = null
      if (apply) patchSelected(apply, true)
    }, DESIGN_PATCH_DEBOUNCE_MS)
  }

  nodeHandlersRef.current = {
    onOpenFill: (id) => {
      setSelection([id])
      setPropsOpen(true)
      setOpenFillToken((token) => token + 1)
    },
    onEnterContainer: (id, event) => {
      event.preventDefault()
      event.stopPropagation()
      const hit = findDesignNode(doc, id)
      if (hit?.kind === 'vector' && !isBooleanNode(hit)) {
        setSelection([id])
        setVectorEditId(id)
        setTool('select')
        return
      }
      enterContainerAt(id, event.clientX, event.clientY)
    },
    onMenu: (id, clientX, clientY) => {
      const point = toDoc(clientX, clientY)
      if (!selection.includes(id)) setSelection([id])
      if (point) setMenu({ x: clientX, y: clientY, canvasX: point.x, canvasY: point.y })
    },
    onSelect: (id, event, exact) => {
      setOverrideTargetId(null)
      if (event.button === 0) setMenu(null)
      if (event.button === 1 || spaceRef.current || toolRef.current === 'pan') {
        event.preventDefault()
        event.stopPropagation()
        startPan(event)
        return
      }
      if (event.button !== 0 || toolRef.current !== 'select') return
      event.preventDefault()
      event.stopPropagation()
      const storedNow = useKoma.getState().design?.docs?.[key]?.doc
      const current = storedNow ? editingDoc(storedNow, focusRef.current) : doc
      const point = toDoc(event.clientX, event.clientY)
      let target = id
      if (point && !exact) {
        const deepId = deepSelectKey(event) ? hitDesignDeepest(current, point.x, point.y)?.id : undefined
        const resolved = deepId ? { kind: 'hit' as const, id: deepId } : resolveDesignSelectHit(current, point.x, point.y, enteredContainerRef.current)
        if (resolved.kind === 'clear') {
          selectionRef.current = []
          setSelection([])
          setPropsOpen(true)
          return
        }
        if (resolved.kind === 'exit-and-hit') {
          setEnteredContainerId(resolved.enteredContainerId)
          if (!resolved.id) {
            selectionRef.current = []
            setSelection([])
            setPropsOpen(true)
            return
          }
          target = resolved.id
        } else {
          target = resolved.id
        }
      }
      const previous = selectionRef.current
      const ids = event.shiftKey ? previous.includes(target) ? previous.filter((item) => item !== target) : [...previous, target] : previous.includes(target) ? previous : [target]
      const moveIds = designRoots(current, ids)
      selectionRef.current = ids
      setSelection(ids)
      setPropsOpen(true)
      if (event.shiftKey) return
      const origins: Record<string, { x: number; y: number }> = {}
      for (const item of moveIds) {
        const row = locateDesign(current, item)
        if (row) origins[item] = { x: row.node.x, y: row.node.y }
      }
      if (locateDesign(current, target)?.node.locked) return
      if (cropEditRef.current && ids.length === 1 && ids[0] === target) {
        const paint = nodePaints(locateDesign(current, target)?.node, 'fill').find(isImageCropPaint)
        if (paint) {
          dragRef.current = { kind: 'crop', id: target, startX: event.clientX, startY: event.clientY, paint: { ...paint }, remembered: false }
          setDragCursor('move')
          return
        }
      }
      dragRef.current = {
        kind: 'move',
        ids: moveIds,
        startX: event.clientX,
        startY: event.clientY,
        origins,
        remembered: false,
        moved: false,
        broke: false,
        alt: event.altKey,
        scene: null,
      }
      setDragCursor('default')
    },
    onEdit: (id) => {
      setSelection([id])
      setEditing(id)
      setPropsOpen(true)
      labelNoted.current = false
    },
    onText: (id, text) => {
      const stored = useKoma.getState().design?.docs?.[key]?.doc
      if (!stored) return
      const view = updateDesignNode(editingDoc(stored, focusRef.current), id, (node) => {
        const runs = retargetTextRuns(node.runs, node.text ?? '', text)
        const next = { ...node, text }
        if (runs.length) next.runs = runs
        else delete next.runs
        return next
      })
      const next = projectDoc(stored, focusRef.current, view)
      if (sameDesign(next, stored)) return
      if (!labelNoted.current) {
        note(stored)
        labelNoted.current = true
      }
      updateDesign(tab.root, tab.path, next)
    },
    onTextRange: (id, start, end) => setTextRange({ id, start, end }),
    onTextBlur: () => {
      labelNoted.current = false
      setEditing(null)
      setTextRange(null)
      // Leaving a text box empty removes it, like Penpot. Its undo step already holds the state before the edit.
      const stored = useKoma.getState().design?.docs?.[key]?.doc
      const view = stored ? editingDoc(stored, focusRef.current) : null
      const node = editing && view ? findDesignNode(view, editing) : null
      if (!stored || !view || node?.kind !== 'text' || node.text?.trim()) return
      const next = projectDoc(stored, focusRef.current, deleteDesignNode(view, node.id))
      const top = pastRef.current[pastRef.current.length - 1]
      if (top && sameDesign(top.doc, next)) pastRef.current.pop()
      selectionRef.current = selectionRef.current.filter((id) => id !== node.id)
      setSelection(selectionRef.current)
      updateDesign(tab.root, tab.path, next)
    },
  }

  return (
    <div className="flex h-full min-h-0 min-w-0 bg-koma-bg text-koma-fg" onContextMenu={(event) => event.preventDefault()}>
      <input
        ref={imageInputRef}
        type="file"
        accept="image/png,image/jpeg,image/webp,image/gif,image/svg+xml"
        hidden
        multiple
        onChange={(event) => {
          const files = [...(event.target.files ?? [])]
          event.target.value = ''
          if (files.length) void importImages(files)
        }}
      />
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <EditorChrome
          path={tab.path}
          icon={<Frame size={13} className="flex-none text-koma-dim" />}
          status={status}
          canSave={canSave}
          canRevert={canRevert}
          saving={file.saving}
          canUndo={pastRef.current.length > 0}
          canRedo={futureRef.current.length > 0}
          onHistory={() => showCodingHistory(tab.root, tab.path)}
          onUndo={undo}
          onRedo={redo}
          onSave={() => saveDesign(tab.root, tab.path)}
          onRevert={revert}
          trailing={
            <button
              type="button"
              title={propsOpen ? 'Hide properties' : 'Properties'}
              aria-label={propsOpen ? 'Hide properties' : 'Properties'}
              aria-pressed={propsOpen}
              onClick={() => setPropsOpen((open) => !open)}
              className={`flex h-6 w-6 flex-none items-center justify-center rounded hover:bg-koma-hover hover:text-koma-fg ${propsOpen ? 'text-koma-fg' : 'text-koma-dim'}`}
            >
              {propsOpen ? <PanelRightClose size={13} /> : <PanelRightOpen size={13} />}
            </button>
          }
        />
        {file.error ? <div className="flex-none border-b border-koma-border px-3 py-1 text-[12px] text-koma-error">{file.error}</div> : null}
        <div className="flex min-h-0 flex-1">
        <div
          ref={canvasRef}
          className={`koma-authored-canvas relative min-h-0 min-w-0 flex-1 overflow-hidden select-none ${spaceDown || tool === 'pan' ? 'cursor-grab' : ''}`}
          onPointerDown={onCanvasPointerDown}
          onPointerMove={onCanvasHover}
          onPointerLeave={() => setDesignHover(null)}
          onContextMenu={(event) => {
            event.preventDefault()
            const point = toDoc(event.clientX, event.clientY)
            if (!point) return
            setMenu({ x: event.clientX, y: event.clientY, canvasX: point.x, canvasY: point.y })
          }}
          onDragOver={(event) => event.preventDefault()}
          onDrop={onDrop}
          style={{ backgroundColor: '#e6e8ed', ...canvasBackdrop(doc.snap, grid, view) }}
        >
          <DesignRulers panX={view.panX} panY={view.panY} zoom={view.zoom} bounds={selection.length ? designSelectionCanvasBox(doc, selection) : null} />
          {vectorEditId ? (() => {
            const vector = findDesignNode(doc, vectorEditId)?.vector
            const modes = vector ? [...new Set(vectorSel.map((i) => handleModeOf(vector, i)))] : []
            return (
              <DesignPathBar
                mode={tool === 'pen' ? 'draw' : vectorBend ? 'bend' : 'move'}
                snap={vectorSnap}
                handleMode={modes.length === 1 ? (modes[0] as HandleMode) : null}
                nodes={vectorSel.length}
                onAction={(action) => vectorAction(action)}
              />
            )
          })() : null}
          {(() => {
            // The armed tool rides on the cursor until the click that places it. The pen shows
            // only before its first point; after that its own preview takes over.
            const ghost = !spaceDown && !(tool === 'pen' && pen) ? cursorGhostFor(tool, framePreset) : null
            return ghost ? <DesignCursorGhost key={ghost.shape} canvasRef={canvasRef} {...ghost} zoom={view.zoom} /> : null
          })()}
          {/* Translate only on the outer layer; CSS `zoom` on the inner layer
              re-layouts/paints at screen resolution so text and shapes stay sharp
              (unlike `transform: scale()`, which upscales a low-res raster). */}
          <div className="pointer-events-none absolute left-0 top-0" style={{ transform: `translate(${view.panX}px, ${view.panY}px)`, transformOrigin: '0 0' }}>
            <div style={{ transform: view.zoom === layerZoom ? undefined : `scale(${view.zoom / layerZoom})`, transformOrigin: '0 0' }}>
            <div style={{ zoom: layerZoom }}>
            {flowBar ? (
              <div className="pointer-events-none absolute z-10" style={{ left: flowBar.x, top: flowBar.y, width: flowBar.w, height: flowBar.h, background: SELECTION }} />
            ) : null}
            {marks?.guides.map((guide, index) => (
              <div
                key={`guide-${guide.axis}-${index}`}
                className="pointer-events-none absolute z-10"
                style={guide.axis === 'x'
                  ? { left: guide.at, top: guide.from, width: 1 / view.zoom, height: Math.max(guide.to - guide.from, 1), background: SELECTION }
                  : { left: guide.from, top: guide.at, width: Math.max(guide.to - guide.from, 1), height: 1 / view.zoom, background: SELECTION }}
              />
            ))}
            {enteredBox ? (
              <div
                className="pointer-events-none absolute z-[5]"
                style={{
                  left: enteredBox.x,
                  top: enteredBox.y,
                  width: enteredBox.w,
                  height: enteredBox.h,
                  outline: `${1 / view.zoom}px dashed ${SELECTION}`,
                }}
              />
            ) : null}
            {marks?.measures.map((measure, index) => (
              <div
                key={`measure-${measure.axis}-${index}`}
                className="pointer-events-none absolute z-10"
                style={{
                  left: measure.x,
                  top: measure.y,
                  width: measure.axis === 'x' ? measure.length : 1 / view.zoom,
                  height: measure.axis === 'y' ? measure.length : 1 / view.zoom,
                  background: SELECTION,
                }}
              >
                <span
                  className="absolute whitespace-nowrap rounded px-1"
                  style={{
                    left: measure.axis === 'x' ? measure.length / 2 : 0,
                    top: measure.axis === 'y' ? measure.length / 2 : 0,
                    transform: 'translate(-50%, -50%)',
                    fontSize: 11 / view.zoom,
                    lineHeight: 1.2,
                    color: SELECTION,
                    background: '#e6e8ed',
                  }}
                >
                  {measure.label}
                </span>
              </div>
            ))}
            {visibleDesignScreens(doc).map((screen) => (
              <DesignNodeView
                key={screen.id}
                doc={paintDoc}
                node={screen}
                editing={editing}
                dragCursor={dragCursor}
                enteredContainerId={enteredContainerId}
                overrideTargetId={overrideTargetId}
                cropEditId={cropEdit ? selectedId : null}
                {...nodeHandlers}
              />
            ))}
            {selection.length === 1 ? (() => {
              const selectedNode = findDesignNode(doc, selection[0])
              const origin = selectedNode ? nodeOrigin(doc, selectedNode.id) : null
              const gradient = selectedNode ? firstVisiblePaint(nodePaints(selectedNode, 'fill')) : null
              if (!selectedNode || !origin || gradient?.type !== 'gradient') return null
              const angle = paintGradientAngle(gradient)
              const center = paintGradientCenter(gradient)
              const cx = origin.x + selectedNode.w * center.x
              const cy = origin.y + selectedNode.h * center.y
              const rad = (angle * Math.PI) / 180
              const hx = cx + Math.cos(rad) * selectedNode.w * 0.4
              const hy = cy + Math.sin(rad) * selectedNode.h * 0.4
              return (
                <>
                  <button type="button" aria-label="Gradient center" className="pointer-events-auto absolute z-20 h-3 w-3 -translate-x-1/2 -translate-y-1/2 rounded-full border border-white bg-koma-accent" style={{ left: cx, top: cy }} onPointerDown={(event) => {
                    event.preventDefault()
                    event.stopPropagation()
                    dragRef.current = { kind: 'move', ids: [selectedNode.id], startX: event.clientX, startY: event.clientY, origins: {}, remembered: false }
                  }} />
                  <button type="button" aria-label="Gradient angle" className="pointer-events-auto absolute z-20 h-3 w-3 -translate-x-1/2 -translate-y-1/2 rounded-full border border-white bg-white" style={{ left: hx, top: hy }} onPointerDown={(event) => {
                    event.preventDefault()
                    event.stopPropagation()
                    const start = { x: event.clientX, y: event.clientY }
                    const onMove = (move: PointerEvent) => {
                      const nextAngle = Math.atan2(canvasPoint(move.clientY - start.y) + hy - cy, canvasPoint(move.clientX - start.x) + hx - cx) * 180 / Math.PI
                      patchSelected((node) => {
                        const paints = nodePaints(node, 'fill')
                        const at = paints.findIndex((item) => item.type === 'gradient')
                        if (at < 0) return node
                        const next = paints.slice()
                        next[at] = { ...next[at], transform: [((nextAngle % 360) + 360) % 360, center.x, center.y] }
                        return { ...node, fills: next }
                      })
                    }
                    const onUp = () => {
                      window.removeEventListener('pointermove', onMove)
                      window.removeEventListener('pointerup', onUp)
                    }
                    window.addEventListener('pointermove', onMove)
                    window.addEventListener('pointerup', onUp)
                  }} />
                </>
              )
            })() : null}
          {ghost ? (
            <div
              className="pointer-events-none absolute"
              style={{
                left: ghost.x,
                top: ghost.y,
                width: Math.max(ghost.w, 1),
                height: Math.max(ghost.h, 1),
                background: ghost.kind === 'marquee' ? 'color-mix(in srgb, var(--koma-accent) 12%, transparent)' : ghost.shape === 'frame' ? '#ffffff' : ghost.shape === 'rect' || ghost.shape === 'ellipse' ? SHAPE_FILL : 'transparent',
                border: `${1 / view.zoom}px solid ${SELECTION}`,
                borderRadius: ghost.kind === 'shape' && ghost.shape === 'ellipse' ? '50%' : undefined,
                transform: ghost.kind === 'shape' && ghost.rotation ? `rotate(${ghost.rotation}deg)` : undefined,
              }}
            >
              {ghost.kind === 'shape' ? (
                <span className="absolute left-0 whitespace-nowrap" style={{ top: '100%', marginTop: 6 / view.zoom, fontSize: 11 / view.zoom, color: SELECTION }}>
                  {ghost.shape === 'line' ? Math.round(ghost.w) : `${Math.round(ghost.w)} × ${Math.round(ghost.h)}`}
                </span>
              ) : null}
            </div>
          ) : null}
          {pen ? <PenOverlay draft={pen} hover={penHandle == null ? penHover : null} zoom={view.zoom} /> : null}
          {vectorEditId ? (() => {
            const edited = findDesignNode(doc, vectorEditId)
            const origin = nodeOrigin(doc, vectorEditId)
            const vector = edited?.vector
            if (!edited || !origin || !vector?.vertices.length) return null
            const unit = 7 / view.zoom
            const stroke = 1 / view.zoom
            const picked = new Set(vectorSel)
            // Handles show for the selected nodes, or for every node while none is selected.
            const showHandles = (vertex: number) => !picked.size || picked.has(vertex)
            return (
              <>
                <svg className="absolute overflow-visible" width={1} height={1} style={{ left: origin.x, top: origin.y }}>
                  {vector.segments.map((_, index) => (
                    <path
                      key={`${vectorEditId}-seg-${index}`}
                      d={segmentSvgPath(vector, index)}
                      fill="none"
                      stroke="transparent"
                      strokeWidth={10 / view.zoom}
                      className="cursor-copy hover:stroke-[color-mix(in_srgb,var(--color-koma-accent)_45%,transparent)]"
                      style={{ pointerEvents: 'stroke' }}
                      onPointerDown={(event) => vectorDownRef.current.segment(event, index)}
                    >
                      <title>Click to add a node, drag to move, ⌘-drag to bend, Alt-click to delete</title>
                    </path>
                  ))}
                  {vectorMarquee ? (
                    <rect
                      x={vectorMarquee.x - origin.x}
                      y={vectorMarquee.y - origin.y}
                      width={vectorMarquee.w}
                      height={vectorMarquee.h}
                      fill="color-mix(in srgb, var(--koma-accent) 12%, transparent)"
                      stroke={SELECTION}
                      strokeWidth={stroke}
                    />
                  ) : null}
                </svg>
                {vector.segments.flatMap((segment, index) => {
                  const start = vector.vertices[segment.start]
                  const end = vector.vertices[segment.end]
                  if (!start || !end) return []
                  const handles = [
                    { end: 'start' as const, vertex: segment.start, x: start.x + segment.tangentStart.x, y: start.y + segment.tangentStart.y, from: start, zero: !segment.tangentStart.x && !segment.tangentStart.y },
                    { end: 'end' as const, vertex: segment.end, x: end.x + segment.tangentEnd.x, y: end.y + segment.tangentEnd.y, from: end, zero: !segment.tangentEnd.x && !segment.tangentEnd.y },
                  ]
                  return handles.flatMap((handle) => {
                    if (handle.zero || !showHandles(handle.vertex)) return []
                    return [
                      <div key={`${vectorEditId}-hline-${index}-${handle.end}`} className="pointer-events-none absolute z-10" style={{ left: origin.x + handle.from.x, top: origin.y + handle.from.y, width: Math.hypot(handle.x - handle.from.x, handle.y - handle.from.y), height: stroke, background: SELECTION, transformOrigin: '0 50%', transform: `rotate(${Math.atan2(handle.y - handle.from.y, handle.x - handle.from.x)}rad)` }} />,
                      <button
                        key={`${vectorEditId}-h-${index}-${handle.end}`}
                        type="button"
                        aria-label={`${handle.end} handle`}
                        title="Drag: ⌘ mirrors, Alt frees, Shift snaps to 45°"
                        className="pointer-events-auto absolute z-20 rounded-full border-0 p-0"
                        style={{ left: origin.x + handle.x, top: origin.y + handle.y, width: unit, height: unit, background: '#ffffff', border: `${stroke}px solid ${SELECTION}`, transform: 'translate(-50%, -50%)' }}
                        onPointerDown={(event) => {
                          event.preventDefault()
                          event.stopPropagation()
                          dragRef.current = {
                            kind: 'handle',
                            id: vectorEditId,
                            segment: index,
                            end: handle.end,
                            startX: event.clientX,
                            startY: event.clientY,
                            remembered: false,
                            mode: handleModeOf(vector, handle.vertex),
                          }
                        }}
                      />,
                    ]
                  })
                })}
                {vector.vertices.map((point, index) => (
                  <button
                    key={`${vectorEditId}-${index}`}
                    type="button"
                    aria-label={`Node ${index + 1}`}
                    aria-pressed={picked.has(index)}
                    title="Shift adds, Alt-drag duplicates, ⌘-click toggles curve"
                    className="pointer-events-auto absolute z-20 border-0 p-0"
                    style={{
                      left: origin.x + point.x,
                      top: origin.y + point.y,
                      width: unit,
                      height: unit,
                      background: picked.has(index) ? SELECTION : '#ffffff',
                      border: `${stroke}px solid ${SELECTION}`,
                      transform: 'translate(-50%, -50%)',
                      cursor: 'move',
                    }}
                    onPointerDown={(event) => vectorDownRef.current.vertex(event, index)}
                  />
                ))}
              </>
            )
          })() : null}
            </div>
            </div>
          </div>
          {/* Root screen names stay one size on screen, like Penpot board titles. Click one to select and drag that screen. */}
          <div className="pointer-events-none absolute inset-0">
            {visibleDesignScreens(doc).filter((screen) => screen.kind === 'frame').map((screen) => {
              const box = designCanvasBox(doc, screen.id)
              if (!box) return null
              return (
                <div
                  key={screen.id}
                  className={`absolute truncate text-[11px] leading-4 ${selection.includes(screen.id) ? 'text-koma-accent' : 'text-koma-dim'} ${screen.locked || tool !== 'select' ? '' : 'pointer-events-auto cursor-default'}`}
                  style={{ left: view.panX + box.x * view.zoom, top: view.panY + box.y * view.zoom - 18, maxWidth: Math.max(box.w * view.zoom, 24) }}
                  onPointerEnter={() => setDesignHover(screen.id)}
                  onPointerLeave={() => setDesignHover(null)}
                  onPointerDown={(event) => {
                    if (event.button !== 0) return
                    event.stopPropagation()
                    nodeHandlers.onSelect(screen.id, event, true)
                  }}
                >
                  {designLayerName(screen)}
                </div>
              )
            })}
          </div>
          <DesignSelectionChrome
            doc={doc}
            panX={view.panX}
            panY={view.panY}
            zoom={view.zoom}
            selectedIds={vectorEditId ? [] : selection}
            dragCursor={dragCursor}
            enteredContainerId={enteredContainerId}
            cropEditId={cropEdit ? selectedId : null}
            hoverId={tool === 'select' && !vectorEditId && !editing ? hoverId : null}
            onCorner={(id, corner, event) => {
              event.preventDefault()
              event.stopPropagation()
              setSelection([id])
              const storedNow = useKoma.getState().design?.docs?.[key]?.doc
              const locatedNow = locateDesign(storedNow ? editingDoc(storedNow, focusRef.current) : doc, id)
              if (!locatedNow) return
              const raw = locatedNow.node.radius
              const radius = typeof raw === 'number' ? raw : Number(resolveRef(doc, typeof raw === 'string' ? raw : '')) || 0
              dragRef.current = {
                kind: 'radius',
                id,
                corner,
                startX: event.clientX,
                startY: event.clientY,
                radius,
                node: { ...locatedNow.node },
                remembered: false,
              }
            }}
            onRotate={(id, event) => {
              event.preventDefault()
              event.stopPropagation()
              setSelection([id])
              const storedNow = useKoma.getState().design?.docs?.[key]?.doc
              const docNow = storedNow ? editingDoc(storedNow, focusRef.current) : doc
              const located = locateDesign(docNow, id)
              if (!located || located.node.locked) return
              const center = nodeCenter(docNow, id)
              const grab = toDoc(event.clientX, event.clientY)
              if (!center || !grab) return
              dragRef.current = {
                kind: 'rotate',
                id,
                cx: center.x,
                cy: center.y,
                startAngle: rotationFromCenter(center.x, center.y, grab.x, grab.y),
                sign: designTurnSign(docNow, id),
                rotation: located.node.rotation ?? 0,
                node: { ...located.node },
                remembered: false,
              }
              setDragCursor('grabbing')
            }}
            onResize={(id, handle, event) => {
              event.preventDefault()
              event.stopPropagation()
              setSelection([id])
              const storedNow = useKoma.getState().design?.docs?.[key]?.doc
              const located = locateDesign(storedNow ? editingDoc(storedNow, focusRef.current) : doc, id)
              if (!located || located.node.locked) return
              const node = { ...located.node }
              if (handle.includes('w') || handle.includes('e')) {
                if (node.kind === 'instance') node.wMode = 'fixed'
                else delete node.wMode
              }
              if (handle.includes('n') || handle.includes('s')) {
                if (node.kind === 'instance') node.hMode = 'fixed'
                else delete node.hMode
              }
              // A hand-sized axis stops hugging: width-hug text keeps wrapping to height, a set height is fixed.
              if (node.textHug && (handle.includes('n') || handle.includes('s'))) delete node.textHug
              else if (node.textHug === 'width') node.textHug = 'height'
              dragRef.current = {
                kind: 'resize',
                id,
                handle,
                startX: event.clientX,
                startY: event.clientY,
                node,
                remembered: false,
              }
              setDragCursor(HANDLES.find((item) => item.id === handle)?.cursor ?? 'grabbing')
            }}
          />
          {doc.screens.length === 0 && !file.loading ? (
            <div className="pointer-events-none absolute inset-0 flex items-center justify-center text-[12px] text-koma-fg opacity-35">
              Drag a frame onto the canvas
            </div>
          ) : null}
        </div>
        </div>
        <div className="flex h-8 flex-none items-center gap-1 border-t border-koma-border bg-koma-panel px-2">
          <ToolButton label="Move (V)" selected={tool === 'select'} onClick={() => chooseTool('select')}>
            <MousePointer2 size={15} strokeWidth={2.25} />
          </ToolButton>
          <FrameToolFlyout
            tool={tool}
            onTool={chooseTool}
            onPreset={(preset) => {
              const stored = useKoma.getState().design?.docs?.[key]?.doc
              if (!stored) return
              const viewDocNow = editingDoc(stored, focusRef.current)
              const id = selectionRef.current[0]
              const selected = id ? locateDesign(viewDocNow, id)?.node : null
              if (selected?.kind === 'frame') {
                commit(updateDesignNode(viewDocNow, id, (node) => ({ ...node, w: preset.width, h: preset.height, wMode: 'fixed', hMode: 'fixed' })))
                return
              }
              // Arm the frame tool with this size; the cursor ghost shows where the click will put it.
              setFramePreset(preset)
              chooseTool('frame')
            }}
          />
          <ShapeToolFlyout tool={tool} onTool={chooseTool} />
          <ToolButton label="Pen (P)" selected={tool === 'pen'} onClick={() => chooseTool('pen')}>
            <PenTool size={15} strokeWidth={2.25} />
          </ToolButton>
          <ToolButton label="Text (T)" selected={tool === 'text'} onClick={() => chooseTool('text')}>
            <Type size={15} strokeWidth={2.25} />
          </ToolButton>
          <ToolButton label="Hand (H)" selected={tool === 'pan'} onClick={() => setTool('pan')}>
            <Hand size={15} strokeWidth={2.25} />
          </ToolButton>
          {focusedComponent || chain.length || enteredContainerId ? (
            <div className="flex min-w-0 items-center gap-0.5 overflow-hidden">
              {focusedComponent ? (
                <button
                  type="button"
                  onClick={() => {
                    setFocusId(null)
                    setSelection([])
                    setEnteredContainerId(null)
                  }}
                  className="h-6 flex-none rounded px-1.5 text-[12px] text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
                >
                  Screens
                </button>
              ) : null}
              {chain.map((node, index) => (
                <span key={node.id} className="flex min-w-0 items-center">
                  {focusedComponent || index > 0 ? <span className="px-0.5 text-[12px] text-koma-dim">/</span> : null}
                  <button
                    type="button"
                    onClick={() => {
                      setSelection([node.id])
                      if (canEnterDesignContainer(node)) setEnteredContainerId(node.id)
                      else setEnteredContainerId(designEnterScopeForLayerSelect(doc, node.id))
                    }}
                    className={`h-6 max-w-24 truncate rounded px-1.5 text-[12px] hover:bg-koma-hover ${enteredContainerId === node.id ? 'bg-koma-accent/15 text-koma-accent' : 'text-koma-fg'}`}
                  >
                    {designLayerName(node)}
                  </button>
                </span>
              ))}
              <button
                type="button"
                aria-label="Clear"
                title="Clear"
                onClick={stepOut}
                className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
              >
                <X size={13} />
              </button>
            </div>
          ) : null}
          {chatQuery ? (
            <button
              type="button"
              onClick={sendChat}
              className="flex h-6 items-center gap-1 rounded px-2 text-[12px] text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
            >
              <Plus size={13} strokeWidth={2.25} />
              Chat
            </button>
          ) : null}
          <div className="ml-auto flex items-center gap-1">
            <ToolButton label="Zoom out" selected={false} onClick={() => {
              const rect = canvasRef.current?.getBoundingClientRect()
              if (!rect) return
              zoomAt(rect.left + rect.width / 2, rect.top + rect.height / 2, view.zoom / 1.1)
            }}>
              <Minus size={15} strokeWidth={2.25} />
            </ToolButton>
            <span className="w-14 text-center text-[11px] text-koma-dim">{Math.round(view.zoom * 100)}%</span>
            <button type="button" title="Zoom to fit (⇧1)" className="h-6 rounded px-1.5 text-[11px] text-koma-dim hover:bg-koma-hover hover:text-koma-fg" onClick={() => zoomTo(null)}>Fit</button>
            <button type="button" title="Zoom to selection (⇧2)" className="h-6 rounded px-1.5 text-[11px] text-koma-dim hover:bg-koma-hover hover:text-koma-fg" onClick={() => zoomTo(selection)}>Selection</button>
            <ToolButton label="Zoom in" selected={false} onClick={() => {
              const rect = canvasRef.current?.getBoundingClientRect()
              if (!rect) return
              zoomAt(rect.left + rect.width / 2, rect.top + rect.height / 2, view.zoom * 1.1)
            }}>
              <Plus size={15} strokeWidth={2.25} />
            </ToolButton>
            {doc.modes.map((mode) => (
              <button
                key={mode}
                type="button"
                aria-pressed={doc.mode === mode}
                onClick={() => commit(setDesignMode(doc, mode))}
                className={`h-6 rounded px-1.5 text-[11px] capitalize ${doc.mode === mode ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover'}`}
              >
                {mode}
              </button>
            ))}
            <button
              type="button"
              aria-pressed={doc.snap}
              onClick={() => commit({ ...doc, snap: !doc.snap })}
              className={`ml-1 h-6 rounded px-2 text-[12px] ${doc.snap ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover'}`}
            >
              Snap
            </button>
          </div>
        </div>
      </div>
      {propsOpen ? (
        <aside className="flex w-[296px] flex-none flex-col overflow-y-auto border-l border-koma-border bg-koma-panel">
          <div className="flex h-[22px] flex-none items-center gap-1 bg-koma-head px-2 text-[11px] font-semibold uppercase tracking-wide text-koma-fg opacity-75">
            <span className="min-w-0 flex-1 truncate">{multi ? `${selectedNodes.length} selected` : selected ? designLayerName(selected) : 'Styles'}</span>
            {selection.length || focusId ? (
              <button
                type="button"
                aria-label="Clear"
                title="Clear"
                onClick={stepOut}
                className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
              >
                <X size={13} />
              </button>
            ) : null}
          </div>
          {selectedNodes.length ? (
            <NodeSettings
              doc={doc}
              nodes={selectedNodes}
              hasParent={everyParent}
              parentLayout={parentNode?.layout ?? null}
              sizeModes={sizeModes}
              textRange={textRange && selectedId === textRange.id ? textRange : null}
              componentName={!multi && selectedVariant ? focusedComponent?.name ?? null : null}
              variantProps={!multi && selectedVariant ? selectedVariant.props : null}
              axes={!multi && selected?.kind === 'instance' ? instanceAxes(storedDoc, selected) : null}
              onMakeComponent={!multi && !focusId && selected?.kind === 'frame' ? () => commandsRef.current?.component() : undefined}
              onAddVariant={!multi && focusId ? () => {
                const stored = useKoma.getState().design?.docs?.[key]?.doc
                const component = stored?.components.find((item) => item.id === focusId)
                if (!stored || !component) return
                const next = addComponentVariant(stored, focusId, nextVariantProps(component.variants), () => mintId('n'))
                if (next) commitStored(next)
              } : undefined}
              onVariantProps={!multi && focusId && selectedVariant ? (props) => {
                const stored = useKoma.getState().design?.docs?.[key]?.doc
                if (!stored || !selectedId) return
                const next = setVariantProps(stored, focusId, selectedId, props)
                if (next) commitStored(next, true)
              } : undefined}
              onRenameComponent={!multi && focusId ? (name) => {
                const stored = useKoma.getState().design?.docs?.[key]?.doc
                if (!stored) return
                const next = renameComponent(stored, focusId, name)
                if (next) commitStored(next, true)
              } : undefined}
              onInstanceVariant={!multi && selected?.kind === 'instance' ? (props) => {
                const stored = useKoma.getState().design?.docs?.[key]?.doc
                if (!stored) return
                const next = setInstanceVariant(stored, selected.id, props)
                if (next) commitStored(next)
              } : undefined}
              onAddToChat={!multi && chatQuery ? sendChat : undefined}
              onBoolean={(op) => commandsRef.current?.boolean(op)}
              onAlign={selectedNodes.length && (multi || everyParent) ? (axis, edge) => {
                const stored = useKoma.getState().design?.docs?.[key]?.doc
                if (!stored) return
                commit(alignDesignNodes(editingDoc(stored, focusRef.current), selection, axis, edge))
              } : undefined}
              onResetInstance={!multi && selected?.kind === 'instance' && (selected.text || selected.fill || selected.overrides?.length) ? () => {
                const stored = useKoma.getState().design?.docs?.[key]?.doc
                if (!stored) return
                commitStored(resetInstanceOverrides(stored, selected.id))
              } : undefined}
              onGoToMain={!multi && selected?.kind === 'instance' && selected.component ? () => {
                const componentId = selected.component ?? null
                const main = storedDoc.components.find((item) => item.id === componentId)?.variants[0]?.node
                focusRef.current = componentId
                setFocusId(componentId)
                setSelection(main ? [main.id] : [])
                if (main) window.requestAnimationFrame(() => zoomTo([main.id]))
              } : undefined}
              onSwapInstance={!multi && selected?.kind === 'instance' ? (componentId) => {
                patchSelected((node) => ({ ...node, component: componentId }))
              } : undefined}
              onDetachInstance={!multi && selected?.kind === 'instance' ? () => {
                const stored = useKoma.getState().design?.docs?.[key]?.doc
                if (!stored) return
                const next = detachInstance(editingDoc(stored, focusRef.current), selected.id, () => mintId('n'))
                if (next) commit(next)
              } : undefined}
              onPatch={(fn) => patchSelected(fn)}
              onType={(fn) => debouncedTypePatch(fn)}
              onTypeFocus={() => {
                labelNoted.current = false
              }}
              onTypeBlur={() => {
                flushDebouncedTypePatch()
                labelNoted.current = false
              }}
              onOverrideTarget={(childId) => {
                if (childId && selected) setSelection([selected.id])
                setOverrideTargetId(childId)
              }}
              openFillToken={openFillToken}
              onInspectorPaint={setInspectorPaint}
              onPickImage={() => imageInputRef.current?.click()}
              onStoreImage={(hash, bytes, mime) => {
                const path = designAssetPath(hash, mime)
                cacheDesignImage(hash, bytes, mime)
                writeDesignAsset(useKoma.getState().req, tab.root, path, bytes)
                const stored = useKoma.getState().design?.docs?.[key]?.doc
                if (!stored || stored.images?.[hash]) return
                updateDesign(tab.root, tab.path, putDesignImage(stored, hash, mime, path))
              }}
            />
          ) : (
            <TokenEditor root={tab.root} path={tab.path} doc={storedDoc} onCommit={(next) => commitStored(next)} />
          )}
        </aside>
      ) : null}
      {palette ? (
        <div className="absolute inset-0 z-50 flex items-start justify-center bg-black/30 pt-16" onClick={() => setPalette(false)}>
          <div className="w-80 rounded border border-koma-border bg-koma-panel p-2 shadow-lg" onClick={(event) => event.stopPropagation()}>
            <p className="px-1 pb-1 text-[11px] text-koma-dim">Commands</p>
            {[
              { id: 'select', label: 'Move tool', run: () => chooseTool('select') },
              { id: 'pen', label: 'Pen tool', run: () => chooseTool('pen') },
              { id: 'union', label: 'Boolean union', run: () => commandsRef.current?.boolean('union') },
              { id: 'outline', label: 'Outline stroke', run: () => commandsRef.current?.outline() },
              { id: 'detach', label: 'Detach instance', run: () => commandsRef.current?.detach() },
              { id: 'image', label: 'Import image', run: () => imageInputRef.current?.click() },
              { id: 'component', label: 'Create component', run: () => commandsRef.current?.component() },
            ].map((item) => (
              <button
                key={item.id}
                type="button"
                className="flex h-7 w-full items-center rounded px-2 text-left text-[12px] text-koma-fg hover:bg-koma-hover"
                onClick={() => {
                  item.run()
                  setPalette(false)
                }}
              >
                {item.label}
              </button>
            ))}
          </div>
        </div>
      ) : null}
      {menu ? (
        <DesignMenu
          x={menu.x}
          y={menu.y}
          items={designMenuItems(doc, selection, menu.canvasX, menu.canvasY, !!getCopiedShape()?.nodes.length, getCopiedStyle() != null, !focusId && selection.length === 1 && findDesignNode(doc, selection[0])?.kind === 'frame', !multi && chatQuery != null)}
          onClose={() => setMenu(null)}
          onPick={(id) => {
            const here = menu
            setMenu(null)
            const commands = commandsRef.current
            if (!commands) return
            if (id === 'chat') sendChat()
            else if (id === 'copy') commands.copy()
            else if (id === 'paste') commands.paste()
            else if (id === 'paste-here') commands.paste({ x: here.canvasX, y: here.canvasY })
            else if (id === 'duplicate') commands.duplicate()
            else if (id === 'copy-style') commands.copyStyle()
            else if (id === 'paste-style') commands.pasteStyle()
            else if (id === 'front' || id === 'forward' || id === 'backward' || id === 'back') commands.order(id)
            else if (id === 'group') commands.wrap('group')
            else if (id === 'ungroup') commands.unwrap()
            else if (id === 'frame-selection') commands.wrap('frame')
            else if (id === 'auto') commands.auto()
            else if (id === 'component') commands.component()
            else if (id === 'detach') commands.detach()
            else if (id === 'union' || id === 'subtract' || id === 'intersect' || id === 'exclude' || id === 'flatten') commands.boolean(id)
            else if (id === 'outline') commands.outline()
            else if (id === 'hide') commands.hide()
            else if (id === 'lock') commands.lock()
            else if (id === 'flip-x') commands.flip('x')
            else if (id === 'flip-y') commands.flip('y')
            else if (id === 'delete') commands.remove()
            else if (id.startsWith('select:')) commands.select(id.slice('select:'.length))
          }}
        />
      ) : null}
    </div>
  )
}
