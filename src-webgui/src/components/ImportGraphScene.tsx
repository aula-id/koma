/**
 * ImportGraphScene — 3D force-directed graph visualization for the import graph.
 *
 * Replaces the SVG-based 2D layout with an interactive Three.js scene.
 * Nodes are spheres with text labels; edges are lines. A custom N-body force
 * simulation runs every frame inside `useFrame`. Camera animates to the focus
 * node when it changes.
 *
 * References: OpenGraph GraphScene.tsx + layout.ts patterns.
 */

import {
  useRef,
  useMemo,
  useEffect,
  useCallback,
  useState,
  type RefObject,
} from 'react'
import { Canvas, useFrame, useThree, type ThreeEvent } from '@react-three/fiber'
import { OrbitControls, Text, Line } from '@react-three/drei'
import * as THREE from 'three'
import type { ImportGraphNode, ImportGraphEdge } from '../store/koma'

// ─── Theme helpers ──────────────────────────────────────────────────────────

function readThemeColors(): { accent: string; dim: string } {
  if (typeof document === 'undefined') {
    return { accent: '#3b82f6', dim: '#6b7280' }
  }
  const style = getComputedStyle(document.documentElement)
  return {
    accent: style.getPropertyValue('--koma-accent').trim() || '#3b82f6',
    dim: style.getPropertyValue('--koma-dim').trim() || '#6b7280',
  }
}

// ─── Role → color ───────────────────────────────────────────────────────────

function roleColor(role: string, accent: string, dim: string): string {
  switch (role) {
    case 'Focus':
      return accent
    case 'Dependency':
      return '#22c55e'
    case 'Dependent':
      return '#f97316'
    default:
      return dim
  }
}

// ─── Physics constants ──────────────────────────────────────────────────────

const REPULSION = 150
const ATTRACTION = 0.008
const DAMPING = 0.5
const CENTER_GRAVITY = 0.02
const CUTOFF_SQ = 900
const MAX_SPEED = 8
const CLICK_THRESHOLD = 5

// ─── Node radius by degree ─────────────────────────────────────────────────

function nodeRadius(node: ImportGraphNode): number {
  const total = node.inDegree + node.outDegree
  return 0.3 + Math.log2(1 + total) * 0.15
}

// ─── Short label from full path ─────────────────────────────────────────────

function shortLabel(path: string): string {
  const parts = path.replace(/\\/g, '/').split('/')
  return parts[parts.length - 1] || path
}

// ─── Fibonacci-sphere initial positions ─────────────────────────────────────

