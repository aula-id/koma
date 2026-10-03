// Selection chrome is drawn in canvas pixels, outside the zoomed layer.
// WebKit paints that layer at document resolution and then scales the bitmap,
// so a stroke inside it grows with zoom. vector-effect and 1/zoom CSS do too.
// This matrix keeps rotation and flip and drops the zoom scale, so 1px stays 1px.

export type DesignAffine = { a: number; b: number; c: number; d: number; e: number; f: number }

export const DESIGN_AFFINE_ID: DesignAffine = { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 }

type PlacedNode = { x: number; y: number; rotation?: number; flipX?: boolean; flipY?: boolean }

/** Node-local box point to the parent's content space. Matches the canvas CSS transform (origin at the center). */
export function designNodeMatrix(node: PlacedNode, w: number, h: number): DesignAffine {
  const cx = w / 2
  const cy = h / 2
  const fx = node.flipX ? -1 : 1
  const fy = node.flipY ? -1 : 1
  const radians = ((node.rotation ?? 0) * Math.PI) / 180
  const cos = Math.cos(radians)
  const sin = Math.sin(radians)
  return {
    a: fx * cos,
    b: fx * sin,
    c: -fy * sin,
    d: fy * cos,
    e: node.x + cx * (1 - fx * cos) + cy * fy * sin,
    f: node.y + cy * (1 - fy * cos) - cx * fx * sin,
  }
}

/** Apply `inner` first, then `outer`. */
export function composeDesignAffine(outer: DesignAffine, inner: DesignAffine): DesignAffine {
  return {
    a: outer.a * inner.a + outer.c * inner.b,
    b: outer.b * inner.a + outer.d * inner.b,
    c: outer.a * inner.c + outer.c * inner.d,
    d: outer.b * inner.c + outer.d * inner.d,
    e: outer.a * inner.e + outer.c * inner.f + outer.e,
    f: outer.b * inner.e + outer.d * inner.f + outer.f,
  }
}

export function applyDesignAffine(matrix: DesignAffine, x: number, y: number): { x: number; y: number } {
  return { x: matrix.a * x + matrix.c * y + matrix.e, y: matrix.b * x + matrix.d * y + matrix.f }
}

export function translateDesignAffine(x: number, y: number): DesignAffine {
  return { a: 1, b: 0, c: 0, d: 1, e: x, f: y }
}

/**
 * CSS matrix for a box of `width`×`height` screen pixels.
 * One pixel in that box is one screen pixel. Rotation and flip still match the node.
 */
export function screenChromePlacement(
  toCanvas: DesignAffine,
  w: number,
  h: number,
  panX: number,
  panY: number,
  zoom: number,
): { matrix: DesignAffine; width: number; height: number } {
  const z = zoom > 0 ? zoom : 1
  const toScreen = composeDesignAffine({ a: z, b: 0, c: 0, d: z, e: panX, f: panY }, toCanvas)
  return {
    matrix: { a: toScreen.a / z, b: toScreen.b / z, c: toScreen.c / z, d: toScreen.d / z, e: toScreen.e, f: toScreen.f },
    width: w * z,
    height: h * z,
  }
}
