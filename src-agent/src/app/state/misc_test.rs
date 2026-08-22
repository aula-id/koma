#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn explicit_catalogue_retry_clears_prior_failure() {
    let endpoint = "https://koma.run/api/v1/koma-premium";
    let mut rest = AppStateRest::new();
    rest.models_cache_failed = Some(endpoint.to_string());

    rest.request_catalogue(endpoint, "access-token", "koma-uuid");

    assert_eq!(rest.models_cache_failed, None);
    let pending = rest.catalogue_pending.expect("retry must be scheduled");
    assert_eq!(pending.endpoint, endpoint);
    assert_eq!(pending.oauth_uuid, "koma-uuid");
}
