/**
 * ImportGraphFlow — React Flow rendering of the import graph.
 *
 * Controlled arrays, custom memoized file-card nodes, directed arrow markers.
 * No external layout library — positions come from importGraphFlowLayout.
 * nodesDraggable false, nodesConnectable false, zoomOnDoubleClick false.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  ReactFlow,
  Background,
  Controls,
  Handle,
  MarkerType,
  Position,
  ReactFlowProvider,
  useReactFlow,
  type Node,
  type Edge,
  type NodeTypes,
} from '@xyflow/react'
import type { ImportGraphNode, ImportGraphEdge } from '../store/koma'
import {
  computeFlowLayout,
  fileName,
  parentPath,
  CARD_W,
  CARD_H,
  FOCUS_CARD_H,
  type FlowLayoutResult,
} from '../lib/importGraphFlowLayout'

// ─── Theme colors ──────────────────────────────────────────────────────────

function readThemeColors(): { accent: string; dim: string; bg: string; fg: string } {
  if (typeof document === 'undefined') {
    return { accent: '#3b82f6', dim: '#6b7280', bg: '#0b0e14', fg: '#c8d3f5' }
  }
  const s = getComputedStyle(document.documentElement)
  return {
    accent: s.getPropertyValue('--koma-accent').trim() || '#3b82f6',
    dim: s.getPropertyValue('--koma-dim').trim() || '#6b7280',
    bg: s.getPropertyValue('--koma-bg').trim() || '#0b0e14',
    fg: s.getPropertyValue('--koma-fg').trim() || '#c8d3f5',
  }
}

const ROLE_COLORS: Record<string, string> = {
  Focus: '#3b82f6',
  Dependency: '#6ba3b0',
  Dependent: '#b09070',
  Overview: '#8896a4',
}

// ─── Custom file card node ─────────────────────────────────────────────────

interface FileCardData {
  node: ImportGraphNode
  isFocused: boolean
  isSelected: boolean
  accent: string
  fg: string
  dim: string
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function FileCard({ data }: { data: FileCardData }) {
  const { node, isFocused, isSelected, accent, fg, dim } = data
  const name = fileName(node.path)
  const parent = parentPath(node.path)
  const roleColor = ROLE_COLORS[node.role] ?? ROLE_COLORS.Overview
  const h = isFocused ? FOCUS_CARD_H : CARD_H

  const bg = isFocused
    ? 'rgba(59,130,246,0.08)'
    : isSelected
      ? 'rgba(255,255,255,0.04)'
      : 'rgba(255,255,255,0.02)'
  const border = isFocused
    ? accent
    : isSelected
      ? 'rgba(255,255,255,0.15)'
      : 'rgba(255,255,255,0.06)'

  return (
    <div
      className="rounded-md border px-3 py-2"
      style={{
        width: CARD_W,
        height: h,
        background: bg,
        borderColor: border,
        borderWidth: 1,
        borderStyle: 'solid',
      }}
    >
      {/* Handles — invisible, non-interactive, for edge routing only.
          Four handles (source+target × left+right) so reciprocal edges
          route to distinct opposite-side handles and never overlap. */}
      <Handle
        type="target"
        position={Position.Left}
        id="left-target"
        style={{ opacity: 0, width: 1, height: 1, pointerEvents: 'none' }}
        isConnectable={false}
      />
      <Handle
        type="source"
        position={Position.Left}
        id="left-source"
        style={{ opacity: 0, width: 1, height: 1, pointerEvents: 'none' }}
        isConnectable={false}
      />
      <Handle
        type="target"
        position={Position.Right}
        id="right-target"
        style={{ opacity: 0, width: 1, height: 1, pointerEvents: 'none' }}
        isConnectable={false}
      />
      <Handle
        type="source"
        position={Position.Right}
        id="right-source"
        style={{ opacity: 0, width: 1, height: 1, pointerEvents: 'none' }}
        isConnectable={false}
      />
      {/* Filename — dominant */}
      <div
        className="truncate font-medium leading-tight"
        style={{ color: isFocused ? accent : fg, fontSize: isFocused ? 14 : 13 }}
        title={node.path}
      >
        {name}
      </div>
      {/* Parent path + language */}
      {(parent || node.language) && (
        <div className="mt-0.5 flex items-center gap-1.5 overflow-hidden">
          {parent && (
            <span className="min-w-0 truncate text-[10px]" style={{ color: dim }}>
              {parent}
            </span>
          )}
          {node.language && (
            <span
              className="flex-none rounded px-1 py-px text-[9px]"
              style={{ background: 'rgba(255,255,255,0.06)', color: dim }}
            >
              {node.language}
            </span>
          )}
        </div>
      )}
      {/* Role + degree badges */}
      <div className="mt-1 flex items-center gap-1.5">
        <span
          className="rounded px-1 py-px text-[9px] font-medium"
          style={{
            background: roleColor + '20',
            color: roleColor,
          }}
        >
          {node.role}
        </span>
        <span className="text-[9px]" style={{ color: dim }}>
          {node.inDegree}↑ {node.outDegree}↓
        </span>
      </div>
    </div>
  )
}

