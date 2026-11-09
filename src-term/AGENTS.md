# Terminal design memory

Updated 2026-10-15 after studying the working tree, reviewing the user's terminal
reference, and restoring the earlier chat-led visual baseline and immediate
settings persistence.

## User preferences

- Scope is a function-for-function legacy port with a fresh UI/UX. Do not add
  product features, filters, validation rules, or save conflict policies beyond
  legacy. Reuse existing runtime operations; add adapters only where the new UI
  needs them.

- The user already likes the chat interface, composer, and related chat UI.
  Preserve their appearance and behavior; use them as the design baseline.
- The user explicitly loved the Sessions screenshot on 2026-10-15: black
  background, green accent, subtle selected-row surface, thin rounded borders,
  plain text tabs/actions, and muted shortcut labels. Preserve this visual style.
  **Cooking and History are tabs: show only the active tab's content using the
  available area, never simultaneous stacked or side-by-side panes, even wide.**
  Keep tab switching clickable and keyboard-accessible; retain each tab's
  selection and the History search when switching. This applies to both the
  landing Hub and the daemon-backed SessionHub.
- Improvements concern every full-screen menu, including missing legacy ports,
  rather than only settings.
- The screenshots showed truncated lists inside tall panels, inconsistent width
  and height allocation, and a bare `key —` line far from its provider list.
- The user explicitly wants **both proper inputs and visible, clickable controls**.
  Shortcuts are an additional input method. Do not require users to infer hidden
  editing targets or type credentials into an unexplained line of text.
- Use the available space for content. Short lists/forms can stay compact; do
  not stretch them just to fill the terminal. Long lists must use their actual
  allocated height, keep selection visible, and show that more items exist.
- Put management Add/New actions in the top-right border of their list panel,
  not at the end of the list or duplicated in the footer. Remove redundant
  provider endpoint/type summaries beneath lists; retain errors and confirmations.
- When a panel title has an existing letter shortcut, accent that letter even
  when inactive (for example, the G in General for g/G). Do not invent shortcuts.
- Hide `/select` from src-term slash suggestions while preserving the command
  and its transcript-copy capability. Preserve daemon selection indices when
  navigating or clicking the remaining suggestions.
- Notification banners show a working countdown and a clickable `[x]` for
  immediate dismissal. Time notifications by event identity, never by a newly
  allocated payload or callback; repeated identical notices are distinct events.
- Exit confirmation uses the shared compact overlay above the composer. Keep
  the chat and native composer mounted; suspend their input while confirming.
  In the sessions hub, show it below the hub content and suspend hub controls.
  Preserve Quit/Detach/Cancel shortcuts, scope semantics, approval priority on
  return, draft/selection restoration, and the exiting input lock.
- All terminal on/off controls use the same switch: `[━●]` in `success` when on,
  `[●─]` in `dim` when off. No checkbox squares, dot-only checkmarks, or redundant
  on/off words. The user approved exactly two arrangements:
  - Type A: `[━●] label`, before the text, for membership lists such as model
    roles, agent tools, skills, and extension use.
  - Type B: label aligned left, switch aligned to the right edge of its row,
    for settings, security, and MCP enabled controls. Reserve switch width so
    long labels/status text cannot push it off-screen.
  Reuse the shared toggle renderer; keep existing selection, activation,
  confirmation, and persistence handlers. Status badges, multi-way choices,
  and expand/collapse disclosures are distinct from on/off controls.

## Design signature

- React/OpenTUI; terminal-native and text-first. Hierarchy comes from spacing,
  alignment, semantic text colors, subtle surfaces, and restrained thin borders.
- Default to the earlier terminal’s black/green baseline; theme activation
  immediately changes the palette. Use `src/theme.ts` roles: `bg`, `fg`, `dim`, `accent`, `panel`, `selFg`, `selBg`,
  and status colors. Ten palettes exist; the identity is broader than dark green.
