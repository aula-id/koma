#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::OAuthProvider;

/// [`OAuthProvider::from_wire_id`] must be the exact inverse of [`OAuthProvider::wire_id`]
/// for every real variant — this is the one mapping BOTH the daemon's attached
/// `StartOAuth` handler and the GUI host-relay's detached path resolve a wire string
/// through, so a drift here silently breaks one side or the other.
#[test]
fn from_wire_id_round_trips_every_variant() {
    for p in [
        OAuthProvider::Codex,
        OAuthProvider::Kilocode,
        OAuthProvider::Xai,
        OAuthProvider::ClaudeAI,
        OAuthProvider::KomaRun,
        OAuthProvider::CommandCode,
    ] {
        assert_eq!(OAuthProvider::from_wire_id(p.wire_id()), Some(p));
    }
}

/// `"codex_paste"` selects the paste-token input screen, not a real flow-driving
/// provider — it must resolve to `None` so callers route it to the paste path
/// instead of mistaking it for (or falling back to) a real provider.
#[test]
fn from_wire_id_rejects_paste_variants_and_unknown() {
    assert_eq!(OAuthProvider::from_wire_id("codex_paste"), None);
    assert_eq!(OAuthProvider::from_wire_id("clinepass_paste"), None);
    assert_eq!(OAuthProvider::from_wire_id("commandcode_paste"), None);
    assert_eq!(OAuthProvider::from_wire_id("not_a_provider"), None);
    assert_eq!(OAuthProvider::from_wire_id(""), None);
    // W11: the `extension` storage marker is NOT a from_wire_id input — ext flows
    // route through the `ext:<id>:<provider>` picker id, never a bare token.
    assert_eq!(OAuthProvider::from_wire_id("extension"), None);
}
