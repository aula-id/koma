import { useEffect, useRef, useState } from 'react'
import {
  ChevronRight,
  AlignCenterHorizontal,
  AlignCenterVertical,
  AlignEndHorizontal,
  AlignEndVertical,
  AlignHorizontalSpaceBetween,
  AlignStartHorizontal,
  AlignStartVertical,
  AlignVerticalSpaceBetween,
  Component,
  Crop,
  Eye,
  EyeOff,
  FlipHorizontal2,
  FlipVertical2,
  Layers,
  Lock,
  MessageSquare,
  Move,
  Plus,
  RotateCcw,
  RotateCw,
  Square,
  Unlock,
  TextAlignCenter,
  TextAlignEnd,
  TextAlignStart,
  WrapText,
  X,
} from 'lucide-react'
import { KomaSelect } from '../KomaSelect'
import { DesignModeContext } from './DesignTokenMenu'
import { InspectorBack, InspectorPageContext, InspectorPageView, type InspectorPage } from './DesignPaintPopup'
import {
  designChatText,
  designChatTitle,
  designLayerName,
  designQueryNode,
  mergeDesignOverride,
  applyFramePreset,
  applyTextRun,
  bindNodeField,
  designImageSize,
  ensureImageCrop,
  nodeHasPaint,
  scaleImageCrops,
  nodePaints,
  setNodePaints,
  reshapeDesignNode,
  retuneDesignShape,
  shapeKindOf,
  FRAME_PRESETS,
  nodeChrome,
  resolveRef,
  setNodeSolid,
  sharedValue,
  solidPaint,
  textStyle,
  type DesignAlign,
  type DesignAlignAxis,
  type DesignAlignEdge,
  type DesignDoc,
  type DesignLayout,
  type DesignLayoutGrid,
  type DesignNode,
  type DesignPaint,
  type DesignReshapeKind,
  type DesignTextRun,
  type DesignToken,
} from '../../lib/design'
import { SHAPE_FILL } from './tabShared'
import { InstanceOverrides } from './DesignPropertyFields'
import {
  AlignButton,
  Choices,
  ColorRow,
  ConstraintWidget,
  FieldGroup,
  GeomField,
  KindMark,
  FillEditor,
  LabeledControl,
  PaintRow,
  RadiusField,
  Section,
  SizeMode,
  containerPaint,
} from './DesignPropertyFields'