- Accent identifies focus and active elements. Supporting text and inactive
  outlines use `dim`. Filled selection uses the theme's `selBg`/`selFg` pair.
- The root has two columns of horizontal padding. Chat adds a one-row vertical
  inset, one-column transcript insets, and one-row gaps between content groups.
- User messages use a `panel` surface, a solid accent rail, and padded content.
  Assistant content uses a dotted `┊` rail and open space.
- Composer: rounded outline, accent when focused, dim when inactive, accent `[$]`
  prompt, two-column inner padding, adaptive height, session name in the top-right
  border, metadata above, compact status/elapsed/usage below.
- Tool/attachment cards and reasoning disclosure reveal detail progressively.
  Chat palettes have eight stable rows inside a ten-row frame. Preserve this.

## Current menu system

- `src/modes/place.ts` decides full-screen versus overlay. Preserve the existing
  distinction unless the user changes it. Effort, Model, Bash, Todo, Skill, attachment
  lists, extension `use`, and the old settings menu remain chat overlays.
  The effort selector sits above the composer in the same compact frame, with
  its choices and any model-specific reasoning note inside that frame.
  All chat overlays share this order: spacer, Koma/version/mode/model row,
  spacer, overlay, composer border, tips/status. Keep no gap between the overlay
  and composer. This includes suggestions, menu overlays, approvals, rewind,
  subagents and exit confirmation. Keep the native composer mounted and preserve
  existing input ownership and overlay priority. The hub has no composer.
- `src/modes/surface.tsx` handles every full-screen `ModeSnapshot` menu, including
  SessionHub. It forwards bracketed paste to the daemon as `Paste`.
- `Fullscreen` in `src/modes/bits.tsx` gives menus a common title/content/actions
  structure, vertical spacing, and a footer protected from growing content.
- Full-screen rows use accent focus, subtle panel surfaces, and muted supporting
  values. Full-screen actions are understated clickable text with dim shortcut
  labels; preserve the compact overlays and their existing controls.
- Shared list/detail layouts keep both areas at narrow widths. Wide layouts use
  proportional widths, equal panel sizing, and consistent gaps.
- Quick settings keeps Appearance and General compact and side by side when
  space permits, stacked when narrow. Spare space stays outside their rounded
  borders. Summaries are bounded; the top navigation stays visible at every size.
- Fields have visible labels and rounded outlines. Active fields show a caret;
  long active values reveal their tail. Save/Cancel and other actions are
  clickable. Hints must match the current editing/navigation submode.
- Search-provider settings show an explicit provider-named API-key field, saved
  credential status, compact provider choices, and visible Save/Cancel actions.
- SessionHub, all eight OAuth choices, OAuth wait/token/failure states, onboarding
  results, conditional model fields, routes, and model add/filter rows are ported.
- Model catalogue results and conditional field order come from the daemon's
  snapshot. Do not reconstruct OAuth membership or field indices in the client.
- Security navigation starts with daemon and YOLO controls, then domain-grouped
  tools. Dependencies are grouped by tier. Rendering must match daemon ordering.
- `/security` is a SOC-style console (`src/modes/security.tsx`): header pane tabs
  with a posture badge (ONLINE/ARMED/YOLO ACTIVE/OFFLINE); a status board of six
  gauge tiles (runtime, daemon, guest, boundary, egress, catalog) that collapses
  to a one-line strip below 96×28; Docker control + Operations panels with Type B
  switches (daemon `s`, YOLO `o`) folding into one wrapping bar when short; then
  the Tools table (columns, readiness state, filter/search in the top-right
  border, selected-tool detail row with Pull/Activate/Remove) and a Guest map
  panel at ≥110 columns. Pane tabs and controls are not duplicated in the footer.
  Operations has a direct lane chooser (None · Research · Red · Blue, `MenuReplace`
  field `lane`; `l` still cycles). `security:` status lines arrive as `notice` and
  show in the Operations bottom border (or inline in the compact bar). Image pulls
  run in the background; `image_pull` drives an animated spinner/bar (local timer,
  only while pulling) on the Images summary row, the pulling row, the Guest tile
  and the Tools state column; `image_pull_result` keeps the last outcome visible.
