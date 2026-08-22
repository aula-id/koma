use std::cell::Cell;
use std::collections::VecDeque;

use super::*;

fn run_bootstrap(
    outputs: &[&str],
    installs: &Cell<usize>,
    confirm: impl FnMut(&str) -> Result<bool>,
) -> Result<bool> {
    let mut outputs: VecDeque<String> =
        outputs.iter().map(|value| (*value).to_string()).collect();
    ensure_compatible_with(
        "0.3.16",
        || {
            outputs
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("unexpected version query"))
        },
        || {
            installs.set(installs.get() + 1);
            Ok(())
        },
        |_| {},
        confirm,
    )
}

#[test]
fn matching_version_skips_install() {
    let installs = Cell::new(0);
    assert!(!run_bootstrap(&["koma 0.3.16"], &installs, |_| Ok(true)).unwrap());
    assert_eq!(installs.get(), 0);
}

#[test]
fn missing_koma_installs_without_confirm() {
    let installs = Cell::new(0);
    let confirmed = Cell::new(0);
    assert!(run_bootstrap(&["MISSING", "koma 0.3.16"], &installs, |_| {
        confirmed.set(confirmed.get() + 1);
        Ok(true)
    })
    .unwrap());
    assert_eq!(installs.get(), 1);
    assert_eq!(confirmed.get(), 0, "missing install must not prompt");
}

#[test]
fn mismatched_version_installs_when_confirmed() {
    let installs = Cell::new(0);
    let confirmed = Cell::new(0);
    assert!(run_bootstrap(&["koma 0.3.15", "koma 0.3.16"], &installs, |obs| {
        assert_eq!(obs, "0.3.15");
        confirmed.set(confirmed.get() + 1);
        Ok(true)
    })
    .unwrap());
    assert_eq!(installs.get(), 1);
    assert_eq!(confirmed.get(), 1);
}

#[test]
fn mismatched_version_declined_skips_install() {
    let installs = Cell::new(0);
    let err = run_bootstrap(&["koma 0.3.15"], &installs, |_| Ok(false))
        .unwrap_err()
        .to_string();
    assert_eq!(installs.get(), 0);
    assert!(err.contains("update declined"));
    assert!(err.contains("0.3.15"));
}

#[test]
fn post_install_mismatch_errors() {
    let installs = Cell::new(0);
    let error = run_bootstrap(&["koma 0.3.15", "koma 0.3.14"], &installs, |_| Ok(true))
        .unwrap_err()
        .to_string();
    assert_eq!(installs.get(), 1);
    assert!(error.contains("expected 0.3.16"));
    assert!(error.contains("observed 0.3.14"));
}

#[test]
fn query_error_treated_as_missing_triggers_install() {
    let installs = Cell::new(0);
    let mut probe_results: VecDeque<Result<String>> = vec![
        Err(anyhow::anyhow!(
            "binfmt exec failed: x86_64-binfmt-P: Could not open '/lib64/ld-linux-x86-64.so.2'"
        )),
        Ok("koma 0.3.16".to_string()),
    ]
    .into();
    let result = ensure_compatible_with(
        "0.3.16",
        || {
            probe_results
                .pop_front()
                .unwrap_or_else(|| Err(anyhow::anyhow!("unexpected version query")))
        },
        || {
            installs.set(installs.get() + 1);
            Ok(())
        },
        |_| {},
        |_| Ok(true),
    );
    assert!(result.unwrap());
    assert_eq!(installs.get(), 1);
}

#[test]
fn query_error_post_install_also_errors_propagates() {
    let installs = Cell::new(0);
    let mut probe_results: VecDeque<Result<String>> = vec![
        Err(anyhow::anyhow!("connection refused")),
        Err(anyhow::anyhow!("connection refused")),
    ]
    .into();
    let result = ensure_compatible_with(
        "0.3.16",
        || {
            probe_results
                .pop_front()
                .unwrap_or_else(|| Err(anyhow::anyhow!("unexpected version query")))
        },
        || {
            installs.set(installs.get() + 1);
            Ok(())
        },
        |_| {},
        |_| Ok(true),
    );
    let err = result.unwrap_err().to_string();
    assert!(err.contains("version mismatch after install"));
    assert_eq!(installs.get(), 1);
}

#[test]
fn parses_and_normalizes_actual_cli_version_output() {
    assert_eq!(
        parse_version_output("\n  koma 0.3.16+release.7  \n"),
        RemoteVersion::Version(SemanticVersion {
            major: 0,
            minor: 3,
            patch: 16,
            prerelease: None,
        })
    );
    assert_eq!(
        parse_version_output("koma 0.3.16-rc.1+build.9"),
        RemoteVersion::Version(SemanticVersion {
            major: 0,
            minor: 3,
            patch: 16,
            prerelease: Some("rc.1".to_string()),
        })
    );
    assert!(matches!(
        parse_version_output("koma version 0.3.16"),
        RemoteVersion::Unrecognized(_)
    ));
}