function computeInitialPositions(
  nodes: ImportGraphNode[],
  _edges: ImportGraphEdge[],
): Map<string, [number, number, number]> {
  const positions = new Map<string, [number, number, number]>()

  const focusNode = nodes.find((n) => n.role === 'Focus')

  if (!focusNode) {
    // Overview: Fibonacci sphere for all nodes
    const count = nodes.length
    if (count === 0) return positions
    const radius = Math.max(3, Math.sqrt(count) * 0.8)
    const goldenAngle = Math.PI * (3 - Math.sqrt(5))
    nodes.forEach((n, i) => {
      const y = 1 - (i / (count - 1 || 1)) * 2
      const radiusAtY = Math.sqrt(1 - y * y)
      const theta = goldenAngle * i
      positions.set(n.path, [
        Math.cos(theta) * radiusAtY * radius,
        y * radius,
        Math.sin(theta) * radiusAtY * radius,
      ])
    })
    return positions
  }

  // Place focus at center
  positions.set(focusNode.path, [0, 0, 0])

  const deps = nodes.filter((n) => n.role === 'Dependency')
  const dependents = nodes.filter((n) => n.role === 'Dependent')
  const others = nodes.filter((n) => n.role === 'Overview')

  const goldenAngle = Math.PI * (3 - Math.sqrt(5))

  // Dependencies on front hemisphere (z > 0)
  const depRadius = Math.max(3, Math.sqrt(deps.length) * 1.2)
  deps.forEach((n, i) => {
    const phi = Math.acos(1 - 2 * (i + 0.5) / (deps.length || 1))
    const theta = goldenAngle * i
    positions.set(n.path, [
      Math.sin(phi) * Math.cos(theta) * depRadius,
      Math.sin(phi) * Math.sin(theta) * depRadius * 0.6,
      Math.cos(phi) * depRadius + 2,
    ])
  })

  // Dependents on back hemisphere (z < 0)
  const depdRadius = Math.max(3, Math.sqrt(dependents.length) * 1.2)
  dependents.forEach((n, i) => {
    const phi = Math.acos(1 - 2 * (i + 0.5) / (dependents.length || 1))
    const theta = goldenAngle * i
    positions.set(n.path, [
      Math.sin(phi) * Math.cos(theta) * depdRadius,
      Math.sin(phi) * Math.sin(theta) * depdRadius * 0.6,
      -(Math.cos(phi) * depdRadius + 2),
    ])
  })

  // Overview nodes on outer shell
  const outerRadius = Math.max(depRadius, depdRadius) + 5
  others.forEach((n, i) => {
    const phi = Math.acos(1 - 2 * (i + 0.5) / (others.length || 1))
    const theta = goldenAngle * (deps.length + dependents.length + i)
    positions.set(n.path, [
      Math.sin(phi) * Math.cos(theta) * outerRadius,
      Math.sin(phi) * Math.sin(theta) * outerRadius,
      Math.cos(phi) * outerRadius,
    ])
  })

  return positions
}

// ─── Neighbor set computation ───────────────────────────────────────────────

function computeNeighborSet(
  focusPath: string,
  edges: ImportGraphEdge[],
): Set<string> {
  const neighbors = new Set<string>()
  neighbors.add(focusPath)
  for (const e of edges) {
    if (e.from === focusPath) neighbors.add(e.to)
    if (e.to === focusPath) neighbors.add(e.from)
  }
  return neighbors
}

// ─── Scene props (inner component runs inside Canvas) ───────────────────────

interface GraphSceneInnerProps {
  nodes: ImportGraphNode[]
  edges: ImportGraphEdge[]
  focus: string | null
  selectedPath: string | null
  onNodeClick: (path: string) => void
  onNodeSelect: (path: string) => void
  themeColors: { accent: string; dim: string }
}

// ─── Inner 3D scene ─────────────────────────────────────────────────────────

