//! Remote koma bootstrap: validate the remote version and install or upgrade it.

use std::fmt;

use anyhow::Result;

use super::auth::SshAuth;
use super::ssh;
use super::RemoteTarget;

const MISSING: &str = "MISSING";
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

    // Treat a probe failure (SSH error, broken binary, QEMU/binfmt, etc.) as
    // "missing" so we fall through to the install path instead of aborting.
    let observed = match query() {
        Ok(output) => parse_version_output(&output),
        Err(_) => RemoteVersion::Missing,
    };
    if observed == RemoteVersion::Version(expected.clone()) {
        return Ok(false);
    }

    install()?;
    let observed = match query() {
        Ok(output) => parse_version_output(&output),
        Err(_) => RemoteVersion::Missing,
    };
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
        || {
            let path = ssh::find_koma(target, auth)?;
            let command = ssh::remote_command(&path, &["--version"])?;
            ssh::exec_remote(
                target,
                &format!("{command} 2>/dev/null || echo {MISSING}"),
                auth,
            )
        },
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
#[path = "bootstrap_test.rs"]
mod tests;