export function NodeSettings({
  doc,
  nodes,
  hasParent,
  sizeModes,
  componentName,
  variantProps,
  axes,
  onMakeComponent,
  onAddVariant,
  onVariantProps,
  onRenameComponent,
  onInstanceVariant,
  onResetInstance,
  onGoToMain,
  onSwapInstance,
  onDetachInstance,
  onAddToChat,
  onAlign,
  onPatch,
  onType,
  onTypeFocus,
  onTypeBlur,
  onOverrideTarget,
  onPickImage,
  onStoreImage,
  textRange,
  parentLayout,
  openFillToken,
  onInspectorPaint,
}: {
  doc: DesignDoc
  nodes: DesignNode[]
  hasParent: boolean
  sizeModes: boolean
  componentName?: string | null
  variantProps?: Record<string, string> | null
  axes?: { name: string; values: string[]; current: string }[] | null
  onMakeComponent?: () => void
  onAddVariant?: () => void
  onVariantProps?: (props: Record<string, string>) => void
  onRenameComponent?: (name: string) => void
  onInstanceVariant?: (props: Record<string, string>) => void
  onResetInstance?: () => void
  onGoToMain?: () => void
  onSwapInstance?: (componentId: string) => void
  onDetachInstance?: () => void
  onAddToChat?: () => void
  onAlign?: (axis: DesignAlignAxis, edge: DesignAlignEdge) => void
  onPatch: (fn: (node: DesignNode) => DesignNode) => void
  onType: (fn: (node: DesignNode) => DesignNode) => void
  onTypeFocus: () => void
  onTypeBlur: () => void
  onOverrideTarget?: (childId: string | null) => void
  onPickImage?: () => void
  onStoreImage?: (hash: string, bytes: Uint8Array, mime: string) => void
  textRange?: { start: number; end: number } | null
  parentLayout?: DesignLayout | null
  openFillToken?: number
  onInspectorPaint?: (paint: DesignPaint | null) => void
}) {
  const [propName, setPropName] = useState('variant')
  const [propValue, setPropValue] = useState('')
  const node = nodes[0]
  const multi = nodes.length > 1
  const allFrames = nodes.every((item) => item.kind === 'frame')
  const allText = nodes.every((item) => item.kind === 'text')
  const showRadius = nodes.every((item) => item.kind === 'rect' || item.kind === 'frame')
  const chrome = nodeChrome(node)
  const style = textStyle(node)
  const colorTokens = doc.tokens.filter((token) => token.kind === 'color')
  const radiusTokens = doc.tokens.filter((token) => token.kind === 'radius')
  const spaceTokens = doc.tokens.filter((token) => token.kind === 'space')
  const typeTokens = doc.tokens.filter((token) => token.kind === 'type')
  const bindField = (field: string, token: string | null) => onPatch((current) => bindNodeField(current, field, token))
  const numberOf = (pick: (item: DesignNode) => number) => {
    const value = sharedValue(nodes.map(pick))
    return { value: value ?? pick(node), mixed: value == null }
  }
  const textOf = <T extends string>(pick: (item: DesignNode) => T) => {
    const value = sharedValue(nodes.map(pick))
    return { value: value ?? pick(node), mixed: value == null }
  }
  const setField = (patch: Partial<DesignNode>, clear: (keyof DesignNode)[] = []) => {
    onPatch((current) => {
      let next: DesignNode = { ...current, ...patch }
      for (const key of clear) delete next[key]
      if ((patch.w != null && patch.w !== current.w) || (patch.h != null && patch.h !== current.h)) {
        next = scaleImageCrops(current, next)
      }
      return next
    })
  }
  const selectedText = !multi && node.kind === 'text' && textRange && textRange.end > textRange.start
    ? { start: textRange.start, end: textRange.end }
    : null
  const applyType = (patch: Partial<DesignNode>, clear: (keyof DesignNode)[] = []) => {
    if (selectedText) {
      const runPatch: Omit<Partial<DesignTextRun>, 'start' | 'end'> = {}
      if (patch.fontSize != null) runPatch.fontSize = patch.fontSize
      if (patch.weight != null) runPatch.weight = patch.weight
      if (patch.italic != null) runPatch.italic = patch.italic
      if (patch.underline != null) runPatch.underline = patch.underline
      if (patch.strike != null) runPatch.strike = patch.strike
      if (patch.fontFamily != null) runPatch.fontFamily = patch.fontFamily
      if (typeof patch.color === 'string') runPatch.color = patch.color
      if (Object.keys(runPatch).length) {
        const runs = applyTextRun(node.runs, selectedText.start, selectedText.end, runPatch, (node.text ?? '').length)
        setField(runs.length ? { runs } : {}, runs.length ? [] : ['runs'])
        return
      }
    }
    setField(patch, clear)
  }
  const paintChange = (field: 'fill' | 'stroke', next: string | null) => {
    onPatch((current) => {
      const fallback = field === 'fill' ? (current.kind === 'frame' ? '#ffffff' : SHAPE_FILL) : '#1c1c1c'
      const themedFill = field === 'fill' && (current.kind === 'rect' || current.kind === 'ellipse' || (current.kind === 'vector' && !!current.vector?.regions.length))
      if (next === 'none') return setNodeSolid(current, field, 'none')
      if (next == null) return themedFill ? setNodeSolid(current, field, null) : setNodeSolid(current, field, fallback)
      return setNodeSolid(current, field, next)
    })
  }
  const xField = numberOf((item) => item.x)
  const yField = numberOf((item) => item.y)
  const wField = numberOf((item) => item.w)
  const hField = numberOf((item) => item.h)
  const rotationField = numberOf((item) => item.rotation ?? 0)
  const opacityField = numberOf((item) => Math.round((item.opacity ?? 1) * 100))
  const radiusField = textOf((item) => (typeof item.radius === 'number' ? (item.radius > 0 ? String(item.radius) : '0') : item.radius || '0'))
  const fillField = textOf((item) => containerPaint(item, 'fill', nodeChrome(item).fill))
  const strokeField = textOf((item) => containerPaint(item, 'stroke', nodeChrome(item).stroke))
  const layoutField = textOf((item) => item.layout ?? 'free')
  const gapField = numberOf((item) => item.gap ?? 0)
  const padField = numberOf((item) => item.pad ?? 0)
  const justifyField = textOf((item) => item.justify ?? 'start')
  const alignField = textOf((item) => item.align ?? 'start')
  const wModeField = textOf((item) => item.wMode ?? 'fixed')
  const hModeField = textOf((item) => item.hMode ?? 'fixed')
  const weightField = textOf((item) => textStyle(item).weight)
  const textAlignField = textOf((item) => textStyle(item).align)
  const fontSizeField = numberOf((item) => textStyle(item).fontSize)
  const lineField = numberOf((item) => textStyle(item).lineHeight)
  const trackingField = numberOf((item) => textStyle(item).letterSpacing)
  const colorField = textOf((item) => textStyle(item).color || 'none')
  const absoluteField = textOf((item) => (item.absolute ? 'on' : 'off'))
  const flows = nodes.every((item) => item.layout === 'row' || item.layout === 'column')
  const flipXOn = nodes.every((item) => !!item.flipX)
  const flipYOn = nodes.every((item) => !!item.flipY)
  const columnLayout = !layoutField.mixed && layoutField.value === 'column'
  const setJustify = (justify: DesignAlign) => {
    setField(justify === 'start' ? {} : { justify }, justify === 'start' ? ['justify'] : [])
  }
  const [radiusSplit, setRadiusSplit] = useState(!!(node.radiusTL != null || node.radiusTR != null || node.radiusBR != null || node.radiusBL != null))
  const [fontQuery, setFontQuery] = useState('')
  const documentFonts = [...new Set(doc.screens.flatMap(function walk(item: DesignNode): string[] {
    return [item.fontFamily ?? '', ...(item.children ?? []).flatMap(walk)]
  }).filter(Boolean))]
  const systemFonts = ['Inter', 'system-ui', 'serif', 'monospace', 'Georgia', 'Times New Roman', 'Arial', 'Helvetica', 'Courier New']
  const fontChoices = [...new Set([...documentFonts, ...systemFonts])].filter((name) => !fontQuery || name.toLowerCase().includes(fontQuery.toLowerCase()))
  const setCross = (align: 'start' | 'center' | 'end' | 'stretch') => {
    setField(align === 'start' ? {} : { align }, align === 'start' ? ['align'] : [])
  }
  const setSide = (key: 'padTop' | 'padRight' | 'padBottom' | 'padLeft', value: number) => {
    const pad = node.pad ?? 0
    if (!Number.isFinite(value) || value < 0 || value === pad) setField({}, [key])
    else setField({ [key]: value })
  }
  const setCorner = (key: 'radiusTL' | 'radiusTR' | 'radiusBR' | 'radiusBL', value: number) => {
    const uniform = typeof node.radius === 'number' ? node.radius : 0
    if (!Number.isFinite(value) || value < 0 || value === uniform) setField({}, [key])
    else setField({ [key]: value })
  }
  const padTopField = numberOf((item) => item.padTop ?? item.pad ?? 0)
  const padRightField = numberOf((item) => item.padRight ?? item.pad ?? 0)
  const padBottomField = numberOf((item) => item.padBottom ?? item.pad ?? 0)
  const padLeftField = numberOf((item) => item.padLeft ?? item.pad ?? 0)
  const minWField = numberOf((item) => item.minW ?? 0)
  const maxWField = numberOf((item) => item.maxW ?? 0)
  const minHField = numberOf((item) => item.minH ?? 0)
  const maxHField = numberOf((item) => item.maxH ?? 0)
  const cornerField = (pick: (item: DesignNode) => number) => numberOf(pick)
  const resolvedCorner = (value: number | string | undefined) => (typeof value === 'number' ? value : Number(resolveRef(doc, value ?? '')) || 0)
  const [inspectorPage, setInspectorPage] = useState<InspectorPage | null>(null)
  const onInspectorPaintRef = useRef(onInspectorPaint)
  onInspectorPaintRef.current = onInspectorPaint
  useEffect(() => {
    setInspectorPage(null)
  }, [node.id, multi])
  useEffect(() => {
    onInspectorPaintRef.current?.(inspectorPage?.kind === 'paint' ? inspectorPage.paint : null)
  }, [inspectorPage])
  useEffect(() => () => onInspectorPaintRef.current?.(null), [])
  const openInspectorPage = (page: InspectorPage) => {
    if (page.kind === 'paint') {
      const apply = page.onChange
      setInspectorPage({
        ...page,
        onChange: (paint) => {
          setInspectorPage((current) => (current?.kind === 'paint' ? { ...current, paint } : current))
          apply(paint)
        },
      })
      return
    }
    setInspectorPage(page)
  }
  const closeInspector = () => setInspectorPage(null)
  useEffect(() => {
    if (!openFillToken || multi) return
    const paints = nodePaints(node, 'fill')
    const paint = paints[0] ?? solidPaint(SHAPE_FILL)
    const boxed = paint.type === 'image' && paint.scale === 'crop'
      ? ensureImageCrop(paint, { w: node.w, h: node.h }, designImageSize(doc, paint.hash))
      : paint
    openInspectorPage({
      kind: 'paint',
      title: 'Fill',
      paint: boxed,
      fallback: SHAPE_FILL,
      tokens: colorTokens,
      allowImage: true,
      allowGradient: true,
      box: { w: node.w, h: node.h },
      natural: designImageSize(doc, paint.hash),
      onChange: (next) => onPatch((current) => {
        const currentPaints = nodePaints(current, 'fill')
        const painted = next.type === 'image' && next.scale === 'crop'
          ? ensureImageCrop(next, { w: current.w, h: current.h }, designImageSize(doc, next.hash))
          : next
        return setNodePaints(current, 'fill', currentPaints.length ? currentPaints.map((item, at) => (at === 0 ? painted : item)) : [painted])
      }),
      onStoreImage,
    })
  }, [openFillToken])
  return (
    <DesignModeContext.Provider value={doc.mode}>
    <InspectorPageContext.Provider value={openInspectorPage}>
    {inspectorPage && (inspectorPage.kind === 'paint' || inspectorPage.kind === 'token') ? (
      <InspectorPageView page={inspectorPage} onBack={closeInspector} />
    ) : inspectorPage ? (
      <div className="flex flex-col gap-2 px-2 pb-2 pt-1 text-[12px]">
        <InspectorBack title={inspectorPage.title} onBack={closeInspector} />
        {inspectorPage.kind === 'shadow' ? <ShadowPage doc={doc} node={node} index={inspectorPage.index ?? 0} colorTokens={colorTokens} onPatch={onPatch} /> : null}
        {inspectorPage.kind === 'blur' ? <BlurPage node={node} index={inspectorPage.index ?? 0} onPatch={onPatch} /> : null}
        {inspectorPage.kind === 'text' ? <TextMorePage node={node} setField={setField} selectedText={selectedText} applyType={applyType} /> : null}
      </div>
    ) : (
    <div className="flex flex-col gap-1 px-2 pb-2 text-[12px]">
      {onMakeComponent || onAddToChat ? (
        <div className="flex items-center gap-1">
          {onMakeComponent ? (
            <button type="button" title="Create component" aria-label="Create component" onClick={onMakeComponent} className="flex h-7 w-7 items-center justify-center rounded bg-koma-accent/20 text-koma-accent hover:bg-koma-accent/30">
              <Component size={14} />
            </button>
          ) : null}
          {onAddToChat ? (
            <button type="button" title="Add to chat" aria-label="Add to chat" onClick={onAddToChat} className="flex h-7 w-7 items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg">
              <MessageSquare size={14} />
            </button>
          ) : null}
        </div>
      ) : null}
      {componentName != null ? (
        <label className="flex flex-col gap-1">
          <span className="text-koma-dim">Component</span>
          <input
            value={componentName}
            onFocus={onTypeFocus}
            onBlur={onTypeBlur}
            onChange={(event) => onRenameComponent?.(event.target.value)}
            className="h-7 rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
          />
        </label>
      ) : null}
      {variantProps ? (
        <div className="flex flex-col gap-1">
          <span className="text-koma-dim">Variant</span>
          {Object.entries(variantProps).map(([key, value]) => (
            <label key={key} className="flex items-center gap-1">
              <span className="w-16 flex-none truncate text-koma-dim">{key}</span>
              <input
                value={value}
                aria-label={`${key} value`}
                onFocus={onTypeFocus}
                onBlur={onTypeBlur}
                onChange={(event) => onVariantProps?.({ ...variantProps, [key]: event.target.value })}
                className="h-7 min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
              />
            </label>
          ))}
          <form
            className="flex items-center gap-1"
            onSubmit={(event) => {
              event.preventDefault()
              const name = propName.trim()
              const value = propValue.trim()
              if (!name || !value) return
              onVariantProps?.({ ...variantProps, [name]: value })
              setPropValue('')
            }}
          >
            <input value={propName} aria-label="Property name" onChange={(event) => setPropName(event.target.value)} className="h-7 w-16 flex-none rounded border border-koma-border bg-koma-bg px-1 text-[12px] text-koma-fg outline-none" />
            <input value={propValue} aria-label="Property value" onChange={(event) => setPropValue(event.target.value)} className="h-7 min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-1 text-[12px] text-koma-fg outline-none" />
            <button type="submit" className="h-7 rounded px-1 text-koma-dim hover:bg-koma-hover">Add</button>
          </form>
          {onAddVariant ? (
            <button type="button" title="Add variant" aria-label="Add variant" onClick={onAddVariant} className="flex h-7 w-7 items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg">
              <Plus size={14} />
            </button>
          ) : null}
        </div>
      ) : null}
      {axes?.map((axis) => (
        <Choices
          key={axis.name}
          label={axis.name}
          value={axis.current}
          options={axis.values.map((value) => ({ value, label: value }))}
          onChange={(value) => onInstanceVariant?.({ ...(node.variant ?? {}), [axis.name]: value })}
        />
      ))}
      {node.kind === 'instance' ? (
        <div className="flex flex-col gap-1.5">
          <div className="flex flex-wrap items-center gap-1">
            {onGoToMain ? (
              <button type="button" title="Go to main" aria-label="Go to main" onClick={onGoToMain} className="h-7 rounded px-2 text-[12px] text-koma-dim hover:bg-koma-hover hover:text-koma-fg">Main</button>
            ) : null}
            {onResetInstance ? (
              <button type="button" title="Reset overrides" aria-label="Reset overrides" onClick={onResetInstance} className="h-7 rounded px-2 text-[12px] text-koma-dim hover:bg-koma-hover hover:text-koma-fg">Reset</button>
            ) : null}
            {onDetachInstance ? (
              <button type="button" title="Detach instance" aria-label="Detach instance" onClick={onDetachInstance} className="h-7 rounded px-2 text-[12px] text-koma-dim hover:bg-koma-hover hover:text-koma-fg">Detach</button>
            ) : null}
          </div>
          {onSwapInstance && doc.components.length ? (
            <LabeledControl label="Swap">
              <KomaSelect
                aria-label="Swap component"
                value={node.component ?? ''}
                onChange={(event) => onSwapInstance(event.target.value)}
                className="h-7 w-full px-1.5 text-[12px]"
              >
                {doc.components.map((component) => (
                  <option key={component.id} value={component.id}>{component.name}</option>
                ))}
              </KomaSelect>
            </LabeledControl>
          ) : null}
        </div>
      ) : null}
      {multi ? null : (
        <label className="flex items-center gap-1">
          <span className={`flex h-7 w-7 flex-none items-center justify-center ${node.kind === 'instance' ? 'text-[#9747ff]' : 'text-koma-dim'}`}>
            <KindMark kind={node.kind} />
          </span>
          <input
            aria-label={node.kind === 'text' || node.kind === 'instance' ? 'Text' : 'Name'}
            value={node.kind === 'text' || node.kind === 'instance' ? node.text ?? '' : node.name ?? ''}
            placeholder={node.kind === 'instance' ? 'Override' : node.kind === 'text' ? 'Text' : 'Name'}
            onFocus={onTypeFocus}
            onBlur={onTypeBlur}
            onChange={(event) => {
              const value = event.target.value
              onType((current) => {
                if (current.kind === 'text') return { ...current, text: value }
                if (current.kind === 'instance') {
                  const next = { ...current }
                  if (value) next.text = value
                  else delete next.text
                  return next
                }
                return { ...current, name: value || undefined }
              })
            }}
            className="h-7 min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
          />
        </label>
      )}
      {onAlign ? (
        <div className="flex flex-wrap items-center gap-1">
          <div className="flex gap-0.5">
            <AlignButton label="Align left" onClick={() => onAlign('horizontal', 'min')}><AlignStartVertical size={14} /></AlignButton>
            <AlignButton label="Align center" onClick={() => onAlign('horizontal', 'center')}><AlignCenterVertical size={14} /></AlignButton>
            <AlignButton label="Align right" onClick={() => onAlign('horizontal', 'max')}><AlignEndVertical size={14} /></AlignButton>
          </div>
          <div className="flex gap-0.5">
            <AlignButton label="Align top" onClick={() => onAlign('vertical', 'min')}><AlignStartHorizontal size={14} /></AlignButton>
            <AlignButton label="Align middle" onClick={() => onAlign('vertical', 'center')}><AlignCenterHorizontal size={14} /></AlignButton>
            <AlignButton label="Align bottom" onClick={() => onAlign('vertical', 'max')}><AlignEndHorizontal size={14} /></AlignButton>
          </div>
          {nodes.length >= 3 && hasParent ? (
            <div className="flex gap-0.5">
              <AlignButton label="Distribute horizontal" onClick={() => onAlign('horizontal', 'spread')}><AlignHorizontalSpaceBetween size={14} /></AlignButton>
              <AlignButton label="Distribute vertical" onClick={() => onAlign('vertical', 'spread')}><AlignVerticalSpaceBetween size={14} /></AlignButton>
            </div>
          ) : null}
        </div>
      ) : null}
      <Section title="Layer">
        <div className="flex items-center gap-1">
          <AlignButton
            label={nodes.every((item) => item.visible === false) ? 'Show' : 'Hide'}
            caption={nodes.every((item) => item.visible === false) ? 'Hidden' : 'Visible'}
            pressed={nodes.every((item) => item.visible === false)}
            onClick={() => {
            const show = nodes.some((item) => item.visible === false)
            onPatch((current) => {
              const next = { ...current }
              if (show) delete next.visible
              else next.visible = false
              return next
            })
          }}>{nodes.every((item) => item.visible === false) ? <EyeOff size={14} /> : <Eye size={14} />}</AlignButton>
          <AlignButton
            label={nodes.every((item) => item.locked) ? 'Unlock' : 'Lock'}
            caption={nodes.every((item) => item.locked) ? 'Locked' : 'Unlocked'}
            pressed={nodes.every((item) => !!item.locked)}
            onClick={() => {
            const unlock = nodes.every((item) => item.locked)
            onPatch((current) => {
              const next = { ...current }
              if (unlock) delete next.locked
              else next.locked = true
              return next
            })
          }}>{nodes.every((item) => item.locked) ? <Lock size={14} /> : <Unlock size={14} />}</AlignButton>
        </div>
        <GeomField label="Opacity" ariaLabel="Opacity" suffix="%" min={0} max={100} value={opacityField.value} mixed={opacityField.mixed} tokens={spaceTokens} bound={node.bindings?.opacity} onBind={(token) => bindField('opacity', token)} onChange={(value) => {
          const opacity = Math.min(100, Math.max(0, value)) / 100
          setField(opacity < 1 ? { opacity } : {}, opacity < 1 ? [] : ['opacity'])
        }} />
      </Section>
      <Section title="Measures">
        {!multi && shapeKindOf(node) ? (
          <label className="flex h-8 items-center gap-2 rounded-lg bg-koma-bg px-2">
            <KindMark kind={node.kind} />
            <span className="flex-none text-[11px] text-koma-dim">Shape</span>
            <KomaSelect
              aria-label="Shape"
              value={shapeKindOf(node) ?? 'rect'}
              onChange={(event) => {
                const kind = event.target.value as DesignReshapeKind
                if (kind !== 'rect' && kind !== 'ellipse' && kind !== 'line' && kind !== 'polygon' && kind !== 'star') return
                onPatch((current) => reshapeDesignNode(current, kind))
              }}
              className="h-7 min-w-0 flex-1 px-1.5 text-[12px]"
            >
              <option value="rect">Rectangle</option>
              <option value="ellipse">Ellipse</option>
              <option value="line">Line</option>
              <option value="polygon">Polygon</option>
              <option value="star">Star</option>
            </KomaSelect>
          </label>
        ) : null}
        {!multi && (shapeKindOf(node) === 'polygon' || shapeKindOf(node) === 'star') ? (
          <div className="grid grid-cols-2 gap-1">
            <GeomField
              label="Sides"
              ariaLabel={shapeKindOf(node) === 'star' ? 'Star points' : 'Polygon sides'}
              value={node.pointCount ?? (shapeKindOf(node) === 'star' ? 5 : 6)}
              min={3}
              onChange={(value) => onPatch((current) => retuneDesignShape(current, { pointCount: value }))}
            />
            {shapeKindOf(node) === 'star' ? (
              <GeomField
                label="Inset"
                ariaLabel="Star inset"
                suffix="%"
                value={Math.round((node.innerRadius ?? 0.38) * 100)}
                min={5}
                max={95}
                onChange={(value) => onPatch((current) => retuneDesignShape(current, { innerRadius: Math.min(95, Math.max(5, value)) / 100 }))}
              />
            ) : null}
          </div>
        ) : null}
        <div className="grid grid-cols-2 gap-1">
          <GeomField label="X" value={xField.value} mixed={xField.mixed} onChange={(x) => setField({ x })} />
          <GeomField label="Y" value={yField.value} mixed={yField.mixed} onChange={(y) => setField({ y })} />
          <GeomField label="W" min={1} value={wField.value} mixed={wField.mixed} onChange={(w) => setField({ w: Math.max(1, w) }, ['wMode'])} />
          <GeomField label="H" min={1} value={hField.value} mixed={hField.mixed} onChange={(h) => setField({ h: Math.max(1, h) }, ['hMode'])} />
        </div>
        {sizeModes ? (
          <div className="grid grid-cols-2 gap-1">
            <FieldGroup label="Width">
              <SizeMode label="Width sizing" value={wModeField.value} mixed={wModeField.mixed} onChange={(mode) => setField(mode === 'fixed' ? {} : { wMode: mode }, mode === 'fixed' ? ['wMode'] : [])} />
            </FieldGroup>
            <FieldGroup label="Height">
              <SizeMode label="Height sizing" value={hModeField.value} mixed={hModeField.mixed} onChange={(mode) => setField(mode === 'fixed' ? {} : { hMode: mode }, mode === 'fixed' ? ['hMode'] : [])} />
            </FieldGroup>
          </div>
        ) : null}
        <div className="flex items-center gap-1">
          <div className="min-w-0 flex-1">
            <GeomField label="Angle" suffix="°" value={rotationField.value} mixed={rotationField.mixed} onChange={(rotation) => {
              const wrapped = ((rotation % 360) + 360) % 360
              setField(wrapped ? { rotation: wrapped } : {}, wrapped ? [] : ['rotation'])
            }} />
          </div>
          <AlignButton label="Flip horizontal" pressed={flipXOn} onClick={() => onPatch((current) => {
            const next = { ...current }
            if (current.flipX) delete next.flipX
            else next.flipX = true
            return next
          })}><FlipHorizontal2 size={14} /></AlignButton>
          <AlignButton label="Flip vertical" pressed={flipYOn} onClick={() => onPatch((current) => {
            const next = { ...current }
            if (current.flipY) delete next.flipY
            else next.flipY = true
            return next
          })}><FlipVertical2 size={14} /></AlignButton>
          <AlignButton label="Lock proportions" pressed={nodes.every((item) => !!item.proportion)} onClick={() => {
            const allOn = nodes.every((item) => item.proportion)
            onPatch((current) => {
              const next = { ...current }
              if (allOn) delete next.proportion
              else next.proportion = true
              return next
            })
          }}><Crop size={14} /></AlignButton>
          <AlignButton label="Rotate 90 degrees" onClick={() => onPatch((current) => {
            const wrapped = (((current.rotation ?? 0) + 90) % 360 + 360) % 360
            const next = { ...current }
            if (wrapped) next.rotation = wrapped
            else delete next.rotation
            return next
          })}><RotateCw size={14} /></AlignButton>
        </div>
        {allFrames ? (
          <LabeledControl label="Preset" wide>
            <KomaSelect
              aria-label="Frame preset"
              value=""
              onChange={(event) => {
                const preset = FRAME_PRESETS.find((item) => item.name === event.target.value)
                if (preset) onPatch((current) => applyFramePreset(current, preset))
              }}
              className="h-7 w-full px-1.5 text-[12px]"
            >
              <option value="">Choose size</option>
              {FRAME_PRESETS.map((preset) => (
                <option key={preset.name} value={preset.name}>{preset.name}</option>
              ))}
            </KomaSelect>
          </LabeledControl>
        ) : null}
        {sizeModes || allFrames ? (
          <div className="grid grid-cols-2 gap-1">
            <GeomField label="Min W" ariaLabel="Minimum width" value={minWField.value} mixed={minWField.mixed} onChange={(minW) => setField(minW > 0 ? { minW } : {}, minW > 0 ? [] : ['minW'])} />
            <GeomField label="Max W" ariaLabel="Maximum width" value={maxWField.value} mixed={maxWField.mixed} onChange={(maxW) => setField(maxW > 0 ? { maxW } : {}, maxW > 0 ? [] : ['maxW'])} />
            <GeomField label="Min H" ariaLabel="Minimum height" value={minHField.value} mixed={minHField.mixed} onChange={(minH) => setField(minH > 0 ? { minH } : {}, minH > 0 ? [] : ['minH'])} />
            <GeomField label="Max H" ariaLabel="Maximum height" value={maxHField.value} mixed={maxHField.mixed} onChange={(maxH) => setField(maxH > 0 ? { maxH } : {}, maxH > 0 ? [] : ['maxH'])} />
          </div>
        ) : null}
      </Section>
      {allFrames || hasParent ? (
      <Section title="Layout">
      {allFrames ? (
        <div className="flex flex-col gap-1.5">
          <LabeledControl label="Type">
            <KomaSelect
              aria-label="Layout"
              value={layoutField.mixed ? '' : layoutField.value}
              onChange={(event) => {
                const layout = event.target.value
                if (layout === 'free') setField({}, ['layout', 'gridColumns', 'gridRows'])
                else if (layout === 'row' || layout === 'column') setField({ layout }, ['gridColumns', 'gridRows'])
                else if (layout === 'grid') setField({ layout: 'grid', gridColumns: [{ size: 'fr', count: 2 }], gridRows: [{ size: 'fr', count: 2 }] })
              }}
              className="h-7 w-full px-1.5 text-[12px]"
            >
              {layoutField.mixed ? <option value="">Mixed</option> : null}
              <option value="free">Free</option>
              <option value="row">Row</option>
              <option value="column">Column</option>
              <option value="grid">Grid</option>
            </KomaSelect>
          </LabeledControl>
          {layoutField.value === 'row' || layoutField.value === 'column' ? (
            <AlignButton
              label="Reverse"
              caption="Reverse direction"
              pressed={nodes.every((item) => item.reverse)}
              onClick={() => {
                const allOn = nodes.every((item) => item.reverse)
                onPatch((current) => {
                  const next = { ...current }
                  if (allOn) delete next.reverse
                  else next.reverse = true
                  return next
                })
              }}
            >
              <RotateCcw size={14} />
            </AlignButton>
          ) : null}
        </div>
      ) : null}
      {!layoutField.mixed && layoutField.value === 'grid' ? (
        <div className="flex flex-col gap-1">
          {(['gridColumns', 'gridRows'] as const).map((key) => (
            <FieldGroup key={key} label={key === 'gridColumns' ? 'Columns' : 'Rows'}>
              {(node[key]?.length ? node[key]! : [{ size: 'fr' as const, count: 2 }]).map((track, index) => (
                <div key={`${key}-${index}`} className="flex flex-col gap-1">
                  <LabeledControl label="Unit">
                    <KomaSelect
                      aria-label={key === 'gridColumns' ? 'Column track' : 'Row track'}
                      value={typeof track.size === 'number' ? 'px' : track.size}
                      onChange={(event) => {
                        const unit = event.target.value
                        const tracks = (node[key] ?? [{ size: 'fr' as const, count: 2 }]).slice()
                        tracks[index] = { ...track, size: unit === 'px' ? 80 : unit === 'auto' ? 'auto' : 'fr' }
                        setField({ layout: 'grid', [key]: tracks })
                      }}
                      className="h-7 w-full px-1.5 text-[12px]"
                    >
                      <option value="fr">Fraction</option>
                      <option value="px">Pixels</option>
                      <option value="auto">Auto</option>
                    </KomaSelect>
                  </LabeledControl>
                  <div className="flex items-center gap-1">
                    <div className="min-w-0 flex-1">
                      <GeomField
                        label={typeof track.size === 'number' ? 'Size' : 'Count'}
                        ariaLabel={typeof track.size === 'number' ? 'Track size' : 'Track count'}
                        value={typeof track.size === 'number' ? track.size : track.count ?? 1}
                        onChange={(value) => {
                          const tracks = (node[key] ?? [{ size: 'fr' as const, count: 2 }]).slice()
                          tracks[index] = typeof track.size === 'number' ? { ...track, size: Math.max(1, value) } : { ...track, count: Math.max(1, Math.round(value)) }
                          setField({ layout: 'grid', [key]: tracks })
                        }}
                      />
                    </div>
                    <AlignButton label="Remove track" onClick={() => {
                      const tracks = (node[key] ?? []).filter((_, at) => at !== index)
                      setField({ layout: 'grid', [key]: tracks.length ? tracks : [{ size: 'fr', count: 1 }] })
                    }}><X size={12} /></AlignButton>
                  </div>
                </div>
              ))}
              <AlignButton label={key === 'gridColumns' ? 'Add column track' : 'Add row track'} caption={key === 'gridColumns' ? 'Add column' : 'Add row'} onClick={() => setField({ layout: 'grid', [key]: [...(node[key] ?? []), { size: 'fr', count: 1 }] })}>
                <Plus size={12} />
              </AlignButton>
            </FieldGroup>
          ))}
        </div>
      ) : null}
      {flows ? (
        <>
          <GeomField label="Gap" value={gapField.value} mixed={gapField.mixed} tokens={spaceTokens} bound={node.bindings?.gap} onBind={(token) => bindField('gap', token)} onChange={(gap) => setField(gap > 0 ? { gap } : {}, gap > 0 ? [] : ['gap'])} />
          <GeomField label="Padding" value={padField.value} mixed={padField.mixed} tokens={spaceTokens} bound={node.bindings?.pad} onBind={(token) => bindField('pad', token)} onChange={(pad) => setField(pad > 0 ? { pad } : {}, pad > 0 ? ['padTop', 'padRight', 'padBottom', 'padLeft'] : ['pad', 'padTop', 'padRight', 'padBottom', 'padLeft'])} />
          {nodes.every((item) => item.wrap) ? (
            <div className="grid grid-cols-2 gap-1">
              <GeomField label="Col" ariaLabel="Column gap" value={numberOf((item) => item.gapX ?? item.gap ?? 0).value} mixed={numberOf((item) => item.gapX ?? item.gap ?? 0).mixed} tokens={spaceTokens} bound={node.bindings?.gapX} onBind={(token) => bindField('gapX', token)} onChange={(gapX) => setField(gapX > 0 ? { gapX } : {}, gapX > 0 ? [] : ['gapX'])} />
              <GeomField label="Row" ariaLabel="Row gap" value={numberOf((item) => item.gapY ?? item.gap ?? 0).value} mixed={numberOf((item) => item.gapY ?? item.gap ?? 0).mixed} tokens={spaceTokens} bound={node.bindings?.gapY} onBind={(token) => bindField('gapY', token)} onChange={(gapY) => setField(gapY > 0 ? { gapY } : {}, gapY > 0 ? [] : ['gapY'])} />
            </div>
          ) : null}
          <div className="grid grid-cols-2 gap-1">
            <GeomField label="Top" ariaLabel="Padding top" value={padTopField.value} mixed={padTopField.mixed} onChange={(value) => setSide('padTop', value)} />
            <GeomField label="Right" ariaLabel="Padding right" value={padRightField.value} mixed={padRightField.mixed} onChange={(value) => setSide('padRight', value)} />
            <GeomField label="Bottom" ariaLabel="Padding bottom" value={padBottomField.value} mixed={padBottomField.mixed} onChange={(value) => setSide('padBottom', value)} />
            <GeomField label="Left" ariaLabel="Padding left" value={padLeftField.value} mixed={padLeftField.mixed} onChange={(value) => setSide('padLeft', value)} />
          </div>
          <div className="flex flex-col gap-1.5">
            <FieldGroup label="Justify">
              <div className="flex flex-wrap gap-0.5">
              {columnLayout ? (
                <>
                  <AlignButton label="Align top" pressed={!justifyField.mixed && justifyField.value === 'start'} onClick={() => setJustify('start')}><AlignStartHorizontal size={14} /></AlignButton>
                  <AlignButton label="Align middle" pressed={!justifyField.mixed && justifyField.value === 'center'} onClick={() => setJustify('center')}><AlignCenterHorizontal size={14} /></AlignButton>
                  <AlignButton label="Align bottom" pressed={!justifyField.mixed && justifyField.value === 'end'} onClick={() => setJustify('end')}><AlignEndHorizontal size={14} /></AlignButton>
                  <AlignButton label="Space between" pressed={!justifyField.mixed && justifyField.value === 'space'} onClick={() => setJustify('space')}><AlignVerticalSpaceBetween size={14} /></AlignButton>
                  <AlignButton label="Space around" caption="Around" pressed={!justifyField.mixed && justifyField.value === 'around'} onClick={() => setJustify('around')}><AlignVerticalSpaceBetween size={14} /></AlignButton>
                  <AlignButton label="Space evenly" caption="Even" pressed={!justifyField.mixed && justifyField.value === 'evenly'} onClick={() => setJustify('evenly')}><AlignVerticalSpaceBetween size={14} /></AlignButton>
                </>
              ) : (
                <>
                  <AlignButton label="Align left" pressed={!justifyField.mixed && justifyField.value === 'start'} onClick={() => setJustify('start')}><AlignStartVertical size={14} /></AlignButton>
                  <AlignButton label="Align center" pressed={!justifyField.mixed && justifyField.value === 'center'} onClick={() => setJustify('center')}><AlignCenterVertical size={14} /></AlignButton>
                  <AlignButton label="Align right" pressed={!justifyField.mixed && justifyField.value === 'end'} onClick={() => setJustify('end')}><AlignEndVertical size={14} /></AlignButton>
                  <AlignButton label="Space between" pressed={!justifyField.mixed && justifyField.value === 'space'} onClick={() => setJustify('space')}><AlignHorizontalSpaceBetween size={14} /></AlignButton>
                  <AlignButton label="Space around" caption="Around" pressed={!justifyField.mixed && justifyField.value === 'around'} onClick={() => setJustify('around')}><AlignHorizontalSpaceBetween size={14} /></AlignButton>
                  <AlignButton label="Space evenly" caption="Even" pressed={!justifyField.mixed && justifyField.value === 'evenly'} onClick={() => setJustify('evenly')}><AlignHorizontalSpaceBetween size={14} /></AlignButton>
                </>
              )}
              </div>
            </FieldGroup>
            <FieldGroup label="Align">
              <div className="flex flex-wrap gap-0.5">
              {columnLayout ? (
                <>
                  <AlignButton label="Align left" pressed={!alignField.mixed && alignField.value === 'start'} onClick={() => setCross('start')}><AlignStartVertical size={14} /></AlignButton>
                  <AlignButton label="Align center" pressed={!alignField.mixed && alignField.value === 'center'} onClick={() => setCross('center')}><AlignCenterVertical size={14} /></AlignButton>
                  <AlignButton label="Align right" pressed={!alignField.mixed && alignField.value === 'end'} onClick={() => setCross('end')}><AlignEndVertical size={14} /></AlignButton>
                  <AlignButton label="Stretch" caption="Stretch" pressed={!alignField.mixed && alignField.value === 'stretch'} onClick={() => setCross('stretch')}><span className="text-[11px] font-semibold">↔</span></AlignButton>
                </>
              ) : (
                <>
                  <AlignButton label="Align top" pressed={!alignField.mixed && alignField.value === 'start'} onClick={() => setCross('start')}><AlignStartHorizontal size={14} /></AlignButton>
                  <AlignButton label="Align middle" pressed={!alignField.mixed && alignField.value === 'center'} onClick={() => setCross('center')}><AlignCenterHorizontal size={14} /></AlignButton>
                  <AlignButton label="Align bottom" pressed={!alignField.mixed && alignField.value === 'end'} onClick={() => setCross('end')}><AlignEndHorizontal size={14} /></AlignButton>
                  <AlignButton label="Stretch" caption="Stretch" pressed={!alignField.mixed && alignField.value === 'stretch'} onClick={() => setCross('stretch')}><span className="text-[11px] font-semibold">↕</span></AlignButton>
                </>
              )}
              </div>
            </FieldGroup>
          </div>
          {nodes.every((item) => item.wrap) ? (
            <LabeledControl label="Content" wide>
              <KomaSelect
                aria-label="Content alignment"
                value={node.alignContent ?? 'start'}
                onChange={(event) => {
                  const alignContent = event.target.value
                  if (alignContent === 'start') setField({}, ['alignContent'])
                  else if (alignContent === 'center' || alignContent === 'end' || alignContent === 'space' || alignContent === 'around' || alignContent === 'evenly') setField({ alignContent })
                }}
                className="h-7 w-full px-1.5 text-[12px]"
              >
                <option value="start">Start</option>
                <option value="center">Center</option>
                <option value="end">End</option>
                <option value="space">Space between</option>
                <option value="around">Space around</option>
                <option value="evenly">Space evenly</option>
              </KomaSelect>
            </LabeledControl>
          ) : null}
        </>
      ) : null}
      {hasParent && parentLayout === 'grid' ? (
        <div className="grid grid-cols-2 gap-1">
          <GeomField label="Col" ariaLabel="Column start" value={node.colStart ?? 0} onChange={(colStart) => setField(colStart > 0 ? { colStart: Math.round(colStart) } : {}, colStart > 0 ? [] : ['colStart'])} />
          <GeomField label="Span" ariaLabel="Column span" value={node.colSpan ?? 1} onChange={(colSpan) => setField(colSpan > 1 ? { colSpan: Math.round(colSpan) } : {}, colSpan > 1 ? [] : ['colSpan'])} />
          <GeomField label="Row" ariaLabel="Row start" value={node.rowStart ?? 0} onChange={(rowStart) => setField(rowStart > 0 ? { rowStart: Math.round(rowStart) } : {}, rowStart > 0 ? [] : ['rowStart'])} />
          <GeomField label="Span" ariaLabel="Row span" value={node.rowSpan ?? 1} onChange={(rowSpan) => setField(rowSpan > 1 ? { rowSpan: Math.round(rowSpan) } : {}, rowSpan > 1 ? [] : ['rowSpan'])} />
        </div>
      ) : null}
      <div className="flex flex-wrap items-center gap-1">
      {flows ? (
        <AlignButton
          label={textOf((item) => (item.wrap ? 'on' : 'off')).mixed ? 'Wrap · Mixed' : nodes.every((item) => item.wrap) ? 'Wrap on' : 'Wrap off'}
          caption="Wrap"
          pressed={nodes.every((item) => item.wrap === true)}
          onClick={() => {
            const allOn = nodes.every((item) => item.wrap === true)
            onPatch((current) => {
              const next = { ...current }
              if (allOn) delete next.wrap
              else next.wrap = true
              return next
            })
          }}
        >
          <WrapText size={14} />
        </AlignButton>
      ) : null}
      {nodes.every((item) => item.kind === 'frame' || item.kind === 'instance') ? (
        <AlignButton
          label={nodes.every((item) => item.clip !== false) ? 'Clip on' : 'Clip off'}
          caption="Clip"
          pressed={nodes.every((item) => item.clip !== false)}
          onClick={() => {
            const allOn = nodes.every((item) => item.clip !== false)
            onPatch((current) => {
              const next = { ...current }
              if (allOn) next.clip = false
              else delete next.clip
              return next
            })
          }}
        >
          <Crop size={14} />
        </AlignButton>
      ) : null}
      {hasParent ? (
        <AlignButton
          label={absoluteField.mixed ? 'Absolute · Mixed' : absoluteField.value === 'on' ? 'Absolute on' : 'Absolute off'}
          caption="Absolute"
          pressed={absoluteField.value === 'on' && !absoluteField.mixed}
          onClick={() => {
            const allOn = nodes.every((item) => item.absolute === true)
            onPatch((current) => {
              const next = { ...current }
              if (allOn) delete next.absolute
              else next.absolute = true
              return next
            })
          }}
        >
          <Move size={14} />
        </AlignButton>
      ) : null}
      </div>
      {hasParent && (parentLayout === 'row' || parentLayout === 'column' || parentLayout === 'grid') ? (
        <LabeledControl label="Align">
          <KomaSelect
            aria-label="Align self"
            value={node.alignSelf ?? 'start'}
            onChange={(event) => {
              const alignSelf = event.target.value
              if (alignSelf !== 'start' && alignSelf !== 'center' && alignSelf !== 'end' && alignSelf !== 'stretch') return
              setField(alignSelf === 'start' ? {} : { alignSelf }, alignSelf === 'start' ? ['alignSelf'] : [])
            }}
            className="h-7 w-full px-1.5 text-[12px]"
          >
            <option value="start">Start</option>
            <option value="center">Center</option>
            <option value="end">End</option>
            <option value="stretch">Stretch</option>
          </KomaSelect>
        </LabeledControl>
      ) : null}
      {hasParent ? (
        <div className="grid grid-cols-2 gap-1">
          <GeomField label="Top" ariaLabel="Margin top" value={numberOf((item) => item.marginTop ?? item.margin ?? 0).value} onChange={(marginTop) => setField(marginTop ? { marginTop } : {}, marginTop ? [] : ['marginTop'])} />
          <GeomField label="Right" ariaLabel="Margin right" value={numberOf((item) => item.marginRight ?? item.margin ?? 0).value} onChange={(marginRight) => setField(marginRight ? { marginRight } : {}, marginRight ? [] : ['marginRight'])} />
          <GeomField label="Bottom" ariaLabel="Margin bottom" value={numberOf((item) => item.marginBottom ?? item.margin ?? 0).value} onChange={(marginBottom) => setField(marginBottom ? { marginBottom } : {}, marginBottom ? [] : ['marginBottom'])} />
          <GeomField label="Left" ariaLabel="Margin left" value={numberOf((item) => item.marginLeft ?? item.margin ?? 0).value} onChange={(marginLeft) => setField(marginLeft ? { marginLeft } : {}, marginLeft ? [] : ['marginLeft'])} />
        </div>
      ) : null}
      {allFrames ? (
        <div className="flex flex-col gap-1">
          <AlignButton
            label="Board grid"
            caption={node.layoutGrids?.length ? 'Board grid on' : 'Board grid'}
            pressed={!!node.layoutGrids?.length}
            onClick={() => setField(node.layoutGrids?.length ? {} : { layoutGrids: [{ kind: 'square', size: 8 }] }, node.layoutGrids?.length ? ['layoutGrids'] : [])}
          >
            <Square size={14} />
          </AlignButton>
          {(node.layoutGrids ?? []).map((grid, index) => {
            const patchGrid = (next: Partial<DesignLayoutGrid>) => setField({ layoutGrids: (node.layoutGrids ?? []).map((item, at) => (at === index ? { ...item, ...next } : item)) })
            return (
              <div key={`${grid.kind}-${index}`} className="flex flex-col gap-1 rounded border border-koma-border bg-koma-bg p-1">
                <div className="flex items-center gap-1">
                  <KomaSelect
                    aria-label="Grid kind"
                    value={grid.kind}
                    onChange={(event) => {
                      const kind = event.target.value
                      if (kind === 'square' || kind === 'column' || kind === 'row') patchGrid({ kind })
                    }}
                    className="h-8 min-w-0 flex-1 px-1.5 text-[12px]"
                  >
                    <option value="square">Square</option>
                    <option value="column">Columns</option>
                    <option value="row">Rows</option>
                  </KomaSelect>
                  <button
                    type="button"
                    title="Remove grid"
                    aria-label="Remove grid"
                    className="flex h-8 w-8 items-center justify-center rounded-lg text-koma-dim hover:bg-koma-hover"
                    onClick={() => {
                      const layoutGrids = (node.layoutGrids ?? []).filter((_, at) => at !== index)
                      setField(layoutGrids.length ? { layoutGrids } : {}, layoutGrids.length ? [] : ['layoutGrids'])
                    }}
                  >
                    <X size={12} />
                  </button>
                </div>
                <div className="grid grid-cols-2 gap-1">
                  <GeomField label="Size" ariaLabel="Grid size" value={grid.size ?? 8} onChange={(size) => patchGrid({ size })} />
                  {grid.kind !== 'square' ? (
                    <GeomField label="Count" ariaLabel="Grid count" value={grid.count ?? 0} onChange={(count) => patchGrid({ count: count > 0 ? Math.round(count) : undefined })} />
                  ) : null}
                  {grid.kind !== 'square' ? (
                    <GeomField label="Gutter" ariaLabel="Grid gutter" value={grid.gutter ?? 0} onChange={(gutter) => patchGrid({ gutter: gutter > 0 ? gutter : undefined })} />
                  ) : null}
                  {grid.kind !== 'square' ? (
                    <GeomField label="Offset" ariaLabel="Grid offset" value={grid.offset ?? 0} onChange={(offset) => patchGrid({ offset })} />
                  ) : null}
                </div>
                {grid.kind !== 'square' ? (
                  <LabeledControl label="Align">
                    <KomaSelect
                      aria-label="Grid align"
                      value={grid.align ?? 'stretch'}
                      onChange={(event) => {
                        const align = event.target.value
                        if (align !== 'stretch' && align !== 'start' && align !== 'center' && align !== 'end') return
                        patchGrid({ align: align === 'stretch' ? undefined : align })
                      }}
                      className="h-7 w-full px-1.5 text-[12px]"
                    >
                      <option value="stretch">Stretch</option>
                      <option value="start">Start</option>
                      <option value="center">Center</option>
                      <option value="end">End</option>
                    </KomaSelect>
                  </LabeledControl>
                ) : null}
                <label className="flex h-7 items-center gap-1 rounded border border-koma-border bg-koma-panel px-1.5">
                  <span className="text-[11px] text-koma-dim">Color</span>
                  <input
                    aria-label="Grid color"
                    value={grid.color ?? ''}
                    placeholder="auto"
                    onChange={(event) => patchGrid({ color: event.target.value || undefined })}
                    className="h-6 min-w-0 flex-1 bg-transparent text-[12px] text-koma-fg outline-none"
                  />
                </label>
              </div>
            )
          })}
          {node.layoutGrids?.length ? (
            <AlignButton label="Add board grid" onClick={() => setField({ layoutGrids: [...(node.layoutGrids ?? []), { kind: 'column', count: 12, gutter: 16, offset: 0 }] })}>
              <Plus size={14} />
            </AlignButton>
          ) : null}
        </div>
      ) : null}
      </Section>
      ) : null}
      <Section title="Appearance">
        {showRadius ? (
          <RadiusField
            value={radiusField.mixed ? undefined : node.radius}
            mixed={radiusField.mixed}
            resolved={typeof node.radius === 'number' ? String(node.radius) : resolveRef(doc, typeof node.radius === 'string' ? node.radius : '')}
            tokens={radiusTokens}
            onChange={(radius) => {
              if (radius == null) setField({}, ['radius'])
              else setField({ radius })
            }}
          />
        ) : null}
        {showRadius ? (
          <AlignButton label={radiusSplit ? 'One radius' : 'Four corners'} caption={radiusSplit ? 'One radius' : 'Four corners'} pressed={radiusSplit} onClick={() => {
            setRadiusSplit((current) => !current)
            if (radiusSplit) setField({}, ['radiusTL', 'radiusTR', 'radiusBR', 'radiusBL'])
          }}><Crop size={14} /></AlignButton>
        ) : null}
        {showRadius && radiusSplit ? (
          <div className="grid grid-cols-2 gap-1">
            <GeomField label="Top L" ariaLabel="Top left radius" value={cornerField((item) => resolvedCorner(item.radiusTL ?? item.radius)).value} mixed={cornerField((item) => resolvedCorner(item.radiusTL ?? item.radius)).mixed} tokens={radiusTokens} bound={node.bindings?.radiusTL} onBind={(token) => bindField('radiusTL', token)} onChange={(value) => setCorner('radiusTL', value)} />
            <GeomField label="Top R" ariaLabel="Top right radius" value={cornerField((item) => resolvedCorner(item.radiusTR ?? item.radius)).value} mixed={cornerField((item) => resolvedCorner(item.radiusTR ?? item.radius)).mixed} tokens={radiusTokens} bound={node.bindings?.radiusTR} onBind={(token) => bindField('radiusTR', token)} onChange={(value) => setCorner('radiusTR', value)} />
            <GeomField label="Bot R" ariaLabel="Bottom right radius" value={cornerField((item) => resolvedCorner(item.radiusBR ?? item.radius)).value} mixed={cornerField((item) => resolvedCorner(item.radiusBR ?? item.radius)).mixed} tokens={radiusTokens} bound={node.bindings?.radiusBR} onBind={(token) => bindField('radiusBR', token)} onChange={(value) => setCorner('radiusBR', value)} />
            <GeomField label="Bot L" ariaLabel="Bottom left radius" value={cornerField((item) => resolvedCorner(item.radiusBL ?? item.radius)).value} mixed={cornerField((item) => resolvedCorner(item.radiusBL ?? item.radius)).mixed} tokens={radiusTokens} bound={node.bindings?.radiusBL} onBind={(token) => bindField('radiusBL', token)} onChange={(value) => setCorner('radiusBL', value)} />
          </div>
        ) : null}
        <div className="flex items-center gap-1">
          <label className="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-lg bg-koma-bg px-2">
            <span className="flex-none text-[11px] text-koma-dim">Blend</span>
            <KomaSelect
              aria-label="Blend"
              value={node.blend ?? 'normal'}
              onChange={(event) => {
                const blend = event.target.value
                setField(blend === 'normal' ? {} : { blend }, blend === 'normal' ? ['blend'] : [])
              }}
              className="h-7 min-w-0 flex-1 px-1.5 text-[12px]"
            >
            <option value="normal">Normal</option>
            <option value="multiply">Multiply</option>
            <option value="screen">Screen</option>
            <option value="overlay">Overlay</option>
            <option value="darken">Darken</option>
            <option value="lighten">Lighten</option>
            <option value="color-burn">Color burn</option>
            <option value="color-dodge">Color dodge</option>
            <option value="soft-light">Soft light</option>
            <option value="hard-light">Hard light</option>
            <option value="difference">Difference</option>
            <option value="exclusion">Exclusion</option>
            <option value="hue">Hue</option>
            <option value="saturation">Saturation</option>
            <option value="color">Color</option>
            <option value="luminosity">Luminosity</option>
            <option value="pass-through">Pass through</option>
            </KomaSelect>
          </label>
          <AlignButton label="Use as mask" caption="Mask" pressed={!!node.mask} onClick={() => setField(node.mask ? {} : { mask: true }, node.mask ? ['mask', 'maskType'] : [])}>
            <Layers size={14} />
          </AlignButton>
        </div>
        {node.mask ? (
          <Choices
            label="Mask type"
            value={node.maskType ?? 'alpha'}
            options={[
              { value: 'alpha', label: 'Alpha' },
              { value: 'luminance', label: 'Luma' },
              { value: 'vector', label: 'Vector' },
            ]}
            onChange={(maskType) => setField({ mask: true, ...(maskType === 'alpha' ? {} : { maskType }) }, maskType === 'alpha' ? ['maskType'] : [])}
          />
        ) : null}
      </Section>
      <Section
        title="Fill"
        action={(
          <AlignButton label="Add fill" onClick={() => onPatch((current) => {
            if (nodeHasPaint(current, 'fill')) return current
            const color = current.kind === 'frame' ? '#ffffff' : SHAPE_FILL
            return setNodeSolid(current, 'fill', color)
          })}>
            <Plus size={14} />
          </AlignButton>
        )}
      >
        {multi ? (
          fillField.mixed || fillField.value !== 'none' ? (
          <PaintRow
            label="Fill"
            doc={doc}
            mixed={fillField.mixed}
            value={fillField.value}
            fallback="#1a1d27"
            resolved={resolveRef(doc, chrome.fill)}
            tokens={colorTokens}
            onChange={(next) => paintChange('fill', next)}
          />
          ) : null
        ) : (
          <FillEditor label="Fill" doc={doc} node={node} field="fill" tokens={colorTokens} onChange={(next) => onPatch(() => next)} onPickImage={onPickImage} onStoreImage={onStoreImage} />
        )}
      </Section>
      <Section
        title="Stroke"
        action={(
          <AlignButton label="Add stroke" onClick={() => onPatch((current) => nodeHasPaint(current, 'stroke') ? current : setNodeSolid(current, 'stroke', '#1c1c1c'))}>
            <Plus size={14} />
          </AlignButton>
        )}
      >
        {multi ? (
          strokeField.mixed || strokeField.value !== 'none' ? (
          <PaintRow
            label="Stroke"
            doc={doc}
            mixed={strokeField.mixed}
            value={strokeField.value}
            fallback="#8b93b8"
            resolved={resolveRef(doc, chrome.stroke)}
            tokens={colorTokens}
            onChange={(next) => paintChange('stroke', next)}
          />
          ) : null
        ) : (
          <FillEditor label="Stroke" doc={doc} node={node} field="stroke" tokens={colorTokens} onChange={(next) => onPatch(() => next)} onStoreImage={onStoreImage} />
        )}
      </Section>
      {hasParent && (!sizeModes || node.absolute) ? (
        <Section title="Constraints">
          <div className="flex items-start gap-2">
            <ConstraintWidget
              horizontal={node.constraintH ?? 'start'}
              vertical={node.constraintV ?? 'start'}
              onHorizontal={(constraintH) => setField(constraintH === 'start' ? {} : { constraintH }, constraintH === 'start' ? ['constraintH'] : [])}
              onVertical={(constraintV) => setField(constraintV === 'start' ? {} : { constraintV }, constraintV === 'start' ? ['constraintV'] : [])}
            />
            <div className="flex min-w-0 flex-1 flex-col gap-2">
              <label className="flex min-w-0 flex-col gap-1">
                <span className="text-[11px] text-koma-dim">Horizontal</span>
                <KomaSelect
                  aria-label="Horizontal constraint"
                  value={node.constraintH ?? 'start'}
                  onChange={(event) => {
                    const constraintH = event.target.value
                    if (constraintH !== 'start' && constraintH !== 'center' && constraintH !== 'end' && constraintH !== 'stretch' && constraintH !== 'scale') return
                    setField(constraintH === 'start' ? {} : { constraintH }, constraintH === 'start' ? ['constraintH'] : [])
                  }}
                  className="h-8 w-full px-1.5 text-[12px]"
                >
                  <option value="start">Left</option>
                  <option value="center">Center</option>
                  <option value="end">Right</option>
                  <option value="stretch">Left and right</option>
                  <option value="scale">Scale</option>
                </KomaSelect>
              </label>
              <label className="flex min-w-0 flex-col gap-1">
                <span className="text-[11px] text-koma-dim">Vertical</span>
                <KomaSelect
                  aria-label="Vertical constraint"
                  value={node.constraintV ?? 'start'}
                  onChange={(event) => {
                    const constraintV = event.target.value
                    if (constraintV !== 'start' && constraintV !== 'center' && constraintV !== 'end' && constraintV !== 'stretch' && constraintV !== 'scale') return
                    setField(constraintV === 'start' ? {} : { constraintV }, constraintV === 'start' ? ['constraintV'] : [])
                  }}
                  className="h-8 w-full px-1.5 text-[12px]"
                >
                  <option value="start">Top</option>
                  <option value="center">Center</option>
                  <option value="end">Bottom</option>
                  <option value="stretch">Top and bottom</option>
                  <option value="scale">Scale</option>
                </KomaSelect>
              </label>
            </div>
          </div>
        </Section>
      ) : null}
      <Section
        title="Shadow"
        action={(
          <AlignButton label="Add drop shadow" onClick={() => onPatch((current) => ({ ...current, effects: [...(current.effects ?? []), { kind: 'drop-shadow', x: 4, y: 4, blur: 4, spread: 0, color: '#000000' }] }))}>
            <Plus size={14} />
          </AlignButton>
        )}
      >
        {(node.effects ?? []).map((effect, index) => {
          if (effect.kind !== 'drop-shadow' && effect.kind !== 'inner-shadow') return null
          return (
            <div key={`${effect.kind}-${index}`} className="flex items-center gap-1">
              <button
                type="button"
                className="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-lg bg-koma-bg px-2 text-left text-[12px] text-koma-fg hover:bg-koma-hover"
                onClick={() => setInspectorPage({ kind: 'shadow', title: 'Shadow', index })}
              >
                <span className="h-4 w-4 flex-none rounded-full border border-koma-border" style={{ background: effect.color && effect.color !== 'none' ? effect.color : '#000000' }} />
                <span className="min-w-0 flex-1 truncate">{effect.kind === 'inner-shadow' ? 'Inner' : 'Drop'}</span>
                <ChevronRight size={14} className="flex-none text-koma-dim" />
              </button>
              <button type="button" title="Remove" aria-label="Remove shadow" className="flex h-8 w-8 flex-none items-center justify-center rounded-lg text-koma-dim hover:bg-koma-hover hover:text-koma-fg" onClick={() => onPatch((current) => ({ ...current, effects: (current.effects ?? []).filter((_, at) => at !== index) }))}>
                <X size={13} />
              </button>
            </div>
          )
        })}
      </Section>
      <Section
        title="Blur"
        action={(
          <AlignButton label="Add blur" onClick={() => onPatch((current) => ({ ...current, effects: [...(current.effects ?? []), { kind: 'layer-blur', blur: 4 }] }))}>
            <Plus size={14} />
          </AlignButton>
        )}
      >
        {(node.effects ?? []).map((effect, index) => {
          if (effect.kind !== 'layer-blur' && effect.kind !== 'background-blur') return null
          return (
            <div key={`${effect.kind}-${index}`} className="flex items-center gap-1">
              <button
                type="button"
                className="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-lg bg-koma-bg px-2 text-left text-[12px] text-koma-fg hover:bg-koma-hover"
                onClick={() => setInspectorPage({ kind: 'blur', title: 'Blur', index })}
              >
                <span className="min-w-0 flex-1 truncate">{effect.kind === 'background-blur' ? 'Background' : 'Layer'} · {effect.blur ?? 4}</span>
                <ChevronRight size={14} className="flex-none text-koma-dim" />
              </button>
              <button type="button" title="Remove" aria-label="Remove blur" className="flex h-8 w-8 flex-none items-center justify-center rounded-lg text-koma-dim hover:bg-koma-hover hover:text-koma-fg" onClick={() => onPatch((current) => ({ ...current, effects: (current.effects ?? []).filter((_, at) => at !== index) }))}>
                <X size={13} />
              </button>
            </div>
          )
        })}
      </Section>
      {allText ? (
        <Section title="Text">
          <GeomField label="Size" value={fontSizeField.value} mixed={fontSizeField.mixed} tokens={typeTokens} bound={node.bindings?.fontSize} onBind={(token) => bindField('fontSize', token)} onChange={(fontSize) => {
            if (!Number.isFinite(fontSize) || fontSize <= 0 || fontSize === 13) applyType({}, ['fontSize'])
            else applyType({ fontSize })
          }} />
          <LabeledControl label="Weight">
            <KomaSelect
              aria-label="Weight"
              value={weightField.mixed ? '' : weightField.value}
              onChange={(event) => {
                const weight = event.target.value
                if (weight !== 'regular' && weight !== 'medium' && weight !== 'bold') return
                applyType(weight === 'regular' ? {} : { weight }, weight === 'regular' ? ['weight'] : [])
              }}
              className="h-7 w-full px-1.5 text-[12px]"
            >
              {weightField.mixed ? <option value="">Mixed</option> : null}
              <option value="regular">Regular</option>
              <option value="medium">Medium</option>
              <option value="bold">Bold</option>
            </KomaSelect>
          </LabeledControl>
          <label className="flex h-7 items-center gap-1 rounded border border-koma-border bg-koma-bg px-1.5">
            <span className="flex-none text-[11px] text-koma-dim">Font</span>
            <input
              aria-label="Font family"
                value={multi ? '' : fontQuery || node.fontFamily || ''}
              placeholder={textOf((item) => item.fontFamily ?? '').mixed ? 'Mixed' : 'UI font'}
              onChange={(event) => {
                const family = event.target.value
                setFontQuery(family)
                if (!family.trim()) applyType({}, ['fontFamily'])
                else if (/^[\w][\w\s,-]{0,80}$/.test(family)) applyType({ fontFamily: family })
              }}
              className="h-6 min-w-0 flex-1 bg-transparent text-[12px] text-koma-fg outline-none"
            />
          </label>
          {fontChoices.length ? (
            <div className="flex max-h-24 flex-col gap-0.5 overflow-y-auto">
              {fontChoices.map((name) => (
                <button key={name} type="button" title={name} onClick={() => { setFontQuery(''); applyType({ fontFamily: name }) }} className={`h-7 truncate rounded px-1.5 text-left text-[12px] ${node.fontFamily === name ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover'}`}>
                  {name}
                </button>
              ))}
            </div>
          ) : null}
          <FieldGroup label="Align">
            <div className="flex flex-wrap gap-0.5">
              <AlignButton label="Align left" pressed={!textAlignField.mixed && textAlignField.value === 'left'} onClick={() => setField({}, ['textAlign'])}><TextAlignStart size={14} /></AlignButton>
              <AlignButton label="Align center" pressed={!textAlignField.mixed && textAlignField.value === 'center'} onClick={() => setField({ textAlign: 'center' })}><TextAlignCenter size={14} /></AlignButton>
              <AlignButton label="Align right" pressed={!textAlignField.mixed && textAlignField.value === 'right'} onClick={() => setField({ textAlign: 'right' })}><TextAlignEnd size={14} /></AlignButton>
              <AlignButton label="Justify" caption="Justify" pressed={!textAlignField.mixed && textAlignField.value === 'justify'} onClick={() => setField({ textAlign: 'justify' })}><span className="text-[10px]">J</span></AlignButton>
            </div>
          </FieldGroup>
          <LabeledControl label="Vertical" wide>
            <KomaSelect
              aria-label="Vertical align"
              value={node.textVertical ?? 'center'}
              onChange={(event) => {
                const textVertical = event.target.value
                if (textVertical === 'center') setField({}, ['textVertical'])
                else if (textVertical === 'top' || textVertical === 'bottom') setField({ textVertical })
              }}
              className="h-7 w-full px-1.5 text-[12px]"
            >
              <option value="top">Top</option>
              <option value="center">Middle</option>
              <option value="bottom">Bottom</option>
            </KomaSelect>
          </LabeledControl>
          <LabeledControl label="Resize" wide>
            <KomaSelect
              aria-label="Text resize"
              value={node.textHug ?? 'fixed'}
              onChange={(event) => {
                const textHug = event.target.value
                if (textHug === 'fixed') setField({}, ['textHug'])
                else if (textHug === 'height' || textHug === 'width') setField({ textHug })
              }}
              className="h-7 w-full px-1.5 text-[12px]"
            >
              <option value="fixed">Fixed box</option>
              <option value="height">Hug height</option>
              <option value="width">Hug width</option>
            </KomaSelect>
          </LabeledControl>
          <div className="flex gap-0.5">
            <AlignButton label="Italic" pressed={!!node.italic} onClick={() => applyType(node.italic ? {} : { italic: true }, node.italic ? ['italic'] : [])}><span className="text-[10px] italic">I</span></AlignButton>
            <AlignButton label="Underline" pressed={!!node.underline} onClick={() => applyType(node.underline ? {} : { underline: true }, node.underline ? ['underline'] : [])}><span className="text-[10px] underline">U</span></AlignButton>
            <AlignButton label="Strike" pressed={!!node.strike} onClick={() => applyType(node.strike ? {} : { strike: true }, node.strike ? ['strike'] : [])}><span className="text-[10px] line-through">S</span></AlignButton>
          </div>
          <PaintRow
            label="Color"
            doc={doc}
            mixed={colorField.mixed}
            value={colorField.value}
            fallback="#c8d3f5"
            resolved={resolveRef(doc, style.color)}
            tokens={colorTokens}
            onChange={(next) => applyType(next && next !== 'none' ? { color: next, fill: next } : {}, next && next !== 'none' ? [] : ['color'])}
          />
          <div className="grid grid-cols-2 gap-1">
            <GeomField label="Line" ariaLabel="Line height" value={lineField.value} mixed={lineField.mixed} onChange={(lineHeight) => {
              if (!Number.isFinite(lineHeight) || lineHeight <= 0) setField({}, ['lineHeight'])
              else setField({ lineHeight })
            }} />
            <GeomField label="Track" ariaLabel="Letter spacing" value={trackingField.value} mixed={trackingField.mixed} onChange={(letterSpacing) => {
              if (!Number.isFinite(letterSpacing) || letterSpacing === 0) setField({}, ['letterSpacing'])
              else setField({ letterSpacing })
            }} />
          </div>
          <button
            type="button"
            className="flex h-8 w-full items-center justify-between rounded-lg bg-koma-bg px-2 text-[12px] text-koma-fg hover:bg-koma-hover"
            onClick={() => setInspectorPage({ kind: 'text', title: 'Text' })}
          >
            <span>More</span>
            <ChevronRight size={14} className="text-koma-dim" />
          </button>
        </Section>
      ) : null}
      {!multi && (node.kind === 'vector' || node.kind === 'line') ? (
        <Section
          title="SVG"
          action={(
            <AlignButton label="Add SVG attribute" onClick={() => {
              const svgAttrs = { ...(node.svgAttrs ?? {}) }
              let key = 'attr'
              let n = 1
              while (svgAttrs[key]) { n += 1; key = `attr${n}` }
              svgAttrs[key] = ''
              setField({ svgAttrs })
            }}><Plus size={14} /></AlignButton>
          )}
        >
          {Object.entries(node.svgAttrs ?? {}).map(([key, value]) => (
            <div key={key} className="flex items-center gap-1">
              <input
                aria-label="SVG attribute name"
                defaultValue={key}
                onBlur={(event) => {
                  const nextKey = event.target.value.trim()
                  const svgAttrs = { ...(node.svgAttrs ?? {}) }
                  delete svgAttrs[key]
                  if (nextKey) svgAttrs[nextKey] = value
                  setField(Object.keys(svgAttrs).length ? { svgAttrs } : {}, Object.keys(svgAttrs).length ? [] : ['svgAttrs'])
                }}
                className="h-7 w-20 flex-none rounded border border-koma-border bg-koma-bg px-1.5 text-[12px] text-koma-fg outline-none focus:border-koma-fg/40"
              />
              <input
                aria-label="SVG attribute value"
                value={value}
                onChange={(event) => setField({ svgAttrs: { ...(node.svgAttrs ?? {}), [key]: event.target.value } })}
                className="h-7 min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-1.5 text-[12px] text-koma-fg outline-none focus:border-koma-fg/40"
              />
              <button type="button" title="Remove" aria-label="Remove SVG attribute" className="flex h-8 w-8 items-center justify-center rounded-lg text-koma-dim hover:bg-koma-hover" onClick={() => {
                const svgAttrs = { ...(node.svgAttrs ?? {}) }
                delete svgAttrs[key]
                setField(Object.keys(svgAttrs).length ? { svgAttrs } : {}, Object.keys(svgAttrs).length ? [] : ['svgAttrs'])
              }}><X size={13} /></button>
            </div>
          ))}
        </Section>
      ) : null}
      {colorTokens.length || doc.tokens.some((token) => token.kind === 'space' || token.kind === 'radius') ? (
        <Section title="Tokens">
          <Choices
            label="Bind fill"
            value={node.bindings?.fill ?? ''}
            options={[{ value: '', label: 'None' }, ...colorTokens.map((token) => ({ value: token.name, label: token.name }))]}
            onChange={(name) => {
              const bindings = { ...(node.bindings ?? {}) }
              if (name) bindings.fill = name
              else delete bindings.fill
              onPatch((current) => {
                const next = { ...current, fill: name || current.fill }
                if (Object.keys(bindings).length) next.bindings = bindings
                else delete next.bindings
                return next
              })
            }}
          />
        </Section>
      ) : null}
      {!multi && node.kind === 'instance' ? (
        <InstanceOverrides
          doc={doc}
          node={node}
          onPatch={onPatch}
          onTypeFocus={onTypeFocus}
          onTypeBlur={onTypeBlur}
          onPickTarget={onOverrideTarget}
        />
      ) : null}
    </div>
    )}
    </InspectorPageContext.Provider>
    </DesignModeContext.Provider>
  )
}

