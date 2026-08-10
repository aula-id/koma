/**
 * ImportGraphScene — 3D force-directed graph for the import graph.
 * Uniform tiny dots, muted role colours, labels on focus/select/hover only.
 */

import {
  useRef,
  useMemo,
  useEffect,
  useCallback,
  useState,
} from 'react'
import { Canvas, useFrame, useThree, type ThreeEvent } from '@react-three/fiber'
import { OrbitControls, Text, Line } from '@react-three/drei'
import * as THREE from 'three'
import type { ImportGraphNode, ImportGraphEdge } from '../store/koma'

// ─── Theme ─────────────────────────────────────────────────────────────────

function readThemeColors(): { accent: string; dim: string } {
  if (typeof document === 'undefined') {
    return { accent: '#3b82f6', dim: '#6b7280' }
  }
  const s = getComputedStyle(document.documentElement)
  return {
    accent: s.getPropertyValue('--koma-accent').trim() || '#3b82f6',
    dim: s.getPropertyValue('--koma-dim').trim() || '#6b7280',
  }
}

function roleColor(role: string, accent: string): string {
  switch (role) {
    case 'Focus': return accent
    case 'Dependency': return '#6ba3b0'
    case 'Dependent': return '#b09070'
    default: return '#8896a4'
  }
}

// ─── Physics ───────────────────────────────────────────────────────────────

const REPULSION = 150
const ATTRACTION = 0.008
const DAMPING = 0.5
const CENTER_GRAVITY = 0.02
const CUTOFF_SQ = 900
const MAX_SPEED = 8
const CLICK_THRESHOLD = 5

// ─── Uniform node radius ──────────────────────────────────────────────────

const NODE_R = 0.12

function fileName(path: string): string {
  const parts = path.replace(/\\/g, '/').split('/')
  return parts[parts.length - 1] || path
}

// ─── Initial positions (fibonacci sphere) ──────────────────────────────────

function computeInitialPositions(
  nodes: ImportGraphNode[],
  _edges: ImportGraphEdge[],
): Map<string, [number, number, number]> {
  const positions = new Map<string, [number, number, number]>()
  const focusNode = nodes.find((n) => n.role === 'Focus')

  if (!focusNode) {
    const count = nodes.length
    if (count === 0) return positions
    const radius = Math.max(3, Math.sqrt(count) * 0.8)
    const goldenAngle = Math.PI * (3 - Math.sqrt(5))
    nodes.forEach((n, i) => {
      const y = 1 - (i / (count - 1 || 1)) * 2
      const ry = Math.sqrt(1 - y * y)
      const theta = goldenAngle * i
      positions.set(n.path, [
        Math.cos(theta) * ry * radius,
        y * radius,
        Math.sin(theta) * ry * radius,
      ])
    })
    return positions
  }

  positions.set(focusNode.path, [0, 0, 0])
  const deps = nodes.filter((n) => n.role === 'Dependency')
  const dependents = nodes.filter((n) => n.role === 'Dependent')
  const others = nodes.filter((n) => n.role === 'Overview')
  const goldenAngle = Math.PI * (3 - Math.sqrt(5))

  const depR = Math.max(3, Math.sqrt(deps.length) * 1.2)
  deps.forEach((n, i) => {
    const phi = Math.acos(1 - 2 * (i + 0.5) / (deps.length || 1))
    const theta = goldenAngle * i
    positions.set(n.path, [
      Math.sin(phi) * Math.cos(theta) * depR,
      Math.sin(phi) * Math.sin(theta) * depR * 0.6,
      Math.cos(phi) * depR + 2,
    ])
  })

  const depdR = Math.max(3, Math.sqrt(dependents.length) * 1.2)
  dependents.forEach((n, i) => {
    const phi = Math.acos(1 - 2 * (i + 0.5) / (dependents.length || 1))
    const theta = goldenAngle * i
    positions.set(n.path, [
      Math.sin(phi) * Math.cos(theta) * depdR,
      Math.sin(phi) * Math.sin(theta) * depdR * 0.6,
      -(Math.cos(phi) * depdR + 2),
    ])
  })

  const outerR = Math.max(depR, depdR) + 5
  others.forEach((n, i) => {
    const phi = Math.acos(1 - 2 * (i + 0.5) / (others.length || 1))
    const theta = goldenAngle * (deps.length + dependents.length + i)
    positions.set(n.path, [
      Math.sin(phi) * Math.cos(theta) * outerR,
      Math.sin(phi) * Math.sin(theta) * outerR,
      Math.cos(phi) * outerR,
    ])
  })

  return positions
}

