// Pure, React-free scale/aggregation helpers for GraphBubble (GK5b's
// bubble/activity chart). Mirrors gitGraphLayout.ts's split: pure math here,
// SVG-pixel rendering in the component. No side effects, no DOM — the
// component owns measurement (container width) and hands plain numbers in.

import type { ActivityCommit } from '../store/koma'
import { authorColor } from './authorColor'

export function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v))
}

// A linear scale [domainMin, domainMax] -> [rangeMin, rangeMax]. Guards a
// zero-width (or inverted) domain — a single commit, every commit on the same
// instant, or an empty series upstream — by mapping everything to the range's
// midpoint instead of dividing by zero.
export function linearScale(
  domainMin: number,
  domainMax: number,
  rangeMin: number,
  rangeMax: number,
): (v: number) => number {
  const span = domainMax - domainMin
  if (!(span > 0)) {
    const mid = (rangeMin + rangeMax) / 2
    return () => mid
  }
  return (v: number) => rangeMin + ((v - domainMin) / span) * (rangeMax - rangeMin)
}

// Bubble radius: sqrt(added+deleted) * k, clamped to [minR, maxR]. sqrt keeps
// AREA (not radius) roughly proportional to lines changed — the conventional
// bubble-chart convention — the clamp keeps one huge commit from swallowing
// the chart, and a 0-line commit (e.g. an empty commit) still draws a dot.
export function radiusScale(linesChanged: number, k: number, minR: number, maxR: number): number {
  return clamp(Math.sqrt(Math.max(0, linesChanged)) * k, minR, maxR)
}

// One author lane's aggregate — `key` is the SAME grouping key authorColor()
// hashes on (email, falling back to name, falling back to "?"), so a lane's
// colour and its bubbles' colours never disagree.
export type AuthorAgg = {
  key: string
  name: string
  email: string
  totalLines: number
  totalAdded: number
  totalDeleted: number
  commitCount: number
  color: string
}

// Group `commits` by author (keyed like authorColor), sum added/deleted lines
// and commit counts, and sort DESCENDING by total lines changed (ties broken
// by commit count) — the busiest author's lane sits at the top.
export function aggregateAuthors(commits: ActivityCommit[]): AuthorAgg[] {
  const byKey = new Map<string, AuthorAgg>()
  for (const c of commits) {
    const key = c.email.trim() || c.author.trim() || '?'
    const existing = byKey.get(key)
    if (existing) {
      existing.totalLines += c.added + c.deleted
      existing.totalAdded += c.added
      existing.totalDeleted += c.deleted
      existing.commitCount += 1
    } else {
      byKey.set(key, {
        key,
        name: c.author,
        email: c.email,
        totalLines: c.added + c.deleted,
        totalAdded: c.added,
        totalDeleted: c.deleted,
        commitCount: 1,
        color: authorColor(c.author, c.email),
      })
    }
  }
  return Array.from(byKey.values()).sort(
    (a, b) => b.totalLines - a.totalLines || b.commitCount - a.commitCount,
  )
}

// Per-author commit-count buckets across the whole commit time range.
// Keyed identically to aggregateAuthors (email||author||'?') so a card can
// look its author up by AuthorAgg.key. Returns [] semantics via an empty Map
// when there are no date-parseable commits.
export function authorSparklines(commits: ActivityCommit[], bucketCount: number): Map<string, number[]> {
  const map = new Map<string, number[]>()
  if (bucketCount <= 0) return map
  const stamped = commits
    .map((c) => ({ c, t: Date.parse(c.date) }))
    .filter((x) => !Number.isNaN(x.t))
  if (stamped.length === 0) return map
  let min = Infinity
  let max = -Infinity
  for (const { t } of stamped) {
    if (t < min) min = t
    if (t > max) max = t
  }
  const span = max - min || 1
  for (const { c, t } of stamped) {
    const key = c.email.trim() || c.author.trim() || '?'
    let arr = map.get(key)
    if (!arr) {
      arr = new Array(bucketCount).fill(0)
      map.set(key, arr)
    }
    let idx = Math.floor(((t - min) / span) * bucketCount)
    if (idx >= bucketCount) idx = bucketCount - 1
    if (idx < 0) idx = 0
    arr[idx] += 1
  }
  return map
}

export type TimeTick = { ts: number; label: string }

// `count` evenly-spaced date ticks across [minTs, maxTs] (inclusive of both
// ends). Guards a zero/negative span (all commits on the same instant, or a
// single commit) by returning just that one instant — never divides by zero.
export function buildTimeTicks(minTs: number, maxTs: number, count: number): TimeTick[] {
  const label = (ts: number): string => {
    const d = new Date(ts)
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
  }
  if (!(maxTs > minTs) || count <= 1) {
    return [{ ts: minTs, label: label(minTs) }]
  }
  const n = Math.max(2, count)
  const ticks: TimeTick[] = []
  for (let i = 0; i < n; i++) {
    const ts = minTs + ((maxTs - minTs) * i) / (n - 1)
    ticks.push({ ts, label: label(ts) })
  }
  return ticks
}
