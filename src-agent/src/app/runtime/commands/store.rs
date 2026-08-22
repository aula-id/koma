//! The `/store` command: open the koma.run extension marketplace browser, plus the
//! shared row/detail mappers the async catalogue/detail drain
//! (`event_loop::global::drains::drain_store`) reuses to fold a landed fetch into
//! `Mode::ExtStore` the SAME way this module's own [`handle_store`] seeds it.

use anyhow::Result;

use crate::app::mode::{ExtStoreState, Mode, StoreDetailData, StoreRow, StoreSubMode};
use crate::app::state::AppState;
use crate::ipc::proto::{StoreDetailWire, StoreItemWire};
use crate::model::app_config::OAuthProvider;

/// Handle the `/store` command: open the marketplace browser in Browse and kick off the
/// initial async catalogue fetch. Does NOT require an active session — browsing hits the
/// PUBLIC store endpoints (no auth); only installing needs a koma.run sign-in.
pub(super) fn handle_store(state: &mut AppState, handle: &tokio::runtime::Handle) -> Result<()> {
    let komarun_connected = state
        .rest
        .config
        .oauth_conns
        .iter()
        .any(|c| c.provider == OAuthProvider::KomaRun);
    *state.mode_mut() = Mode::ExtStore(Box::new(ExtStoreState {
        sub_mode: StoreSubMode::Browse,
        rows: Vec::new(),
        list_sel: 0,
        loading: true,
        error: None,
        detail: None,
        detail_loading: false,
        detail_error: None,
        installing: false,
        install_error: None,
        komarun_connected,
    }));
    crate::app::ext::ext_store::kick_off_store_browse(&mut state.rest, handle, None, None);
    Ok(())
}

/// Map one catalogue [`StoreItemWire`] to a [`StoreRow`], baking in whether `id` is
/// already installed. `installed_ids` is a snapshot of `config.installed_extensions`'s
/// ids taken by the caller BEFORE the fold (so the membership check never races a
/// concurrent registry mutation mid-loop over many rows).
pub(crate) fn store_row_from_item(
    item: &StoreItemWire,
    installed_ids: &std::collections::HashSet<String>,
) -> StoreRow {
    StoreRow {
        id: item.id.clone(),
        name: item.name.clone(),
        tagline: item.tagline.clone(),
        tier: item.tier.clone(),
        kind: item.kind.clone(),
        latest_version: item.latest_version.clone(),
        author: item.author.clone(),
        installed: installed_ids.contains(&item.id),
    }
}

/// Map one [`StoreDetailWire`] to a [`StoreDetailData`], stripping markdown headers
/// (leading `#`-runs) from `description_md` for a plain-text TUI render — no full
/// markdown renderer, per the `/store` v1 scope (see [`strip_markdown_headers`]).
pub(crate) fn store_detail_from_wire(d: &StoreDetailWire) -> StoreDetailData {
    StoreDetailData {
        description: strip_markdown_headers(&d.description_md),
        contributes_models: d.contributes.models,
        contributes_panels: d.contributes.panels,
        contributes_tools: d.contributes.tools,
        contributes_sub_agents: d.contributes.sub_agents,
        requires: d.requires.clone(),
        versions: d.versions.clone(),
    }
}

/// Strip leading `#` markdown-header markers (`# `, `## `, …) from each line, leaving the
/// heading text as a plain line — the "minimal" markdown stripping the `/store` detail
/// view does instead of running a full markdown renderer.
///
/// A stripped heading is forced onto its OWN paragraph: a blank line is inserted right
/// after it (unless one is already there, or it's the last line), so a heading glued
/// directly to the following prose in the source (`"# Title\nBody..."`, no blank line
/// between them) still reads as a distinct line once rendered — without this, the
/// following text visually runs straight into the heading (see the `/store` detail
/// view's paragraph-aware wrap in `view::store`, which splits on these `\n`s).
fn strip_markdown_headers(md: &str) -> String {
    let lines: Vec<&str> = md.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 1);
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let stripped = trimmed.trim_start_matches('#');
        if stripped.len() != trimmed.len() {
            out.push(stripped.trim_start().to_string());
            // Force a paragraph break after the heading, unless the source already
            // has one (or there's nothing left to separate it from).
            let next_is_blank_or_absent = lines
                .get(i + 1)
                .map(|l| l.trim().is_empty())
                .unwrap_or(true);
            if !next_is_blank_or_absent {
                out.push(String::new());
            }
        } else {
            out.push(line.to_string());
        }
    }
    out.join("\n")
}

#[cfg(test)]
#[path = "store_test.rs"]
mod tests;