// ─── Neighbor set ──────────────────────────────────────────────────────────

function computeNeighborSet(focusPath: string, edges: ImportGraphEdge[]): Set<string> {
  const s = new Set<string>()
  s.add(focusPath)
  for (const e of edges) {
    if (e.from === focusPath) s.add(e.to)
    if (e.to === focusPath) s.add(e.from)
  }
  return s
}

// ─── Inner scene props ─────────────────────────────────────────────────────

interface GraphSceneInnerProps {
  nodes: ImportGraphNode[]
  edges: ImportGraphEdge[]
  focus: string | null
  selectedPath: string | null
  onNodeClick: (path: string) => void
  onNodeSelect: (path: string) => void
  themeColors: { accent: string; dim: string }
}

// ─── Inner 3D scene ───────────────────────────────────────────────────────

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

  const [hoveredPath, setHoveredPath] = useState<string | null>(null)
  const hoveredRef = useRef<string | null>(null)
  hoveredRef.current = hoveredPath

  // Cursor management for hover
  useEffect(() => {
    if (hoveredPath) {
      document.body.style.cursor = 'pointer'
      return () => { document.body.style.cursor = '' }
    }
    document.body.style.cursor = ''
  }, [hoveredPath])

  const nodePaths = useMemo(() => nodes.map((n) => n.path), [nodes])
  const pathToIdx = useMemo(() => {
    const map = new Map<string, number>()
    nodes.forEach((n, i) => map.set(n.path, i))
    return map
  }, [nodes])

  const edgePairs = useMemo(() => {
    const pairs: [number, number][] = []
    for (const e of edges) {
      const si = pathToIdx.get(e.from)
      const ti = pathToIdx.get(e.to)
      if (si !== undefined && ti !== undefined) pairs.push([si, ti])
    }
    return pairs
  }, [edges, pathToIdx])

  const initialPositions = useMemo(
    () => computeInitialPositions(nodes, edges),
    [nodes, edges],
  )

  const posRef = useRef<Map<string, { x: number; y: number; z: number }>>(new Map())
  const velRef = useRef<Map<string, { x: number; y: number; z: number }>>(new Map())

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

  const meshRefs = useRef<Map<string, THREE.Mesh>>(new Map())

  const neighborSet = useMemo(() => {
    if (!focus) return null
    return computeNeighborSet(focus, edges)
  }, [focus, edges])

  // Camera animation
  const cameraTargetDest = useRef<THREE.Vector3 | null>(null)
  const cameraPosDest = useRef<THREE.Vector3 | null>(null)
  const prevFocusRef = useRef<string | null>(null)

  const zoomDistance = useMemo(
    () => 8 + Math.log2(Math.max(nodes.length, 1)) * 2,
    [nodes.length],
  )

  useEffect(() => {
    if (focus === prevFocusRef.current) return
    prevFocusRef.current = focus

    if (focus) {
      const p = posRef.current.get(focus)
      if (p && orbitRef.current) {
        const target = new THREE.Vector3(p.x, p.y, p.z)
        cameraTargetDest.current = target.clone()
        const dir = new THREE.Vector3()
          .subVectors(camera.position, (orbitRef.current as any).target)
        if (dir.lengthSq() < 0.001) dir.set(0, 0, 1)
        dir.normalize()
        cameraPosDest.current = target.clone().add(dir.multiplyScalar(zoomDistance))
      }
    } else {
      const dir = new THREE.Vector3()
        .subVectors(camera.position, (orbitRef.current as any)?.target ?? new THREE.Vector3())
      if (dir.lengthSq() < 0.001) dir.set(0, 0, 1)
      dir.normalize()
      cameraTargetDest.current = new THREE.Vector3(0, 0, 0)
      cameraPosDest.current = dir.multiplyScalar(zoomDistance * 1.5)
    }
  }, [focus, zoomDistance, camera])

  useEffect(() => {
    const c = orbitRef.current
    if (!c) return
    const onStart = () => {
      cameraTargetDest.current = null
      cameraPosDest.current = null
    }
    c.addEventListener('start', onStart)
    return () => c.removeEventListener('start', onStart)
  })

  // Click vs drag state
  const pointerStart = useRef<{ x: number; y: number } | null>(null)
  const dragIdxRef = useRef<string | null>(null)
  const lastClickTime = useRef<number>(0)
  const lastClickPath = useRef<string | null>(null)

  // ── Force simulation + per-frame updates ───────────────────────────────
  useFrame((_, delta) => {
    const dt = Math.min(delta, 0.05)
    const pos = posRef.current
    const vel = velRef.current
    const n = nodePaths.length
    const hasFocus = focus !== null
    const repulsion = hasFocus ? REPULSION : REPULSION * 1.3

    // Pairwise repulsion
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
        vi.x += fx; vi.y += fy; vi.z += fz
        vj.x -= fx; vj.y -= fy; vj.z -= fz
      }
    }

    // Edge attraction
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
      vsi.x += fx; vsi.y += fy; vsi.z += fz
      vti.x -= fx; vti.y -= fy; vti.z -= fz
    }

    // Gravity, damping, integrate
    for (let i = 0; i < n; i++) {
      const path = nodePaths[i]
      const p = pos.get(path)
      const v = vel.get(path)
      if (!p || !v) continue
      v.x -= p.x * CENTER_GRAVITY * dt
      v.y -= p.y * CENTER_GRAVITY * dt
      v.z -= p.z * CENTER_GRAVITY * dt
      v.x *= DAMPING; v.y *= DAMPING; v.z *= DAMPING
      const speed = Math.sqrt(v.x * v.x + v.y * v.y + v.z * v.z)
      if (speed > MAX_SPEED) {
        const s = MAX_SPEED / speed
        v.x *= s; v.y *= s; v.z *= s
      }
      p.x += v.x; p.y += v.y; p.z += v.z
      const mesh = meshRefs.current.get(path)
      if (mesh) mesh.position.set(p.x, p.y, p.z)
    }

    // Per-node material + scale updates
    const hov = hoveredRef.current
    if (hasFocus && neighborSet) {
      for (let i = 0; i < n; i++) {
        const path = nodePaths[i]
        const mesh = meshRefs.current.get(path)
        if (!mesh) continue
        const mat = mesh.material as THREE.MeshStandardMaterial
        const isNeighbor = neighborSet.has(path)
        mat.opacity = isNeighbor ? 1.0 : 0.1
        const role = nodes[i].role
        if (role === 'Focus') mesh.scale.setScalar(1.15)
        else if (path === selectedPath) mesh.scale.setScalar(1.1)
        else if (path === hov) mesh.scale.setScalar(1.08)
        else mesh.scale.setScalar(1)
      }
    } else {
      for (let i = 0; i < n; i++) {
        const path = nodePaths[i]
        const mesh = meshRefs.current.get(path)
        if (!mesh) continue
        const mat = mesh.material as THREE.MeshStandardMaterial
        mat.opacity = 1.0
        if (path === selectedPath) mesh.scale.setScalar(1.1)
        else if (path === hov) mesh.scale.setScalar(1.08)
        else mesh.scale.setScalar(1)
      }
    }
    prevFocusRef.current = focus

    // Camera lerp
    if (orbitRef.current) {
      if (cameraTargetDest.current) {
        const t = (orbitRef.current as any).target as THREE.Vector3
        t.lerp(cameraTargetDest.current, 0.08)
        if (t.distanceTo(cameraTargetDest.current) < 0.05) {
          t.copy(cameraTargetDest.current)
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

  // Click handlers
  const handlePointerDown = useCallback(
    (e: ThreeEvent<PointerEvent>, path: string) => {
      e.stopPropagation()
      pointerStart.current = { x: e.nativeEvent.clientX, y: e.nativeEvent.clientY }
      dragIdxRef.current = path
      if (orbitRef.current) orbitRef.current.enabled = false
    },
    [],
  )

  const handlePointerMove = useCallback((_e: ThreeEvent<PointerEvent>) => {}, [])

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
      if (dist > CLICK_THRESHOLD) return
      const now = Date.now()
      const timeSinceLast = now - lastClickTime.current
      const isSameNode = lastClickPath.current === path
      if (timeSinceLast < 400 && isSameNode) {
        onNodeClickRef.current(path)
        lastClickTime.current = 0
        lastClickPath.current = null
      } else {
        onNodeSelectRef.current(path)
        lastClickTime.current = now
        lastClickPath.current = path
      }
    },
    [],
  )

  const handleBgPointerUp = useCallback(() => {
    if (dragIdxRef.current !== null) {
      dragIdxRef.current = null
      if (orbitRef.current) orbitRef.current.enabled = true
    }
    pointerStart.current = null
  }, [])

  // Edge highlight set (focus + selection)
  const edgeLines = useMemo(() => {
    const highlighted = new Set<string>()
    if (focus) {
      for (const e of edges) {
        if (e.from === focus || e.to === focus) highlighted.add(`${e.from}->${e.to}`)
      }
    }
    if (selectedPath) {
      for (const e of edges) {
        if (e.from === selectedPath || e.to === selectedPath) highlighted.add(`${e.from}->${e.to}`)
      }
    }
    return edgePairs.map(([si, ti]) => {
      const key = `${nodePaths[si]}->${nodePaths[ti]}`
      return { si, ti, isHighlighted: highlighted.has(key) }
    })
  }, [edgePairs, edges, focus, selectedPath, nodePaths])

  const accentColor = themeColors.accent

  return (
    <group onPointerMove={handlePointerMove} onPointerUp={handleBgPointerUp}>
      <OrbitControls ref={orbitRef as any} enableDamping dampingFactor={0.1} makeDefault />

      {nodes.map((node) => {
        const color = roleColor(node.role, accentColor)
        const isSelected = node.path === selectedPath
        const isFocused = node.path === focus
        const isHovered = node.path === hoveredPath
        const showLabel = isFocused || isSelected || isHovered

        const emissive = isFocused
          ? color
          : isSelected ? '#e2e8f0'
          : isHovered ? color
          : '#000000'
        const emissiveIntensity = isFocused ? 0.25 : isSelected ? 0.15 : isHovered ? 0.12 : 0

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
            onPointerEnter={(e) => {
              e.stopPropagation()
              setHoveredPath(node.path)
            }}
            onPointerLeave={(e) => {
              e.stopPropagation()
              setHoveredPath((prev) => (prev === node.path ? null : prev))
            }}
          >
            <sphereGeometry args={[NODE_R, 10, 6]} />
            <meshStandardMaterial
              color={isSelected ? '#e2e8f0' : color}
              transparent
              opacity={1.0}
              emissive={emissive}
              emissiveIntensity={emissiveIntensity}
              roughness={0.8}
              metalness={0}
            />
            {showLabel && (
              <Text
                position={[0, NODE_R + 0.18, 0]}
                fontSize={isFocused ? 0.22 : 0.18}
                color={isFocused ? '#e8edf2' : isSelected ? '#e8edf2' : '#c8cdd4'}
                anchorX="center"
                anchorY="bottom"
                outlineWidth={0.018}
                outlineColor="#000000"
                outlineOpacity={0.45}
                fillOpacity={1}
                font={undefined}
              >
                {fileName(node.path)}
              </Text>
            )}
          </mesh>
        )
      })}

      {edgeLines.map(({ si, ti, isHighlighted }) => {
        const sp = posRef.current.get(nodePaths[si])
        const tp = posRef.current.get(nodePaths[ti])
        if (!sp || !tp) return null
        return (
          <Line
            key={`${nodePaths[si]}-${nodePaths[ti]}`}
            points={[[sp.x, sp.y, sp.z], [tp.x, tp.y, tp.z]]}
            color={isHighlighted ? accentColor : '#3a4555'}
            lineWidth={isHighlighted ? 1.0 : 0.5}
            transparent
            opacity={isHighlighted ? 0.35 : 0.08}
            segments
          />
        )
      })}

      <ambientLight intensity={0.7} />
      <pointLight position={[10, 10, 10]} intensity={0.4} />
      <pointLight position={[-10, -10, -10]} intensity={0.2} />
    </group>
  )
}

// ─── Public wrapper ────────────────────────────────────────────────────────

export type ImportGraphSceneProps = {
  nodes: ImportGraphNode[]
  edges: ImportGraphEdge[]
  focus: string | null
  selectedPath: string | null
  onNodeClick: (path: string) => void
  onNodeSelect: (path: string) => void
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

  useEffect(() => {
    const observer = new MutationObserver(() => setThemeColors(readThemeColors()))
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['class'],
    })
    return () => observer.disconnect()
  }, [])

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
