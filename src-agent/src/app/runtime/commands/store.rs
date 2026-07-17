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
fn strip_markdown_headers(md: &str) -> String {
    md.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            let stripped = trimmed.trim_start_matches('#');
            if stripped.len() != trimmed.len() {
                stripped.trim_start().to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_markdown_headers_strips_leading_hashes_only() {
        let md = "# Title\n\nSome body text.\n## Sub heading\nMore body, #not-a-header inline.";
        let out = strip_markdown_headers(md);
        assert_eq!(
            out,
            "Title\n\nSome body text.\nSub heading\nMore body, #not-a-header inline."
        );
    }
}