function ShadowPage({
  doc,
  node,
  index,
  colorTokens,
  onPatch,
}: {
  doc: DesignDoc
  node: DesignNode
  index: number
  colorTokens: DesignToken[]
  onPatch: (fn: (node: DesignNode) => DesignNode) => void
}) {
  const effect = node.effects?.[index]
  if (!effect || (effect.kind !== 'drop-shadow' && effect.kind !== 'inner-shadow')) return null
  const patchEffect = (next: Partial<typeof effect>) => onPatch((current) => ({ ...current, effects: (current.effects ?? []).map((item, at) => (at === index ? { ...item, ...next } : item)) }))
  return (
    <div className="flex flex-col gap-2">
      <LabeledControl label="Type">
        <KomaSelect
          aria-label="Shadow type"
          value={effect.kind}
          onChange={(event) => {
            const kind = event.target.value
            if (kind === 'drop-shadow' || kind === 'inner-shadow') patchEffect({ kind })
          }}
          className="h-7 w-full px-1.5 text-[12px]"
        >
          <option value="drop-shadow">Drop shadow</option>
          <option value="inner-shadow">Inner shadow</option>
        </KomaSelect>
      </LabeledControl>
      <ColorRow
        doc={doc}
        paint={solidPaint(effect.color && effect.color !== 'none' ? effect.color : '#000000')}
        fallback="#000000"
        tokens={colorTokens}
        pageTitle="Shadow"
        allowImage={false}
        allowGradient={false}
        onChange={(paint) => patchEffect({ color: paint.color && paint.color !== 'none' ? paint.color : '#000000' })}
      />
      <div className="grid grid-cols-2 gap-1">
        <GeomField label="X" ariaLabel="Shadow X" value={effect.x ?? 0} onChange={(x) => patchEffect({ x })} />
        <GeomField label="Y" ariaLabel="Shadow Y" value={effect.y ?? 4} onChange={(y) => patchEffect({ y })} />
        <GeomField label="Blur" ariaLabel="Shadow blur" value={effect.blur ?? 4} onChange={(blur) => patchEffect({ blur })} />
        <GeomField label="Spread" ariaLabel="Shadow spread" value={effect.spread ?? 0} onChange={(spread) => patchEffect({ spread })} />
      </div>
    </div>
  )
}

