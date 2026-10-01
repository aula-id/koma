import type { DesignVector } from '../types'

/** Decode a Figma vectorNetworkBlob into a Koma vector. */
export function decodeVectorNetworkBlob(data: Uint8Array): DesignVector {
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength)
  let offset = 0
  const vertexCount = view.getUint32(offset, true)
  offset += 4
  const segmentCount = view.getUint32(offset, true)
  offset += 4
  const regionCount = view.getUint32(offset, true)
  offset += 4

  const vertices: DesignVector['vertices'] = []
  for (let index = 0; index < vertexCount; index++) {
    offset += 4
    const x = view.getFloat32(offset, true)
    offset += 4
    const y = view.getFloat32(offset, true)
    offset += 4
    vertices.push({ x, y })
  }

  const segments: DesignVector['segments'] = []
  for (let index = 0; index < segmentCount; index++) {
    offset += 4
    const start = view.getUint32(offset, true)
    offset += 4
    const tangentStartX = view.getFloat32(offset, true)
    offset += 4
    const tangentStartY = view.getFloat32(offset, true)
    offset += 4
    const end = view.getUint32(offset, true)
    offset += 4
    const tangentEndX = view.getFloat32(offset, true)
    offset += 4
    const tangentEndY = view.getFloat32(offset, true)
    offset += 4
    segments.push({
      start,
      end,
      tangentStart: { x: tangentStartX, y: tangentStartY },
      tangentEnd: { x: tangentEndX, y: tangentEndY },
    })
  }

  const regions: DesignVector['regions'] = []
  for (let index = 0; index < regionCount; index++) {
    const winding = view.getUint32(offset, true) === 0 ? 'evenodd' : 'nonzero'
    offset += 4
    const loopCount = view.getUint32(offset, true)
    offset += 4
    const loops: number[][] = []
    for (let loopIndex = 0; loopIndex < loopCount; loopIndex++) {
      const segmentIndexCount = view.getUint32(offset, true)
      offset += 4
      const loop: number[] = []
      for (let segmentIndex = 0; segmentIndex < segmentIndexCount; segmentIndex++) {
        loop.push(view.getUint32(offset, true))
        offset += 4
      }
      loops.push(loop)
    }
    regions.push({ winding, loops })
  }

  return { vertices, segments, regions }
}