- Text editors fit their allocated height and retain the caret in view. Extension
  documents scroll to the selected action. Usage respects cost/token selection.
- Hub, quit, loading, and the subagent viewer also have independent compositions
  outside ModeSurface. These should remain part of future visual review.

## OpenTUI and verification notes

- Installed OpenTUI 0.6.5 uses native scenes. Per-box `resize` listeners do not
  update these layouts; `resized`/`onSizeChange` hooks are rejected by native scenes.
  Read completed layout on the renderer's `frame` event instead, update React
  state only when dimensions change, and remove listeners on unmount.
- Keep `flexBasis={0}`, `minHeight={0}`, and `minWidth={0}` where growing panes
  must share bounded space. Protect fixed fields/actions with `flexShrink={0}`.
- `src/modes/layout.test.tsx` renders 27 full-screen states at 50×24, 80×18, and
  100×40, plus resize, narrow navigation, paste/click, nested-selection, and long
  extension-menu checks. This is rendered fixture coverage, not live-account
  verification of every integration.
- `src/keys.ts` must preserve typed case, punctuation, spaces, and Unicode.
  Normalize navigation names separately from characters entered in forms.
- The daemon owns state and most editing/navigation. Check its actual handlers
  before changing labels, field order, selection indices, or action dispatch.
- The TypeScript check has React/Bun development types and passes. Verify with
  `.tools/bun node_modules/typescript/bin/tsc --noEmit` from `src-term`.

## Full-screen migration follow-up (2026-10-15)

- The previous pass's universal list/detail layout, hint-string button inference,
  and arrow-replay pointer behavior were regressions. Full-screen management lists
  now span their width and open dedicated forms/pages. Preserve compact overlays.
- Panels use complete thin rounded borders with plain titles; existing shortcut
  letters use accent, and focus accents the outline. Keep semantic toggle/status
  colors. Do not add decorative title
  fragments, bulky theme swatches, duplicate edit controls, or bracket every
  full-screen action.
- `modes/actions.ts` defines explicit controls and stable entity selection. Menu
  mutations use additive protocol v1 and correlated results. Legacy Ack is never a
  completion signal for a new menu operation or composer flush.
- `modes/data.ts` is generated from Rust. Regenerate with
  `python3 src-term/scripts/generate-menu-types.py` after changing projections.
- Native inputs/textareas own editable buffers, selection and cursor. Do not
  replace their buffer with each live snapshot. Keep secret values masked and
  unchanged blank secrets intact. Clear the native focus registry on unmount.
- Settings apply and persist when activated: Enter commits a theme immediately;
  moving the highlight alone never applies it. Toggles/choices persist on
  activation; text persists on confirmation. Esc cancels only the current
  unconfirmed edit or goes back; it never undoes an applied setting. Do not add
  screen-level Save buttons, dirty indicators, save/discard prompts, or a
  preview-versus-saved theme state. Retain genuine provider/model/search form
  submission controls. Reuse the existing persistence operation while keeping
  settings open; leaving a clean screen must not rewrite its old catalogue.
  Catalogue reloads must preserve open drafts and stable navigation identities.
- Remote sessions use `remote/term_helper.rs` and `DaemonTransport`; keep the same
  OpenTUI frame decoder/session UI for local and remote. Never suspend the renderer
  to invoke Ratatui for this flow. Credentials travel only over helper pipes.
- Read layout only after a completed renderer frame; pre-layout reads report
  zero and can leave fixed panels with unmeasured rows. Keep list slots stable
  while scrolling and reserve a visible range indicator for clipped lists.
