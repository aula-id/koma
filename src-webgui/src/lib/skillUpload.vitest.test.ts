import { describe, expect, it } from 'vitest'
import { MAX_SKILL_ZIP_BYTES, UPLOAD_CHUNK_BYTES, uploadChunks, uploadProgress } from './skillUpload'

describe('uploadChunks', () => {
  it('has no chunks for an empty file', () => {
    expect(uploadChunks(0)).toEqual([])
  })

  it('splits with a short last chunk', () => {
    expect(uploadChunks(10, 4)).toEqual([
      { offset: 0, end: 4 },
      { offset: 4, end: 8 },
      { offset: 8, end: 10 },
    ])
  })

  it('has no empty trailing chunk for an exact multiple', () => {
    expect(uploadChunks(8, 4)).toEqual([
      { offset: 0, end: 4 },
      { offset: 4, end: 8 },
    ])
  })

  it('covers the largest package contiguously', () => {
    const chunks = uploadChunks(MAX_SKILL_ZIP_BYTES)
    expect(chunks).toHaveLength(MAX_SKILL_ZIP_BYTES / UPLOAD_CHUNK_BYTES)
    chunks.forEach((c, i) => {
      if (i > 0) expect(c.offset).toBe(chunks[i - 1].end)
    })
    expect(chunks[0].offset).toBe(0)
    expect(chunks[chunks.length - 1].end).toBe(MAX_SKILL_ZIP_BYTES)
  })
})

describe('uploadProgress', () => {
  it('rounds and caps at 100', () => {
    expect(uploadProgress(1, 3)).toBe(33)
    expect(uploadProgress(2, 3)).toBe(67)
    expect(uploadProgress(5, 3)).toBe(100)
  })

  it('is 0 for an unknown total', () => {
    expect(uploadProgress(10, 0)).toBe(0)
  })
})