function BlurPage({
  node,
  index,
  onPatch,
}: {
  node: DesignNode
  index: number
  onPatch: (fn: (node: DesignNode) => DesignNode) => void
}) {
  const effect = node.effects?.[index]
  if (!effect || (effect.kind !== 'layer-blur' && effect.kind !== 'background-blur')) return null
  const patchEffect = (next: Partial<typeof effect>) => onPatch((current) => ({ ...current, effects: (current.effects ?? []).map((item, at) => (at === index ? { ...item, ...next } : item)) }))
  return (
    <div className="flex flex-col gap-2">
      <LabeledControl label="Type">
        <KomaSelect
          aria-label="Blur type"
          value={effect.kind}
          onChange={(event) => {
            const kind = event.target.value
            if (kind === 'layer-blur' || kind === 'background-blur') patchEffect({ kind })
          }}
          className="h-7 w-full px-1.5 text-[12px]"
        >
          <option value="layer-blur">Layer</option>
          <option value="background-blur">Background</option>
        </KomaSelect>
      </LabeledControl>
      <GeomField label="Blur" ariaLabel="Blur" value={effect.blur ?? 4} onChange={(blur) => patchEffect({ blur })} />
    </div>
  )
}

function TextMorePage({
  node,
  setField,
  selectedText,
  applyType,
}: {
  node: DesignNode
  setField: (patch: Partial<DesignNode>, clear?: (keyof DesignNode)[]) => void
  selectedText: { start: number; end: number } | null
  applyType: (patch: Partial<DesignNode>, clear?: (keyof DesignNode)[]) => void
}) {
  return (
    <div className="flex flex-col gap-2">
      <Choices label="Case" value={node.textCase ?? 'original'} options={[{ value: 'original', label: 'As typed' }, { value: 'upper', label: 'UPPER' }, { value: 'lower', label: 'lower' }, { value: 'title', label: 'Title' }]} onChange={(textCase) => setField(textCase === 'original' ? {} : { textCase }, textCase === 'original' ? ['textCase'] : [])} />
      <div className="grid grid-cols-2 gap-1">
        <Choices
          label="Truncate"
          value={node.truncate ?? 'off'}
          options={[{ value: 'off', label: 'Off' }, { value: 'end', label: 'End' }]}
          onChange={(truncate) => setField(truncate === 'off' ? {} : { truncate }, truncate === 'off' ? ['truncate'] : [])}
        />
        <GeomField label="Lines" ariaLabel="Max lines" value={node.maxLines ?? 0} onChange={(maxLines) => setField(maxLines > 0 ? { maxLines: Math.round(maxLines) } : {}, maxLines > 0 ? [] : ['maxLines'])} />
      </div>
      {selectedText ? (
        <p className="truncate text-[11px] text-koma-dim" title={(node.text ?? '').slice(selectedText.start, selectedText.end)}>
          Selection: {(node.text ?? '').slice(selectedText.start, selectedText.end) || '…'}
        </p>
      ) : (
        <p className="text-[11px] text-koma-dim">Select text on the canvas to style a run</p>
      )}
      {(node.runs ?? []).map((run, index) => (
        <div key={`${run.start}-${index}`} className="flex items-center gap-1">
          <span className="min-w-0 flex-1 truncate rounded-lg bg-koma-bg px-2 py-1 text-[11px] text-koma-fg" title={(node.text ?? '').slice(run.start, run.end)}>
            {(node.text ?? '').slice(run.start, run.end) || '…'}
            {run.fontSize ? ` · ${run.fontSize}` : ''}
          </span>
          <button type="button" title="Remove run" aria-label="Remove run" className="flex h-8 w-8 items-center justify-center rounded-lg text-koma-dim hover:bg-koma-hover" onClick={() => {
            const runs = (node.runs ?? []).filter((_, at) => at !== index)
            setField(runs.length ? { runs } : {}, runs.length ? [] : ['runs'])
          }}><X size={12} /></button>
        </div>
      ))}
      {selectedText ? (
        <AlignButton label="Style selection" onClick={() => applyType({ fontSize: node.fontSize ?? 13 })}>
          <Plus size={14} />
        </AlignButton>
      ) : null}
    </div>
  )
}