- The forty theme/size color tests inspect captured native spans, not just text.
  `KOMA_PREVIEW_PATH` exports those real frames as an HTML review artifact.
- Type checking now has React/Bun development types and passes. Real daemon and
  SSH-helper integration scripts are in `src-term/scripts`. Read the verification
  limits in `docs/implementation/terminal-slash-parity.md` before claiming parity
  against external accounts, live hosts or installed services.

## Terminal parity preferences (2026-10-15)

- Enclosed surfaces use thin rounded borders; preserve transcript rails and separators.
- Every editable field accepts paste exactly once through its active input.
- Attachments follow their messages and keep their markers and metadata through edits and follow-ups.
- Pending approval controls remain visible and usable immediately above the composer,
  with bounded scrollable details. Approval owns decision keys and paste until resolved;
  preserve the composer draft and restore focus afterwards.
- Composer content wraps by words and grows from the native visual line count after layout.

- OpenTUI textarea `virtualLineCount` is viewport-limited in 0.6.5. Composer height
  must use `editorView.getTotalVirtualLineCount()` after a renderer frame, with
  `minWidth={0}` and `flexBasis={0}` so long content receives a bounded wrap width.

## Form navigation and transient status (2026-10-15)

- Highlighting a field is browsing. Mixed forms start without native input focus;
  Enter or clicking an editable field activates its editor. Esc leaves editing
  before leaving the form. Keep intentional autofocus for dedicated searches and
  multiline editors; result-list arrows and editor arrows retain their own roles.
- Web-search settings start in provider selection. Arrows browse; Enter/click
  confirms. API providers then edit a masked key; DuckDuckGo needs no key.
  Return to providers is visible, Esc returns there before cancelling, and changing
  provider clears its unsaved key. Blank replacement keys retain saved credentials.
- Model provider fields open a compact anchored chooser on Enter/click. Use the
  daemon's eligible provider/OAuth choices, stable IDs and canonical indices.
  Arrows only browse; Enter/click commits; Esc preserves the original binding.
  Use the shared runtime provider helper for query, route and catalogue changes.
- Native drafts, selections and caret offsets survive browsing/editing transitions.
  Scope cached drafts to the form and provider; discard them with the form.
  Flush changed native content before dependent actions, preserving exactly-once
  bracketed paste. Keep Save/Cancel adjacent in short forms and protected in long ones.
- Code blocks are the explicit square-border exception. Render parser code tokens,
  with language labels, inner padding and width-aware wrapping. Never split message
  contents with a fence regex. Preserve indentation, Unicode and literal backticks,
  including streamed and incomplete fences. Python/Rust assets and source hashes
  live in `assets/parsers`; JS/TS use OpenTUI's shipped assets. No runtime downloads.
- Unselected queued follow-ups and attachment controls use neutral colors. Apply
  the accent to follow-ups only while follow-up navigation actually owns focus.
- Do not render images in the terminal yet. Show compact neutral attachment piles,
  laid out horizontally and wrapping onto new rows. Preserve attachment metadata,
  open/remove controls and native image paste support for future rendering work.
- Transient Refreshing/Loading/Installing notices belong at the bottom-right of a
  bordered panel or in a reserved footer status row. They must not insert/remove
  content rows or shift lists and fields while toggling. Preserve useful errors.

Verification: `scripts/verify-form-input.tsx` exercises actual OpenTUI keyboard,
pointer and native selection/paste against an isolated daemon and local catalogue
server. Build the daemon first, then run `.tools/bun scripts/verify-form-input.tsx`.
It captures `/tmp/koma-form-frames.txt` at 50×24, 80×18, 100×40 and 160×45.
`src/markdown.test.tsx` checks real parsers, aliases and captured native color spans
for all four languages and ten themes; it exports `/tmp/koma-code-frames.txt`.
`src/modes/focus-status.test.tsx` covers shared focus, exactly-once selection paste
and stable loading positions. These checks use fixtures and local services; they
are not verification of live provider accounts, OAuth flows, remote hosts or a
future image renderer. Keep the existing daemon harness and Rust protocol checks.

