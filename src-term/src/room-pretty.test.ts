import { expect, test } from "bun:test"
import { outline, prettyText, toolDigest } from "./room-pretty.ts"

const refs = '[{"claim":"claims/a","references":[{"key":"claims/a","revision":2},{"key":"evidence/x","revision":1}],"revision":2},{"claim":"claims/b","references":[],"revision":1}]'

test("claim and evidence references become a nested list", () => {
  const out = prettyText(`Re-check these revisions independently:\n${refs}\nRecord a verdict.`)
  expect(out).toContain("Re-check these revisions independently:")
  expect(out).toContain("- `claims/a` (rev 2)")
  expect(out).toContain("  - **references**")
  expect(out).toContain("    - `evidence/x` (rev 1)")
  expect(out).toContain("- `claims/b` (rev 1)")
  expect(out).toContain("Record a verdict.")
  expect(out).not.toContain('{"claim"')
})

test("short brackets, broken JSON and code fences are left alone", () => {
  expect(prettyText("see [1] and {x}")).toBe("see [1] and {x}")
  const broken = `look {"a": 1, "b": [1, 2, 3, 4, 5, 6, 7, 8,}`
  expect(prettyText(broken)).toBe(broken)
  const fenced = "```json\n" + refs + "\n```"
  expect(prettyText(fenced)).toBe(fenced)
})

test("brackets inside a string do not end the block early", () => {
  const out = prettyText('x {"name":"a } b","list":[1,2,3],"note":"some longer text"} y')
  expect(out).toContain("**name**: a } b")
  expect(out).toContain("- 2")
})

test("outline of a plain object lists its fields", () => {
  expect(outline({ a: 1, b: { c: "x" } })).toBe("- **a**: 1\n- **b**\n  - **c**: x")
})

test("a delegation reads as who, what kind, and the prompt with its JSON set out", () => {
  const args = JSON.stringify({ goal_key: "g", goal: "Check it", dispatch_key: "d1", assignments: [{ avatar: "reviewer", kind: "verification", prompt: `Re-check:\n${refs}\nThen finish.` }] })
  const out = toolDigest("group_delegate", args)!
  expect(out).toContain("**reviewer** · verification")
  expect(out).toContain("- `claims/a` (rev 2)")
  expect(out).toContain("Check it")
  expect(toolDigest("bash", '{"command":"ls"}')).toBeNull()
})
