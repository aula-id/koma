//! Remote koma bootstrap: validate the remote version and install or upgrade it.

use std::fmt;

use anyhow::Result;

use super::auth::SshAuth;
use super::ssh;
use super::RemoteTarget;

const MISSING: &str = "MISSING";
const VERSION_COMMAND: &str =
    "if command -v koma >/dev/null 2>&1; then koma --version; else echo MISSING; fi";

#[derive(Clone, Debug, Eq, PartialEq)]
struct SemanticVersion {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Option<String>,
}

impl fmt::Display for SemanticVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(prerelease) = &self.prerelease {
            write!(f, "-{prerelease}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RemoteVersion {
    Missing,
    Version(SemanticVersion),
    Unrecognized(String),
}

impl fmt::Display for RemoteVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("missing"),
            Self::Version(version) => version.fmt(f),
            Self::Unrecognized(output) => write!(f, "unrecognized output {output:?}"),
        }
    }
}

fn parse_semantic_version(value: &str) -> Option<SemanticVersion> {
    let (without_build, build) = value
        .split_once('+')
        .map_or((value, None), |(version, build)| (version, Some(build)));
    if build.is_some_and(|value| !valid_identifiers(value, false)) {
        return None;
    }

    let (core, prerelease) = without_build
        .split_once('-')
        .map_or((without_build, None), |(core, prerelease)| {
            (core, Some(prerelease))
        });
    if prerelease.is_some_and(|value| !valid_identifiers(value, true)) {
        return None;
    }

    let mut components = core.split('.');
    let major = parse_core_number(components.next()?)?;
    let minor = parse_core_number(components.next()?)?;
    let patch = parse_core_number(components.next()?)?;
    if components.next().is_some() {
        return None;
    }

    Some(SemanticVersion {
        major,
        minor,
        patch,
        prerelease: prerelease.map(str::to_string),
    })
}

fn parse_core_number(value: &str) -> Option<u64> {
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) {
        return None;
    }
    value.parse().ok()
}

fn valid_identifiers(value: &str, reject_numeric_leading_zero: bool) -> bool {
    !value.is_empty()
        && value.split('.').all(|identifier| {
            !identifier.is_empty()
                && identifier
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && !(reject_numeric_leading_zero
                    && identifier.len() > 1
                    && identifier.starts_with('0')
                    && identifier.bytes().all(|byte| byte.is_ascii_digit()))
        })
}

/// Parse the actual CLI format emitted by `koma --version`: `koma <semver>`.
fn parse_version_output(output: &str) -> RemoteVersion {
    let output = output.trim();
    if output == MISSING {
        return RemoteVersion::Missing;
    }

    let mut words = output.split_whitespace();
    let parsed = match (words.next(), words.next(), words.next()) {
        (Some("koma"), Some(version), None) => parse_semantic_version(version),
        _ => None,
    };
    parsed.map_or_else(
        || RemoteVersion::Unrecognized(output.to_string()),
        RemoteVersion::Version,
    )
}

fn ensure_compatible_with<Q, I>(local: &str, mut query: Q, mut install: I) -> Result<bool>
where
    Q: FnMut() -> Result<String>,
    I: FnMut() -> Result<()>,
{
    let expected = parse_semantic_version(local).ok_or_else(|| {
        anyhow::anyhow!("local Koma version is not valid semantic version: {local:?}")
    })?;
    let observed = parse_version_output(&query()?);
    if observed == RemoteVersion::Version(expected.clone()) {
        return Ok(false);
    }

    install()?;
    let observed = parse_version_output(&query()?);
    if observed != RemoteVersion::Version(expected.clone()) {
        anyhow::bail!(
            "remote Koma version mismatch after install: expected {expected}, observed {observed}"
        );
    }
    Ok(true)
}

/// Ensure the remote Koma is installed and exactly matches this running client.
///
/// Returns `true` when the installer was run and `false` when the existing
/// remote version was already compatible.
pub(crate) fn ensure_koma_compatible(
    target: &RemoteTarget,
    auth: Option<&SshAuth>,
) -> Result<bool> {
    ensure_compatible_with(
        env!("CARGO_PKG_VERSION"),
        || ssh::exec_remote(target, VERSION_COMMAND, auth),
        || install_koma(target, auth),
    )
}

/// Install or upgrade koma on the remote machine using the official installer.
fn install_koma(target: &RemoteTarget, auth: Option<&SshAuth>) -> Result<()> {
    let cmd = "curl -fsSL https://koma.run/install.sh | sh";
    ssh::exec_remote(target, cmd, auth)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::VecDeque;

    use super::*;

    fn run_bootstrap(outputs: &[&str], installs: &Cell<usize>) -> Result<bool> {
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
        )
    }

    #[test]
    fn matching_version_skips_install() {
        let installs = Cell::new(0);
        assert!(!run_bootstrap(&["koma 0.3.16"], &installs).unwrap());
        assert_eq!(installs.get(), 0);
    }

    #[test]
    fn missing_koma_installs() {
        let installs = Cell::new(0);
        assert!(run_bootstrap(&["MISSING", "koma 0.3.16"], &installs).unwrap());
        assert_eq!(installs.get(), 1);
    }

    #[test]
    fn mismatched_version_installs() {
        let installs = Cell::new(0);
        assert!(run_bootstrap(&["koma 0.3.15", "koma 0.3.16"], &installs).unwrap());
        assert_eq!(installs.get(), 1);
    }

    #[test]
    fn post_install_mismatch_errors() {
        let installs = Cell::new(0);
        let error = run_bootstrap(&["koma 0.3.15", "koma 0.3.14"], &installs)
            .unwrap_err()
            .to_string();
        assert_eq!(installs.get(), 1);
        assert!(error.contains("expected 0.3.16"));
        assert!(error.contains("observed 0.3.14"));
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
}
