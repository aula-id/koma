//! Agent Skill discovery and loading: scan known directories for SKILL.md files,
//! build a catalogue, and provide on-demand body loading.
//!
//! ```text
//! Skills are user-authored markdown files with optional YAML frontmatter.
//! The filename/dir stem is the skill name (lowercased, validated).
//!
//! Layout per root:
//!   skills/
//!     foo.md                 ← flat: name = "foo"
//!     bar/SKILL.md           ← dir form: name = "bar"
//!     bar/references/...     ← reachable after load via read/glob
//!
//! Discovery tiers (later overrides earlier by name):
//!   1. ~/.koma/skills/       (global)
//!   2. <workdir>/.claude/skills/  (Claude compat)
//!   3. <workdir>/.agent/skills/   (koma project)
//!   4. <workdir>/.agents/skills/  (koma project, highest priority)
//! ```

mod def;
mod parse;
mod registry;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;

pub use registry::SkillRegistry;