// ─── Inner ReactFlow wrapper (must be inside ReactFlowProvider) ────────────

interface FlowInnerProps {
  graphNodes: ImportGraphNode[]
  graphEdges: ImportGraphEdge[]
  focus: string | null
  selectedPath: string | null
  onNodeClick: (path: string) => void
  onNodeDoubleClick: (path: string) => void
}

function FlowInner({
  graphNodes,
  graphEdges,
  focus,
  selectedPath,
  onNodeClick,
  onNodeDoubleClick,
}: FlowInnerProps) {
  const rfInstance = useReactFlow()

  // Theme reactivity: track CSS var mutations so colors stay current.
  const [theme, setTheme] = useState(readThemeColors)
  useEffect(() => {
    const observer = new MutationObserver(() => setTheme(readThemeColors()))
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['class', 'style'],
    })
    return () => observer.disconnect()
  }, [])

  // Compute layout
  const layout: FlowLayoutResult = useMemo(
    () => computeFlowLayout(graphNodes, graphEdges, focus),
    [graphNodes, graphEdges, focus],
  )

  // Build React Flow nodes
  const nodes: Node[] = useMemo(() => {
    return layout.positions.map((p) => {
      const gNode = graphNodes.find((n) => n.path === p.id)
      if (!gNode) return null
      const isFocused = p.id === focus
      const isSelected = p.id === selectedPath
      const cardH = isFocused ? FOCUS_CARD_H : CARD_H
      return {
        id: p.id,
        position: { x: p.x, y: p.y },
        data: {
          node: gNode,
          isFocused,
          isSelected,
          accent: theme.accent,
          fg: theme.fg,
          dim: theme.dim,
        },
        style: {
          width: CARD_W,
          height: cardH,
          padding: 0,
        },
        draggable: false,
        selectable: true,
      }
    }).filter(Boolean) as Node[]
  }, [layout, graphNodes, focus, selectedPath, theme])

  // Build React Flow edges with handle-aware routing
  const edges: Edge[] = useMemo(() => {
    return layout.edges.map((e, idx) => {
      const isDepToFocus = e.target === focus
      const isFocusToDep = e.source === focus
      const markerEnd = {
        id: `arrow-${e.id}-${idx}`,
        type: MarkerType.ArrowClosed,
        width: 16,
        height: 16,
        color: isDepToFocus ? ROLE_COLORS.Dependent : isFocusToDep ? ROLE_COLORS.Dependency : '#5a6a78',
      }
      // For reciprocal pairs (both directions), offset one slightly
      const isReciprocal = layout.edges.some(
        (other) => other.id !== e.id && other.source === e.target && other.target === e.source,
      )
      const style: Record<string, unknown> = {
        stroke: isDepToFocus ? ROLE_COLORS.Dependent : isFocusToDep ? ROLE_COLORS.Dependency : '#5a6a78',
        strokeWidth: 1.5,
        opacity: 0.6,
      }
      if (isReciprocal && isDepToFocus) {
        style.strokeDasharray = '6,3'
      }
      return {
        id: e.id,
        source: e.source,
        target: e.target,
        sourceHandle: e.sourceHandle,
        targetHandle: e.targetHandle,
        type: 'smoothstep',
        markerEnd,
        style,
        zIndex: 0,
      }
    })
  }, [layout, focus])

  // Stable graph signature for fitView — full signature including node IDs
  // and edge IDs so selection-only changes don't trigger a refit.
  const graphSignature = useMemo(
    () => `${focus ?? 'none'}:${nodes.map((n) => n.id).sort().join(',')}:${edges.map((e) => e.id).sort().join(',')}`,
    [focus, nodes, edges],
  )

  // Re-fit after actual graph content change (not selection-only changes).
  const prevGraphSignatureRef = useRef(graphSignature)
  useEffect(() => {
    if (graphSignature !== prevGraphSignatureRef.current) {
      prevGraphSignatureRef.current = graphSignature
      if (nodes.length > 0) {
        // Small delay to let React Flow measure
        const timer = setTimeout(() => {
          rfInstance.fitView({ padding: 0.15, duration: 200 })
        }, 50)
        return () => clearTimeout(timer)
      }
    }
  }, [graphSignature, nodes.length, rfInstance])

  const handleNodeClick = useCallback(
    (_event: React.MouseEvent, node: { id: string }) => {
      onNodeClick(node.id)
    },
    [onNodeClick],
  )

  const handleNodeDoubleClick = useCallback(
    (_event: React.MouseEvent, node: { id: string }) => {
      onNodeDoubleClick(node.id)
    },
    [onNodeDoubleClick],
  )

  const nodeTypes: NodeTypes = useMemo(
    () => ({
      fileCard: FileCard,
    }),
    [],
  )

  // Wrap each node with the custom type
  const typedNodes: Node[] = useMemo(
    () => nodes.map((n) => ({ ...n, type: 'fileCard' })),
    [nodes],
  )

  if (nodes.length === 0) return null

  return (
    <ReactFlow
      nodes={typedNodes}
      edges={edges}
      nodeTypes={nodeTypes}
      onNodeClick={handleNodeClick}
      onNodeDoubleClick={handleNodeDoubleClick}
      nodesDraggable={false}
      nodesConnectable={false}
      zoomOnDoubleClick={false}
      fitView
      fitViewOptions={{ padding: 0.15 }}
      defaultEdgeOptions={{ type: 'smoothstep' }}
      proOptions={{ hideAttribution: true }}
    >
      <Background color="rgba(255,255,255,0.03)" gap={24} />
      <Controls
        showInteractive={false}
        position="bottom-left"
        style={{ margin: 8 }}
      />
    </ReactFlow>
  )
}

// ─── Public wrapper ────────────────────────────────────────────────────────

export interface ImportGraphFlowProps {
  nodes: ImportGraphNode[]
  edges: ImportGraphEdge[]
  focus: string | null
  selectedPath: string | null
  onNodeClick: (path: string) => void
  onNodeDoubleClick: (path: string) => void
}

export function ImportGraphFlow({
  nodes,
  edges,
  focus,
  selectedPath,
  onNodeClick,
  onNodeDoubleClick,
}: ImportGraphFlowProps) {
  if (nodes.length === 0) return null

  return (
    <ReactFlowProvider>
      <div style={{ width: '100%', height: '100%' }}>
        <FlowInner
          graphNodes={nodes}
          graphEdges={edges}
          focus={focus}
          selectedPath={selectedPath}
          onNodeClick={onNodeClick}
          onNodeDoubleClick={onNodeDoubleClick}
        />
      </div>
    </ReactFlowProvider>
  )
}