- Bare `/model` opens the five-role picker (main, awareness, planner, compactor,
  safeguard). `/model <role>` remains a direct shortcut; `/model help` and `?`
  retain documentation. Role/agent model lists have an active native search
  input, daemon filtering by name/model/provider, arrow navigation, and Enter
  or direct stable-ID pointer confirmation. Empty results never inherit or
  modify a binding. Esc from a role chosen in the role picker returns there;
  directly opened role pickers still close. Preserve the composer draft.
  `scripts/verify-model-picker.tsx` exercises the flow against an isolated daemon,
  including a persisted role swap, empty results, stable filtered pointer targets,
  native paste, direct shortcuts/help and the four terminal sizes. Fixture startup
  notices are dismissed before the short-terminal geometry captures.

## Independent Usage reader (2026-10-15)

- `/usage` in OpenTUI uses `usage-reader.ts` and the framed `koma remote-usage`
  worker. Start it lazily, retain it for the terminal connection, and poll only
  while Usage is mounted. One request may be active; scope/range/session changes
  invalidate old responses. Cost/Tokens and chart selection are local rendering
  decisions. Keep transient status and the interval detail outside the scrollbox.
- The reader opens only the resolved `usage.sqlite`, read-only, and gathers each
  result in one transaction through the fallible connection queries. Never call
  the writable ledger opener, migrate schemas, or scan session files here.
  `KOMA_TEST_DATA_DIR` remains the debug fixture override.
- `GetMenuCapabilities.usage_reader` advertises optional support. Supporting
  terminals send `SetUsageReader`; the daemon then projects navigation and live
  counters without collecting ledger data. Keep legacy data for other clients.
- Remote Usage travels through its own authenticated SSH child in `term_helper`.
  Usage failures must remain Usage errors, and remote sessions must never create
  a local ledger reader. Keep worker cleanup tied to helper/terminal lifetime.
- Today is local midnight to midnight; Week/Year preserve rolling 7/365-day
  cutoffs. Session is lifetime. Format dates with the reader's UTC offset, fill
  zero periods, and preserve partial boundary buckets and selected timestamps.
- Run `scripts/verify-usage.py`, `scripts/verify-remote.py`, and
  `.tools/bun scripts/verify-usage-ui.tsx` after building Rust.
  These use isolated ledgers, real framed pipes/daemon sockets, and fake SSH,
  not live accounts or external hosts. `usage-dashboard.test.tsx` captures all
  four views at 50×24, 80×18, 100×40 and 160×45, and inspects native color spans
  for every theme. Review `/tmp/koma-usage-frames.txt` and
  `/tmp/koma-usage-review.html`. See `docs/implementation/usage-reader.md`.

- Usage bars use the legacy percentile heat colors (green/lime/orange/red for the
  default palette). Selection uses a separate `^^^` row underneath the bars;
  never fill a selected column's background or recolor its data, because that
  changes its apparent height/intensity. Keep the marker row reserved while browsing.
  Fill the background of each occupied bar row with its heat color as well as the
  glyph foreground, so terminal font line spacing cannot break bars into dashes.

- Year calendars distribute all available columns across visible weeks, including
  fractional cell widths rounded into balanced whole-column widths. Do not cap
  the grid at one/two columns per week and leave half the panel empty. Narrow
  widths still scroll horizontally with selection; avoid overlapping month labels.

- Usage scope, range and metric choices reuse the shared `Action` tabs with
  `primary` for the active choice, like Cooking/History. Use the approved header:
  `Global · Session (tab) | Today (1) | Week (2) | Year (3) | Cost · Tokens (m) | Back (esc)`.
  Keep shortcuts/separators muted. Do not add selected-status dot suffixes or
  custom filled selection wrappers. Hide date controls in Session.