function GraphSceneInner({
  nodes,
  edges,
  focus,
  selectedPath,
  onNodeClick,
  onNodeSelect,
  themeColors,
}: GraphSceneInnerProps) {
  const { camera } = useThree()
  const orbitRef = useRef<any>(null)
  const onNodeClickRef = useRef(onNodeClick)
  const onNodeSelectRef = useRef(onNodeSelect)
  onNodeClickRef.current = onNodeClick
  onNodeSelectRef.current = onNodeSelect

  const themeRef = useRef(themeColors)
  themeRef.current = themeColors

  // ── Node index maps ──────────────────────────────────────────────────────
  const nodePaths = useMemo(() => nodes.map((n) => n.path), [nodes])
  const pathToIdx = useMemo(() => {
    const map = new Map<string, number>()
    nodes.forEach((n, i) => map.set(n.path, i))
    return map
  }, [nodes])

  // ── Edge pairs (resolved to indices) ─────────────────────────────────────
  const edgePairs = useMemo(() => {
    const pairs: [number, number][] = []
    for (const e of edges) {
      const si = pathToIdx.get(e.from)
      const ti = pathToIdx.get(e.to)
      if (si !== undefined && ti !== undefined) pairs.push([si, ti])
    }
    return pairs
  }, [edges, pathToIdx])

  // ── Initial positions ────────────────────────────────────────────────────
  const initialPositions = useMemo(
    () => computeInitialPositions(nodes, edges),
    [nodes, edges],
  )

  // ── Physics state: positions + velocities ────────────────────────────────
  const posRef = useRef<Map<string, { x: number; y: number; z: number }>>(
    new Map(),
  )
  const velRef = useRef<Map<string, { x: number; y: number; z: number }>>(
    new Map(),
  )

  // Reset physics when nodes change
  useEffect(() => {
    const pos = new Map<string, { x: number; y: number; z: number }>()
    const vel = new Map<string, { x: number; y: number; z: number }>()
    for (const [path, [x, y, z]] of initialPositions) {
      pos.set(path, { x, y, z })
      vel.set(path, { x: 0, y: 0, z: 0 })
    }
    posRef.current = pos
    velRef.current = vel
  }, [initialPositions])

  // ── Refs for mesh positions (updated in useFrame) ────────────────────────
  const meshRefs = useRef<Map<string, THREE.Mesh>>(new Map())

  // ── Neighbor set for focus dimming ───────────────────────────────────────
  const neighborSet = useMemo(() => {
    if (!focus) return null
    return computeNeighborSet(focus, edges)
  }, [focus, edges])

  // ── Camera animation ─────────────────────────────────────────────────────
  const cameraTargetDest = useRef<THREE.Vector3 | null>(null)
  const cameraPosDest = useRef<THREE.Vector3 | null>(null)
  const prevFocusRef = useRef<string | null>(null)

  const zoomDistance = useMemo(() => {
    return 8 + Math.log2(Math.max(nodes.length, 1)) * 2
  }, [nodes.length])

  // Animate camera when focus changes
  useEffect(() => {
    if (focus === prevFocusRef.current) return
    prevFocusRef.current = focus

    if (focus) {
      const p = posRef.current.get(focus)
      if (p && orbitRef.current) {
        const target = new THREE.Vector3(p.x, p.y, p.z)
        cameraTargetDest.current = target.clone()
        // Approach from current direction
        const dir = new THREE.Vector3()
          .subVectors(camera.position, (orbitRef.current as any).target)
        if (dir.lengthSq() < 0.001) dir.set(0, 0, 1)
        dir.normalize()
        cameraPosDest.current = target.clone().add(dir.multiplyScalar(zoomDistance))
      }
    } else {
      // No focus: zoom out toward origin
      const dir = new THREE.Vector3()
        .subVectors(camera.position, (orbitRef.current as any)?.target ?? new THREE.Vector3())
      if (dir.lengthSq() < 0.001) dir.set(0, 0, 1)
      dir.normalize()
      cameraTargetDest.current = new THREE.Vector3(0, 0, 0)
      cameraPosDest.current = dir.multiplyScalar(zoomDistance * 1.5)
    }
  }, [focus, zoomDistance, camera])

  // Cancel camera animation on orbit start
  useEffect(() => {
    const controls = orbitRef.current
    if (!controls) return
    const onControlStart = () => {
      cameraTargetDest.current = null
      cameraPosDest.current = null
    }
    controls.addEventListener('start', onControlStart)
    return () => controls.removeEventListener('start', onControlStart)
  })

  // ── Pointer state for click vs drag ──────────────────────────────────────
  const pointerStart = useRef<{ x: number; y: number } | null>(null)
  const dragIdxRef = useRef<string | null>(null)
  const lastClickTime = useRef<number>(0)
  const lastClickPath = useRef<string | null>(null)

  // ── Force simulation + camera lerp in useFrame ───────────────────────────
  useFrame((_, delta) => {
    const dt = Math.min(delta, 0.05)
    const pos = posRef.current
    const vel = velRef.current
    const n = nodePaths.length
    const hasFocus = focus !== null
    const repulsion = hasFocus ? REPULSION : REPULSION * 1.3

    // 1. Pairwise repulsion
    for (let i = 0; i < n; i++) {
      const pi = pos.get(nodePaths[i])
      if (!pi) continue
      for (let j = i + 1; j < n; j++) {
        const pj = pos.get(nodePaths[j])
        if (!pj) continue
        const dx = pi.x - pj.x
        const dy = pi.y - pj.y
        const dz = pi.z - pj.z
        const distSq = dx * dx + dy * dy + dz * dz
        if (distSq > CUTOFF_SQ || distSq < 0.0001) continue
        const dist = Math.sqrt(distSq) + 0.1
        const force = (repulsion / (dist * dist)) * dt
        const fx = (dx / dist) * force
        const fy = (dy / dist) * force
        const fz = (dz / dist) * force
        const vi = vel.get(nodePaths[i])!
        const vj = vel.get(nodePaths[j])!
        vi.x += fx
        vi.y += fy
        vi.z += fz
        vj.x -= fx
        vj.y -= fy
        vj.z -= fz
      }
    }

    // 2. Edge attraction (spring)
    for (const [si, ti] of edgePairs) {
      const sp = pos.get(nodePaths[si])
      const tp = pos.get(nodePaths[ti])
      if (!sp || !tp) continue
      const dx = tp.x - sp.x
      const dy = tp.y - sp.y
      const dz = tp.z - sp.z
      const fx = dx * ATTRACTION * dt
      const fy = dy * ATTRACTION * dt
      const fz = dz * ATTRACTION * dt
      const vsi = vel.get(nodePaths[si])!
      const vti = vel.get(nodePaths[ti])!
      vsi.x += fx
      vsi.y += fy
      vsi.z += fz
      vti.x -= fx
      vti.y -= fy
      vti.z -= fz
    }

    // 3. Center gravity + damping + speed cap + apply velocity
    for (let i = 0; i < n; i++) {
      const path = nodePaths[i]
      const p = pos.get(path)
      const v = vel.get(path)
      if (!p || !v) continue

      // Center gravity
      v.x -= p.x * CENTER_GRAVITY * dt
      v.y -= p.y * CENTER_GRAVITY * dt
      v.z -= p.z * CENTER_GRAVITY * dt

      // Damping
      v.x *= DAMPING
      v.y *= DAMPING
      v.z *= DAMPING

      // Speed cap
      const speed = Math.sqrt(v.x * v.x + v.y * v.y + v.z * v.z)
      if (speed > MAX_SPEED) {
        const scale = MAX_SPEED / speed
        v.x *= scale
        v.y *= scale
        v.z *= scale
      }

      // Integrate
      p.x += v.x
      p.y += v.y
      p.z += v.z

      // Update mesh position
      const mesh = meshRefs.current.get(path)
      if (mesh) {
        mesh.position.set(p.x, p.y, p.z)
      }
    }

    // 4. Focus dimming
    const accentColor = themeRef.current.accent
    const dimColor = themeRef.current.dim
    if (hasFocus && neighborSet) {
      for (let i = 0; i < n; i++) {
        const path = nodePaths[i]
        const mesh = meshRefs.current.get(path)
        if (!mesh) continue
        const mat = mesh.material as THREE.MeshStandardMaterial
        const isNeighbor = neighborSet.has(path)
        mat.opacity = isNeighbor ? 1.0 : 0.3
        // Make focus node slightly bigger
        const node = nodes[i]
        if (node.role === 'Focus') {
          const r = nodeRadius(node) * 1.3
          mesh.scale.setScalar(r / nodeRadius(node))
        }
      }
    } else if (focus === null && prevFocusRef.current !== null) {
      // Transitioned to unfocused — reset opacity
      for (let i = 0; i < n; i++) {
        const path = nodePaths[i]
        const mesh = meshRefs.current.get(path)
        if (!mesh) continue
        const mat = mesh.material as THREE.MeshStandardMaterial
        mat.opacity = 1.0
        mesh.scale.setScalar(1)
      }
    }
    prevFocusRef.current = focus

    // 5. Camera animation
    if (orbitRef.current) {
      if (cameraTargetDest.current) {
        const target = (orbitRef.current as any).target as THREE.Vector3
        target.lerp(cameraTargetDest.current, 0.08)
        if (target.distanceTo(cameraTargetDest.current) < 0.05) {
          target.copy(cameraTargetDest.current)
          cameraTargetDest.current = null
        }
      }
      if (cameraPosDest.current) {
        camera.position.lerp(cameraPosDest.current, 0.08)
        if (camera.position.distanceTo(cameraPosDest.current) < 0.05) {
          camera.position.copy(cameraPosDest.current)
          cameraPosDest.current = null
        }
      }
    }
  })

  // ── Click handling ───────────────────────────────────────────────────────

  const handlePointerDown = useCallback(
    (e: ThreeEvent<PointerEvent>, path: string) => {
      e.stopPropagation()
      pointerStart.current = { x: e.nativeEvent.clientX, y: e.nativeEvent.clientY }
      dragIdxRef.current = path
      if (orbitRef.current) orbitRef.current.enabled = false
    },
    [],
  )

  const handlePointerMove = useCallback((_e: ThreeEvent<PointerEvent>) => {
    // Drag orbit is handled by OrbitControls when re-enabled
  }, [])

  const handlePointerUp = useCallback(
    (e: ThreeEvent<PointerEvent>) => {
      if (dragIdxRef.current === null) return
      const path = dragIdxRef.current
      dragIdxRef.current = null

      if (orbitRef.current) orbitRef.current.enabled = true

      if (!pointerStart.current) return
      const dx = e.nativeEvent.clientX - pointerStart.current.x
      const dy = e.nativeEvent.clientY - pointerStart.current.y
      const dist = Math.sqrt(dx * dx + dy * dy)
      pointerStart.current = null

      if (dist > CLICK_THRESHOLD) return // was a drag, not a click

      const now = Date.now()
      const timeSinceLast = now - lastClickTime.current
      const isSameNode = lastClickPath.current === path

      if (timeSinceLast < 400 && isSameNode) {
        // Double-click → chain navigation
        onNodeClickRef.current(path)
        lastClickTime.current = 0
        lastClickPath.current = null
      } else {
        // Single click → select for detail pane
        onNodeSelectRef.current(path)
        lastClickTime.current = now
        lastClickPath.current = path
      }
    },
    [],
  )

  // ── Pointer up on background (cancel drag) ───────────────────────────────
  const handleBgPointerUp = useCallback(() => {
    if (dragIdxRef.current !== null) {
      dragIdxRef.current = null
      if (orbitRef.current) orbitRef.current.enabled = true
    }
    pointerStart.current = null
  }, [])

  // ── Edge data for rendering ──────────────────────────────────────────────
  const edgeLines = useMemo(() => {
    const focusNeighborEdges = new Set<string>()
    if (focus) {
      for (const e of edges) {
        if (e.from === focus || e.to === focus) {
          focusNeighborEdges.add(`${e.from}->${e.to}`)
        }
      }
    }
    return edgePairs.map(([si, ti]) => {
      const key = `${nodePaths[si]}->${nodePaths[ti]}`
      const isHighlighted = focusNeighborEdges.has(key)
      return { si, ti, isHighlighted }
    })
  }, [edgePairs, edges, focus, nodePaths])

  const accentColor = themeColors.accent

  // ── Render ───────────────────────────────────────────────────────────────

  return (
    <group
      onPointerMove={handlePointerMove}
      onPointerUp={handleBgPointerUp}
    >
      <OrbitControls
        ref={orbitRef as any}
        enableDamping
        dampingFactor={0.1}
        makeDefault
      />

      {/* Node spheres */}
      {nodes.map((node) => {
        const r = nodeRadius(node)
        const color = roleColor(node.role, accentColor, themeColors.dim)
        const isSelected = node.path === selectedPath
        const isFocused = node.path === focus

        return (
          <mesh
            key={node.path}
            ref={(el) => {
              if (el) meshRefs.current.set(node.path, el)
              else meshRefs.current.delete(node.path)
            }}
            position={[
              posRef.current.get(node.path)?.x ?? 0,
              posRef.current.get(node.path)?.y ?? 0,
              posRef.current.get(node.path)?.z ?? 0,
            ]}
            onPointerDown={(e) => handlePointerDown(e, node.path)}
            onPointerUp={(e) => handlePointerUp(e)}
          >
            <sphereGeometry args={[r, 16, 10]} />
            <meshStandardMaterial
              color={isSelected ? '#ffffff' : color}
              transparent
              opacity={1.0}
              emissive={isFocused ? color : '#000000'}
              emissiveIntensity={isFocused ? 0.4 : 0}
              roughness={0.4}
              metalness={0.1}
            />

            {/* Text label */}
            <Text
              position={[0, r + 0.3, 0]}
              fontSize={isFocused ? 0.4 : 0.3}
              color="#ffffff"
              anchorX="center"
              anchorY="bottom"
              outlineWidth={0.03}
              outlineColor="#000000"
              outlineOpacity={0.7}
              fillOpacity={1}
              font={undefined}
            >
              {shortLabel(node.path)}
            </Text>
          </mesh>
        )
      })}

      {/* Edges */}
      {edgeLines.map(({ si, ti, isHighlighted }) => {
        const sp = posRef.current.get(nodePaths[si])
        const tp = posRef.current.get(nodePaths[ti])
        if (!sp || !tp) return null
        return (
          <Line
            key={`${nodePaths[si]}-${nodePaths[ti]}`}
            points={[
              [sp.x, sp.y, sp.z],
              [tp.x, tp.y, tp.z],
            ]}
            color={isHighlighted ? accentColor : '#94a3b8'}
            lineWidth={isHighlighted ? 2 : 1}
            transparent
            opacity={isHighlighted ? 0.7 : 0.3}
            segments
          />
        )
      })}

      {/* Lights */}
      <ambientLight intensity={0.5} />
      <pointLight position={[10, 10, 10]} intensity={0.8} />
      <pointLight position={[-10, -10, -10]} intensity={0.3} />
    </group>
  )
}

