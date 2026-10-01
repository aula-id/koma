import { useState } from 'react'
import {
  AlignCenterHorizontal,
  AlignCenterVertical,
  AlignEndHorizontal,
  AlignEndVertical,
  AlignHorizontalSpaceBetween,
  AlignStartHorizontal,
  AlignStartVertical,
  AlignVerticalSpaceBetween,
  ArrowDown,
  ArrowRight,
  Blend,
  Component,
  Crop,
  FlipHorizontal2,
  FlipVertical2,
  Layers,
  MessageSquare,
  Move,
  Plus,
  RotateCcw,
  RotateCw,
  Square,
  TextAlignCenter,
  TextAlignEnd,
  TextAlignStart,
  WrapText,
  X,
} from 'lucide-react'
import { KomaSelect } from '../KomaSelect'
import {
  designChatText,
  designChatTitle,
  designLayerName,
  designQueryNode,
  mergeDesignOverride,
  appendNodePaint,
  nodeChrome,
  resolveRef,
  setNodeSolid,
  sharedValue,
  solidPaint,
  textStyle,
  type DesignAlignAxis,
  type DesignAlignEdge,
  type DesignDoc,
  type DesignNode,
} from '../../lib/design'
import { SHAPE_FILL } from './tabShared'
import { InstanceOverrides } from './DesignPropertyFields'
import {
  AlignButton,
  Choices,
  ColorRow,
  ConstraintWidget,
  GeomField,
  KindMark,
  FillEditor,
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
  onAddToChat,
  onAlign,
  onPatch,
  onType,
  onTypeFocus,
  onTypeBlur,
  onOverrideTarget,
  onPickImage,
  onStoreImage,
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
  onAddToChat?: () => void
  onAlign?: (axis: DesignAlignAxis, edge: DesignAlignEdge) => void
  onPatch: (fn: (node: DesignNode) => DesignNode) => void
  onType: (fn: (node: DesignNode) => DesignNode) => void
  onTypeFocus: () => void
  onTypeBlur: () => void
  onOverrideTarget?: (childId: string | null) => void
  onPickImage?: () => void
  onStoreImage?: (hash: string, bytes: Uint8Array, mime: string) => void
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
      const next: DesignNode = { ...current, ...patch }
      for (const key of clear) delete next[key]
      return next
    })
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
  const strokeWidthField = numberOf((item) => nodeChrome(item).strokeWidth)
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
  const setJustify = (justify: 'start' | 'center' | 'end' | 'space') => {
    setField(justify === 'start' ? {} : { justify }, justify === 'start' ? ['justify'] : [])
  }
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
  return (
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
      {onResetInstance ? (
        <button type="button" title="Reset overrides" aria-label="Reset overrides" onClick={onResetInstance} className="flex h-7 w-7 items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg">
          <RotateCcw size={14} />
        </button>
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
      <Section title="Position">
        <div className="grid grid-cols-2 gap-1">
          <GeomField label="X" value={xField.value} mixed={xField.mixed} onChange={(x) => setField({ x })} />
          <GeomField label="Y" value={yField.value} mixed={yField.mixed} onChange={(y) => setField({ y })} />
          <div className="flex min-w-0 gap-1">
            <div className="min-w-0 flex-1">
              <GeomField label="W" value={wField.value} mixed={wField.mixed} onChange={(w) => setField({ w: Math.max(1, w) }, ['wMode'])} />
            </div>
            {sizeModes ? (
              <SizeMode label="Width sizing" value={wModeField.value} mixed={wModeField.mixed} onChange={(mode) => setField(mode === 'fixed' ? {} : { wMode: mode }, mode === 'fixed' ? ['wMode'] : [])} />
            ) : null}
          </div>
          <div className="flex min-w-0 gap-1">
            <div className="min-w-0 flex-1">
              <GeomField label="H" value={hField.value} mixed={hField.mixed} onChange={(h) => setField({ h: Math.max(1, h) }, ['hMode'])} />
            </div>
            {sizeModes ? (
              <SizeMode label="Height sizing" value={hModeField.value} mixed={hModeField.mixed} onChange={(mode) => setField(mode === 'fixed' ? {} : { hMode: mode }, mode === 'fixed' ? ['hMode'] : [])} />
            ) : null}
          </div>
        </div>
        <div className="flex items-center gap-1">
          <div className="min-w-0 flex-1">
            <GeomField label="R" suffix="°" value={rotationField.value} mixed={rotationField.mixed} onChange={(rotation) => {
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
          <AlignButton label="Rotate 90 degrees" onClick={() => onPatch((current) => {
            const wrapped = (((current.rotation ?? 0) + 90) % 360 + 360) % 360
            const next = { ...current }
            if (wrapped) next.rotation = wrapped
            else delete next.rotation
            return next
          })}><RotateCw size={14} /></AlignButton>
        </div>
        {sizeModes || allFrames ? (
          <div className="grid grid-cols-2 gap-1">
            <GeomField label="Min" ariaLabel="Minimum width" value={minWField.value} mixed={minWField.mixed} onChange={(minW) => setField(minW > 0 ? { minW } : {}, minW > 0 ? [] : ['minW'])} />
            <GeomField label="Max" ariaLabel="Maximum width" value={maxWField.value} mixed={maxWField.mixed} onChange={(maxW) => setField(maxW > 0 ? { maxW } : {}, maxW > 0 ? [] : ['maxW'])} />
            <GeomField label="Min" ariaLabel="Minimum height" value={minHField.value} mixed={minHField.mixed} onChange={(minH) => setField(minH > 0 ? { minH } : {}, minH > 0 ? [] : ['minH'])} />
            <GeomField label="Max" ariaLabel="Maximum height" value={maxHField.value} mixed={maxHField.mixed} onChange={(maxH) => setField(maxH > 0 ? { maxH } : {}, maxH > 0 ? [] : ['maxH'])} />
          </div>
        ) : null}
      </Section>
      {allFrames || hasParent ? (
      <Section title="Layout">
      {allFrames ? (
        <div className="flex gap-0.5">
          <AlignButton label="Free" pressed={!layoutField.mixed && layoutField.value === 'free'} onClick={() => setField({}, ['layout'])}><Square size={14} /></AlignButton>
          <AlignButton label="Row" pressed={!layoutField.mixed && layoutField.value === 'row'} onClick={() => setField({ layout: 'row' })}><ArrowRight size={14} /></AlignButton>
          <AlignButton label="Column" pressed={!layoutField.mixed && layoutField.value === 'column'} onClick={() => setField({ layout: 'column' })}><ArrowDown size={14} /></AlignButton>
          <AlignButton label="Grid" pressed={!layoutField.mixed && layoutField.value === 'grid'} onClick={() => setField({ layout: 'grid', gridColumns: [{ size: 'fr', count: 2 }], gridRows: [{ size: 'fr', count: 2 }] })}><Square size={14} /></AlignButton>
        </div>
      ) : null}
      {!layoutField.mixed && layoutField.value === 'grid' ? (
        <div className="grid grid-cols-2 gap-1">
          <GeomField label="Cols" ariaLabel="Grid columns" value={node.gridColumns?.[0]?.count ?? 2} onChange={(count) => setField({ layout: 'grid', gridColumns: [{ size: 'fr', count: Math.max(1, Math.round(count)) }] })} />
          <GeomField label="Rows" ariaLabel="Grid rows" value={node.gridRows?.[0]?.count ?? 2} onChange={(count) => setField({ layout: 'grid', gridRows: [{ size: 'fr', count: Math.max(1, Math.round(count)) }] })} />
        </div>
      ) : null}
      {flows ? (
        <>
          <div className="grid grid-cols-2 gap-1">
            <GeomField label="Gap" value={gapField.value} mixed={gapField.mixed} onChange={(gap) => setField(gap > 0 ? { gap } : {}, gap > 0 ? [] : ['gap'])} />
            <GeomField label="Pad" value={padField.value} mixed={padField.mixed} onChange={(pad) => setField(pad > 0 ? { pad } : {}, pad > 0 ? ['padTop', 'padRight', 'padBottom', 'padLeft'] : ['pad', 'padTop', 'padRight', 'padBottom', 'padLeft'])} />
          </div>
          <div className="grid grid-cols-4 gap-1">
            <GeomField label="T" ariaLabel="Padding top" value={padTopField.value} mixed={padTopField.mixed} onChange={(value) => setSide('padTop', value)} />
            <GeomField label="R" ariaLabel="Padding right" value={padRightField.value} mixed={padRightField.mixed} onChange={(value) => setSide('padRight', value)} />
            <GeomField label="B" ariaLabel="Padding bottom" value={padBottomField.value} mixed={padBottomField.mixed} onChange={(value) => setSide('padBottom', value)} />
            <GeomField label="L" ariaLabel="Padding left" value={padLeftField.value} mixed={padLeftField.mixed} onChange={(value) => setSide('padLeft', value)} />
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <div className="flex gap-0.5">
              {columnLayout ? (
                <>
                  <AlignButton label="Align top" pressed={!justifyField.mixed && justifyField.value === 'start'} onClick={() => setJustify('start')}><AlignStartHorizontal size={14} /></AlignButton>
                  <AlignButton label="Align middle" pressed={!justifyField.mixed && justifyField.value === 'center'} onClick={() => setJustify('center')}><AlignCenterHorizontal size={14} /></AlignButton>
                  <AlignButton label="Align bottom" pressed={!justifyField.mixed && justifyField.value === 'end'} onClick={() => setJustify('end')}><AlignEndHorizontal size={14} /></AlignButton>
                  <AlignButton label="Space between" pressed={!justifyField.mixed && justifyField.value === 'space'} onClick={() => setJustify('space')}><AlignVerticalSpaceBetween size={14} /></AlignButton>
                </>
              ) : (
                <>
                  <AlignButton label="Align left" pressed={!justifyField.mixed && justifyField.value === 'start'} onClick={() => setJustify('start')}><AlignStartVertical size={14} /></AlignButton>
                  <AlignButton label="Align center" pressed={!justifyField.mixed && justifyField.value === 'center'} onClick={() => setJustify('center')}><AlignCenterVertical size={14} /></AlignButton>
                  <AlignButton label="Align right" pressed={!justifyField.mixed && justifyField.value === 'end'} onClick={() => setJustify('end')}><AlignEndVertical size={14} /></AlignButton>
                  <AlignButton label="Space between" pressed={!justifyField.mixed && justifyField.value === 'space'} onClick={() => setJustify('space')}><AlignHorizontalSpaceBetween size={14} /></AlignButton>
                </>
              )}
            </div>
            <div className="flex gap-0.5">
              {columnLayout ? (
                <>
                  <AlignButton label="Align left" pressed={!alignField.mixed && alignField.value === 'start'} onClick={() => setCross('start')}><AlignStartVertical size={14} /></AlignButton>
                  <AlignButton label="Align center" pressed={!alignField.mixed && alignField.value === 'center'} onClick={() => setCross('center')}><AlignCenterVertical size={14} /></AlignButton>
                  <AlignButton label="Align right" pressed={!alignField.mixed && alignField.value === 'end'} onClick={() => setCross('end')}><AlignEndVertical size={14} /></AlignButton>
                  <AlignButton label="Stretch" pressed={!alignField.mixed && alignField.value === 'stretch'} onClick={() => setCross('stretch')}><span className="text-[11px] font-semibold">↔</span></AlignButton>
                </>
              ) : (
                <>
                  <AlignButton label="Align top" pressed={!alignField.mixed && alignField.value === 'start'} onClick={() => setCross('start')}><AlignStartHorizontal size={14} /></AlignButton>
                  <AlignButton label="Align middle" pressed={!alignField.mixed && alignField.value === 'center'} onClick={() => setCross('center')}><AlignCenterHorizontal size={14} /></AlignButton>
                  <AlignButton label="Align bottom" pressed={!alignField.mixed && alignField.value === 'end'} onClick={() => setCross('end')}><AlignEndHorizontal size={14} /></AlignButton>
                  <AlignButton label="Stretch" pressed={!alignField.mixed && alignField.value === 'stretch'} onClick={() => setCross('stretch')}><span className="text-[11px] font-semibold">↕</span></AlignButton>
                </>
              )}
            </div>
          </div>
        </>
      ) : null}
      <div className="flex items-center gap-0.5">
      {flows ? (
        <AlignButton
          label={textOf((item) => (item.wrap ? 'on' : 'off')).mixed ? 'Wrap · Mixed' : nodes.every((item) => item.wrap) ? 'Wrap on' : 'Wrap off'}
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
      </Section>
      ) : null}
      <Section title="Appearance">
        <div className={`grid gap-1 ${showRadius ? 'grid-cols-2' : 'grid-cols-1'}`}>
          <GeomField label="Op" ariaLabel="Opacity" suffix="%" value={opacityField.value} mixed={opacityField.mixed} onChange={(value) => {
            const opacity = Math.min(100, Math.max(0, value)) / 100
            setField(opacity < 1 ? { opacity } : {}, opacity < 1 ? [] : ['opacity'])
          }} />
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
        </div>
        {showRadius ? (
          <div className="grid grid-cols-4 gap-1">
            <GeomField label="TL" ariaLabel="Top left radius" value={cornerField((item) => resolvedCorner(item.radiusTL ?? item.radius)).value} mixed={cornerField((item) => resolvedCorner(item.radiusTL ?? item.radius)).mixed} onChange={(value) => setCorner('radiusTL', value)} />
            <GeomField label="TR" ariaLabel="Top right radius" value={cornerField((item) => resolvedCorner(item.radiusTR ?? item.radius)).value} mixed={cornerField((item) => resolvedCorner(item.radiusTR ?? item.radius)).mixed} onChange={(value) => setCorner('radiusTR', value)} />
            <GeomField label="BR" ariaLabel="Bottom right radius" value={cornerField((item) => resolvedCorner(item.radiusBR ?? item.radius)).value} mixed={cornerField((item) => resolvedCorner(item.radiusBR ?? item.radius)).mixed} onChange={(value) => setCorner('radiusBR', value)} />
            <GeomField label="BL" ariaLabel="Bottom left radius" value={cornerField((item) => resolvedCorner(item.radiusBL ?? item.radius)).value} mixed={cornerField((item) => resolvedCorner(item.radiusBL ?? item.radius)).mixed} onChange={(value) => setCorner('radiusBL', value)} />
          </div>
        ) : null}
        <div className="flex items-center gap-1">
          <span title="Blend" className="flex h-8 w-8 flex-none items-center justify-center text-koma-dim"><Blend size={13} /></span>
          <KomaSelect
            aria-label="Blend"
            title="Blend"
            value={node.blend ?? 'normal'}
            onChange={(event) => {
              const blend = event.target.value
              setField(blend === 'normal' ? {} : { blend }, blend === 'normal' ? ['blend'] : [])
            }}
            className="h-8 min-w-0 flex-1 px-1.5 text-[12px]"
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
          </KomaSelect>
          <AlignButton label="Use as mask" pressed={!!node.mask} onClick={() => setField(node.mask ? {} : { mask: true }, node.mask ? ['mask', 'maskType'] : [])}>
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
            const color = current.kind === 'frame' ? '#ffffff' : SHAPE_FILL
            return appendNodePaint(current, 'fill', solidPaint(color), solidPaint(color))
          })}>
            <Plus size={14} />
          </AlignButton>
        )}
      >
        {multi ? (
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
        ) : (
          <FillEditor label="Fill" doc={doc} node={node} field="fill" tokens={colorTokens} onChange={(next) => onPatch(() => next)} onPickImage={onPickImage} onStoreImage={onStoreImage} />
        )}
      </Section>
      <Section
        title="Stroke"
        action={(
          <AlignButton label="Add stroke" onClick={() => onPatch((current) => appendNodePaint(current, 'stroke', solidPaint('#1c1c1c'), solidPaint('#1c1c1c')))}>
            <Plus size={14} />
          </AlignButton>
        )}
      >
        {multi ? (
          <PaintRow
            label="Stroke"
            doc={doc}
            mixed={strokeField.mixed}
            value={strokeField.value}
            fallback="#8b93b8"
            resolved={resolveRef(doc, chrome.stroke)}
            tokens={colorTokens}
            weight={strokeWidthField}
            onWeight={(strokeWidth) => setField(strokeWidth > 0 && strokeWidth !== 1 ? { strokeWidth } : {}, strokeWidth > 0 && strokeWidth !== 1 ? [] : ['strokeWidth'])}
            onChange={(next) => paintChange('stroke', next)}
          />
        ) : (
          <FillEditor label="Stroke" doc={doc} node={node} field="stroke" tokens={colorTokens} onChange={(next) => onPatch(() => next)} onStoreImage={onStoreImage} />
        )}
        <div className="grid grid-cols-[1fr_1fr] gap-1">
          <GeomField label="W" ariaLabel="Width" value={strokeWidthField.value} mixed={strokeWidthField.mixed} onChange={(strokeWidth) => setField(strokeWidth > 0 && strokeWidth !== 1 ? { strokeWidth } : {}, strokeWidth > 0 && strokeWidth !== 1 ? [] : ['strokeWidth'])} />
          <KomaSelect
            aria-label="Alignment"
            value={node.strokeAlign ?? 'center'}
            onChange={(event) => {
              const strokeAlign = event.target.value
              if (strokeAlign !== 'inside' && strokeAlign !== 'center' && strokeAlign !== 'outside') return
              setField(strokeAlign === 'center' ? {} : { strokeAlign }, strokeAlign === 'center' ? ['strokeAlign'] : [])
            }}
            className="h-8 px-1.5 text-[12px]"
          >
            <option value="inside">Inside</option>
            <option value="center">Center</option>
            <option value="outside">Outside</option>
          </KomaSelect>
          <KomaSelect
            aria-label="Style"
            value={!node.strokeDash?.length ? 'solid' : (node.strokeDash[0] ?? 4) <= 1 ? 'dotted' : 'dashed'}
            onChange={(event) => {
              const style = event.target.value
              if (style === 'solid') setField({}, ['strokeDash'])
              else if (style === 'dotted') setField({ strokeDash: [1, 3] })
              else setField({ strokeDash: [4, 4] })
            }}
            className="h-8 px-1.5 text-[12px]"
          >
            <option value="solid">Solid</option>
            <option value="dashed">Dashed</option>
            <option value="dotted">Dotted</option>
          </KomaSelect>
          <KomaSelect
            aria-label="Cap"
            value={node.strokeCap ?? 'none'}
            onChange={(event) => {
              const strokeCap = event.target.value
              if (strokeCap !== 'none' && strokeCap !== 'round' && strokeCap !== 'square') return
              setField(strokeCap === 'none' ? {} : { strokeCap }, strokeCap === 'none' ? ['strokeCap'] : [])
            }}
            className="h-8 px-1.5 text-[12px]"
          >
            <option value="none">None</option>
            <option value="round">Round</option>
            <option value="square">Square</option>
          </KomaSelect>
        </div>
      </Section>
      {hasParent && !sizeModes ? (
        <Section title="Constraints">
          <div className="flex items-start gap-1">
            <ConstraintWidget
              horizontal={node.constraintH ?? 'start'}
              vertical={node.constraintV ?? 'start'}
              onHorizontal={(constraintH) => setField(constraintH === 'start' ? {} : { constraintH }, constraintH === 'start' ? ['constraintH'] : [])}
              onVertical={(constraintV) => setField(constraintV === 'start' ? {} : { constraintV }, constraintV === 'start' ? ['constraintV'] : [])}
            />
            <div className="flex min-w-0 flex-1 flex-col gap-1">
              <KomaSelect
                aria-label="Horizontal"
                title="Horizontal"
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
                <option value="stretch">Left & right</option>
                <option value="scale">Scale</option>
              </KomaSelect>
              <KomaSelect
                aria-label="Vertical"
                title="Vertical"
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
                <option value="stretch">Top & bottom</option>
                <option value="scale">Scale</option>
              </KomaSelect>
            </div>
          </div>
        </Section>
      ) : null}
      <Section
        title="Effects"
        action={(
          <AlignButton label="Add drop shadow" onClick={() => onPatch((current) => ({ ...current, effects: [...(current.effects ?? []), { kind: 'drop-shadow', x: 4, y: 4, blur: 4, spread: 0, color: '#000000' }] }))}>
            <Plus size={14} />
          </AlignButton>
        )}
      >
        {(node.effects ?? []).map((effect, index) => {
          const patchEffect = (next: Partial<typeof effect>) => onPatch((current) => ({ ...current, effects: (current.effects ?? []).map((item, at) => (at === index ? { ...item, ...next } : item)) }))
          const removeEffect = () => onPatch((current) => ({ ...current, effects: (current.effects ?? []).filter((_, at) => at !== index) }))
          if (effect.kind === 'drop-shadow' || effect.kind === 'inner-shadow') {
            return (
              <div key={`${effect.kind}-${index}`} className="flex flex-col gap-1">
                <div className="flex items-center gap-1">
                  <KomaSelect
                    aria-label="Shadow type"
                    value={effect.kind}
                    onChange={(event) => {
                      const kind = event.target.value
                      if (kind === 'drop-shadow' || kind === 'inner-shadow') patchEffect({ kind })
                    }}
                    className="h-8 min-w-0 flex-1 px-1.5 text-[12px]"
                  >
                    <option value="drop-shadow">Drop shadow</option>
                    <option value="inner-shadow">Inner shadow</option>
                  </KomaSelect>
                  <button type="button" title="Remove" aria-label="Remove shadow" className="flex h-8 w-8 flex-none items-center justify-center rounded-lg text-koma-dim hover:bg-koma-hover hover:text-koma-fg" onClick={removeEffect}>
                    <X size={13} />
                  </button>
                </div>
                <ColorRow
                  doc={doc}
                  paint={solidPaint(effect.color && effect.color !== 'none' ? effect.color : '#000000')}
                  fallback="#000000"
                  tokens={colorTokens}
                  allowImage={false}
                  allowGradient={false}
                  onChange={(paint) => patchEffect({ color: paint.color && paint.color !== 'none' ? paint.color : '#000000' })}
                />
                <div className="grid grid-cols-4 gap-1">
                  <GeomField label="X" ariaLabel="Shadow X" value={effect.x ?? 0} onChange={(x) => patchEffect({ x })} />
                  <GeomField label="Y" ariaLabel="Shadow Y" value={effect.y ?? 4} onChange={(y) => patchEffect({ y })} />
                  <GeomField label="B" ariaLabel="Shadow blur" value={effect.blur ?? 4} onChange={(blur) => patchEffect({ blur })} />
                  <GeomField label="S" ariaLabel="Shadow spread" value={effect.spread ?? 0} onChange={(spread) => patchEffect({ spread })} />
                </div>
              </div>
            )
          }
          return (
            <div key={`${effect.kind}-${index}`} className="flex items-center gap-1">
              <KomaSelect
                aria-label="Blur type"
                value={effect.kind}
                onChange={(event) => {
                  const kind = event.target.value
                  if (kind === 'layer-blur' || kind === 'background-blur') patchEffect({ kind })
                }}
                className="h-8 min-w-0 flex-1 px-1.5 text-[12px]"
              >
                <option value="layer-blur">Layer blur</option>
                <option value="background-blur">Background blur</option>
              </KomaSelect>
              <div className="w-16 flex-none">
                <GeomField label="B" ariaLabel="Blur" value={effect.blur ?? 4} onChange={(blur) => patchEffect({ blur })} />
              </div>
              <button type="button" title="Remove" aria-label="Remove blur" className="flex h-8 w-8 flex-none items-center justify-center rounded-lg text-koma-dim hover:bg-koma-hover hover:text-koma-fg" onClick={removeEffect}>
                <X size={13} />
              </button>
            </div>
          )
        })}
      </Section>
      {allText ? (
        <Section title="Text">
          <div className="flex items-center gap-1">
            <div className="min-w-0 flex-1">
              <GeomField label="Size" value={fontSizeField.value} mixed={fontSizeField.mixed} onChange={(fontSize) => {
                if (!Number.isFinite(fontSize) || fontSize <= 0 || fontSize === 13) setField({}, ['fontSize'])
                else setField({ fontSize })
              }} />
            </div>
            <KomaSelect
              aria-label="Weight"
              value={weightField.mixed ? '' : weightField.value}
              onChange={(event) => {
                const weight = event.target.value
                if (weight !== 'regular' && weight !== 'medium' && weight !== 'bold') return
                setField(weight === 'regular' ? {} : { weight }, weight === 'regular' ? ['weight'] : [])
              }}
              className="h-7 flex-none px-1 text-[12px]"
            >
              {weightField.mixed ? <option value="">Mixed</option> : null}
              <option value="regular">Regular</option>
              <option value="medium">Medium</option>
              <option value="bold">Bold</option>
            </KomaSelect>
          </div>
          <label className="flex h-7 items-center gap-1 rounded border border-koma-border bg-koma-bg px-1.5">
            <span className="flex-none text-[11px] text-koma-dim">Font</span>
            <input
              aria-label="Font family"
              value={multi ? '' : node.fontFamily ?? ''}
              placeholder={textOf((item) => item.fontFamily ?? '').mixed ? 'Mixed' : 'UI font'}
              onChange={(event) => {
                const family = event.target.value
                if (!family.trim()) setField({}, ['fontFamily'])
                else if (/^[\w][\w\s,-]{0,80}$/.test(family)) setField({ fontFamily: family })
              }}
              className="h-6 min-w-0 flex-1 bg-transparent text-[12px] text-koma-fg outline-none"
            />
          </label>
          <div className="flex gap-0.5">
            <AlignButton label="Align left" pressed={!textAlignField.mixed && textAlignField.value === 'left'} onClick={() => setField({}, ['textAlign'])}><TextAlignStart size={14} /></AlignButton>
            <AlignButton label="Align center" pressed={!textAlignField.mixed && textAlignField.value === 'center'} onClick={() => setField({ textAlign: 'center' })}><TextAlignCenter size={14} /></AlignButton>
            <AlignButton label="Align right" pressed={!textAlignField.mixed && textAlignField.value === 'right'} onClick={() => setField({ textAlign: 'right' })}><TextAlignEnd size={14} /></AlignButton>
            <AlignButton label="Align top" pressed={!textOf((item) => item.textVertical ?? 'center').mixed && (node.textVertical ?? 'center') === 'top'} onClick={() => setField({ textVertical: 'top' })}><span className="text-[10px]">T</span></AlignButton>
            <AlignButton label="Align middle" pressed={!textOf((item) => item.textVertical ?? 'center').mixed && (node.textVertical ?? 'center') === 'center'} onClick={() => setField({}, ['textVertical'])}><span className="text-[10px]">M</span></AlignButton>
            <AlignButton label="Align bottom" pressed={!textOf((item) => item.textVertical ?? 'center').mixed && (node.textVertical ?? 'center') === 'bottom'} onClick={() => setField({ textVertical: 'bottom' })}><span className="text-[10px]">B</span></AlignButton>
          </div>
          <div className="flex gap-0.5">
            <AlignButton label="Fixed text box" pressed={!textOf((item) => item.textHug ?? 'fixed').mixed && !node.textHug} onClick={() => setField({}, ['textHug'])}><span className="text-[10px]">Fix</span></AlignButton>
            <AlignButton label="Hug height" pressed={!textOf((item) => item.textHug ?? 'fixed').mixed && node.textHug === 'height'} onClick={() => setField({ textHug: 'height' })}><span className="text-[10px]">H</span></AlignButton>
            <AlignButton label="Hug width" pressed={!textOf((item) => item.textHug ?? 'fixed').mixed && node.textHug === 'width'} onClick={() => setField({ textHug: 'width' })}><span className="text-[10px]">W</span></AlignButton>
          </div>
          <div className="flex gap-0.5">
            <AlignButton label="Italic" pressed={!!node.italic} onClick={() => setField(node.italic ? {} : { italic: true }, node.italic ? ['italic'] : [])}><span className="text-[10px] italic">I</span></AlignButton>
            <AlignButton label="Underline" pressed={!!node.underline} onClick={() => setField(node.underline ? {} : { underline: true }, node.underline ? ['underline'] : [])}><span className="text-[10px] underline">U</span></AlignButton>
            <AlignButton label="Strike" pressed={!!node.strike} onClick={() => setField(node.strike ? {} : { strike: true }, node.strike ? ['strike'] : [])}><span className="text-[10px] line-through">S</span></AlignButton>
          </div>
          <Choices label="Case" value={node.textCase ?? 'original'} options={[{ value: 'original', label: 'Aa' }, { value: 'upper', label: 'AA' }, { value: 'lower', label: 'aa' }, { value: 'title', label: 'Aa' }]} onChange={(textCase) => setField(textCase === 'original' ? {} : { textCase }, textCase === 'original' ? ['textCase'] : [])} />
          <PaintRow
            label="Color"
            doc={doc}
            mixed={colorField.mixed}
            value={colorField.value}
            fallback="#c8d3f5"
            resolved={resolveRef(doc, style.color)}
            tokens={colorTokens}
            onChange={(next) => setField(next && next !== 'none' ? { color: next } : {}, next && next !== 'none' ? [] : ['color'])}
          />
          <div className="grid grid-cols-2 gap-1">
            <GeomField label="Line" value={lineField.value} mixed={lineField.mixed} onChange={(lineHeight) => {
              if (!Number.isFinite(lineHeight) || lineHeight <= 0) setField({}, ['lineHeight'])
              else setField({ lineHeight })
            }} />
            <GeomField label="Spacing" value={trackingField.value} mixed={trackingField.mixed} onChange={(letterSpacing) => {
              if (!Number.isFinite(letterSpacing) || letterSpacing === 0) setField({}, ['letterSpacing'])
              else setField({ letterSpacing })
            }} />
          </div>
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
  )
}
