import { createFileRoute, Link } from '@tanstack/react-router'

export const Route = createFileRoute('/_docs/gui/skills')({
  component: GuiSkillsPage,
})

function GuiSkillsPage() {
  return (
    <article>
      <h1 className="mb-4 text-2xl font-bold text-koma-accent">Skills</h1>
      <p className="mb-6 text-koma-fg">
        The Skills panel discovers reusable instruction sets and controls which ones are loaded into the current chat.
      </p>

      <div className="space-y-5 text-sm leading-relaxed text-koma-dim">
        <section>
          <h2 className="mb-1 text-base font-semibold text-koma-fg">Scope and ownership</h2>
          <p><strong>Global</strong> skills belong to Koma across projects, <strong>Project</strong> skills belong to the active chat&apos;s workspace, and <strong>External</strong> skills are read-only. Koma automatically discovers <code>~/.koma/skills</code>, <code>&lt;project&gt;/.claude/skills</code>, <code>&lt;project&gt;/.agent/skills</code>, and <code>&lt;project&gt;/.agents/skills</code>. Project locations require an active chat because its project directory supplies <code>&lt;project&gt;</code>. Settings → Skills configures only additional External locations.</p>
          <p className="mt-2">Locations are scanned when the Skills panel opens or when the active chat changes while the panel stays open. To scan again, use the circular-arrow button in the Skills header (<strong>Rescan skill locations</strong>). Discovery is request-driven, not a live filesystem watcher. If no catalogue reply arrives within 12 seconds, the spinner stops and the panel marks the list unconfirmed rather than claiming the scan failed; you may retry Rescan, and a delayed reply can still update the list. When the same name exists in multiple roots, later sources win: <code>Global → .claude → .agent → .agents → additional External</code>.</p>
        </section>
        <section>
          <h2 className="mb-1 text-base font-semibold text-koma-fg">Adding a skill</h2>
          <p><strong>Add skill</strong> is available only while a project is open. It offers three methods: <strong>Create new</strong> opens the structured Global/Project editor, <strong>Upload .zip</strong> installs a packaged skill, and <strong>Create with Koma</strong> prefills a guided draft in the active chat. Existing Global skills stay editable without a project. Choosing <strong>Project</strong> scope, while creating, uploading, or duplicating, stays disabled until a project is open. The upload screen keeps its summary concise and exposes the exact limits under <strong>Package requirements</strong>: maximum 1,000 files, depth 16, 8 MiB per companion, 2 MiB for SKILL.md, and 64 MiB compressed and uncompressed total. Symlinks, special files, path traversal, collisions, and malformed SKILL.md are rejected.</p>
        </section>
        <section>
          <h2 className="mb-1 text-base font-semibold text-koma-fg">Loaded state</h2>
          <p><strong>Load into chat</strong> and <strong>Remove from chat</strong> change only the active chat context. This is the same state controlled by the TUI <Link to="/tui/commands-skill" className="text-koma-accent hover:underline">/skill command</Link>; it is not a persistent enabled switch. <strong>Save</strong> updates disk only. For an owned skill already loaded in the active chat, <strong>Save &amp; Reload</strong> saves and adopts that version in one operation. <strong>Reload from disk</strong> remains available for changes made outside the editor, including External skills and edits made with Koma. If saving succeeds but the chat can no longer accept the reload, Koma reports disk-saved/chat-unchanged rather than claiming full success.</p>
        </section>
        <section>
          <h2 className="mb-1 text-base font-semibold text-koma-fg">Search and selection</h2>
          <p>Search matches names, descriptions, and triggers. Click or Enter opens a skill. Right-click a row in the Skills panel for <strong>Load into chat</strong>, <strong>Remove from chat</strong>, <strong>Reload from disk</strong>, <strong>Duplicate</strong>, and <strong>Delete</strong>. Ctrl/Cmd-click toggles extra rows and Shift-click selects a visible range; that menu then applies to the whole selection. Ctrl-selected rows use the panel header tint. The list does not label scope or whether the editor is open. <strong>Global</strong> and <strong>Project</strong> split the catalogue by ownership. Claude project skills appear under Project, and additional External roots appear under Global. A skill loaded into the chat is sorted to the top of its tab, marked with a left accent border, and labeled with an accent “active” pill at the top right. Without an active chat, the panel explains why Project skills and chat-loading actions are unavailable.</p>
        </section>
        <section>
          <h2 className="mb-1 text-base font-semibold text-koma-fg">Safe editing</h2>
          <p>Owned skills can be created, edited, downloaded as a ZIP, and deleted. Koma uses opaque source identities and content generations, so a skill changed or removed on disk leaves the draft visible but read-only rather than overwriting a new winner. Ordinary unknown YAML fields are preserved. Structured Save is disabled for advanced YAML constructs that cannot be round-tripped safely; use <strong>Edit with Koma</strong> or edit SKILL.md manually. Companion files are lazy, read-only, and offer Code/Preview views.</p>
        </section>
        <section>
          <h2 className="mb-1 text-base font-semibold text-koma-fg">External skills</h2>
          <p>Add existing skill directories under Settings → Skills. Locations are canonicalized when saved; duplicate or equivalent roots are rejected, as are roots overlapping Koma-owned Global or Project roots. Use <strong>Duplicate to Koma</strong> to copy a complete External skill and its companion files into Global or Project scope before editing; the source remains unchanged. <strong>Edit with Koma</strong> guides a revision but explicitly directs the result into an owned duplicate. External files cannot be saved, deleted, or downloaded directly.</p>
        </section>
        <section>
          <h2 className="mb-1 text-base font-semibold text-koma-fg">Declared tools</h2>
          <p>The optional <code>allowed-tools</code> frontmatter is edited with the same tool chips as a sub-agent. It is compatibility metadata; Koma does not currently enforce it as a runtime restriction.</p>
        </section>
      </div>
    </article>
  )
}