// ─── Public wrapper with Canvas ─────────────────────────────────────────────

export type ImportGraphSceneProps = {
  nodes: ImportGraphNode[]
  edges: ImportGraphEdge[]
  focus: string | null
  selectedPath: string | null
  onNodeClick: (path: string) => void // chain navigation
  onNodeSelect: (path: string) => void // select for detail pane (single click)
}

export function ImportGraphScene({
  nodes,
  edges,
  focus,
  selectedPath,
  onNodeClick,
  onNodeSelect,
}: ImportGraphSceneProps) {
  const [themeColors, setThemeColors] = useState(readThemeColors)

  // Re-read theme on class changes (dark/light toggle)
  useEffect(() => {
    const observer = new MutationObserver(() => setThemeColors(readThemeColors()))
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['class'],
    })
    return () => observer.disconnect()
  }, [])

  // Empty state
  if (nodes.length === 0) {
    return (
      <div className="flex h-full w-full items-center justify-center text-[12px] text-koma-dim opacity-60">
        No graph data
      </div>
    )
  }

  return (
    <div style={{ width: '100%', height: '100%' }}>
      <Canvas
        camera={{ position: [0, 0, 10], fov: 50 }}
        gl={{ antialias: true, alpha: true }}
        style={{ background: 'transparent' }}
      >
        <GraphSceneInner
          nodes={nodes}
          edges={edges}
          focus={focus}
          selectedPath={selectedPath}
          onNodeClick={onNodeClick}
          onNodeSelect={onNodeSelect}
          themeColors={themeColors}
        />
      </Canvas>
    </div>
  )
}
