//! Self-update awareness: know the compiled-in koma version and, on every new
//! session spawn, fire a NON-BLOCKING check against the GitHub release.
//!
//! The release workflow stamps `release.json` (`version` + commit `sha`) onto
//! the GitHub release. A stable build reads that file from the latest release.
//! A preview build (`0.4.7-preview.2`) reads the release list and picks the
//! highest semver, including prereleases. If the stamped file is missing, the
//! tag name on the GitHub API is enough for the version compare.
//!
//! [`spawn_check`] mirrors `tool::internet::http_get_blocking`'s concurrency
//! pattern: `reqwest::blocking` panics if run on a tokio runtime thread, so the
//! whole fetch happens inside a freshly spawned `std::thread`. The result is
//! handed back over a `tokio::sync::mpsc` sender, drained in the event-loop
//! tick and stashed in `AppStateRest::latest_version`.
//!
//! GRACEFUL DEGRADATION: a network failure, a non-success status, or a body
//! that doesn't parse does nothing. The app shows only the current version.

use std::cmp::Ordering;
use std::time::Duration;

/// Latest stable release manifest, written by the release workflow.
pub(crate) const RELEASE_MANIFEST_URL: &str =
    "https://github.com/aula-id/koma/releases/latest/download/release.json";

const RELEASES_URL: &str = "https://api.github.com/repos/aula-id/koma/releases?per_page=30";
const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/aula-id/koma/releases/latest";

/// HTTP timeout for one blocking request. The check must never delay the session.
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// A published koma release. `sha` is the commit the release workflow stamped.
#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
pub struct VersionInfo {
    pub version: String,
    pub message: Option<String>,
    #[serde(default)]
    pub sha: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Semver {
    core: Vec<u64>,
    pre: Vec<Ident>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Ident {
    Num(u64),
    Text(String),
}

/// `latest` is strictly newer than `current` under semver.
///
/// A leading `v` is ignored. `1.2` and `1.2.0` are equal. A pre-release is older
/// than the same core version: `0.4.7-preview.2` < `0.4.7` < `0.4.8-preview.1`.
/// Unparseable input is not newer.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_semver(latest), parse_semver(current)) {
        (Some(latest), Some(current)) => cmp_semver(&latest, &current) == Ordering::Greater,
        _ => false,
    }
}

fn is_same(latest: &str, current: &str) -> bool {
    match (parse_semver(latest), parse_semver(current)) {
        (Some(latest), Some(current)) => cmp_semver(&latest, &current) == Ordering::Equal,
        _ => false,
    }
}

/// A release the running binary should install.
///
/// A newer semver always counts. The same semver counts only when both commits
/// are known and differ, which is a rebuilt tag. A local build has no commit
/// (`unknown`) and is compared by version only.
pub fn release_supersedes(latest: &VersionInfo, current_version: &str, current_sha: &str) -> bool {
    if is_newer(&latest.version, current_version) {
        return true;
    }
    if !is_same(&latest.version, current_version) {
        return false;
    }
    match (&latest.sha, normalize_sha(current_sha)) {
        (Some(remote), Some(local)) => remote != &local,
        _ => false,
    }
}

/// Fetch the release this build should compare against. Stable builds ignore
/// prereleases. Preview builds consider them.
pub fn fetch_latest() -> Result<VersionInfo, ()> {
    let client = reqwest::blocking::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent(concat!("koma/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| ())?;
    let current = crate::model::store::current_version();
    if is_preview(current) {
        fetch_preview(&client, current)
    } else {
        fetch_stable(&client)
    }
}

fn fetch_stable(client: &reqwest::blocking::Client) -> Result<VersionInfo, ()> {
    if let Ok(info) = fetch_manifest(client, RELEASE_MANIFEST_URL) {
        if !is_preview(&info.version) {
            return Ok(info);
        }
    }
    let release: GhRelease =
        serde_json::from_str(&get_text(client, LATEST_RELEASE_URL)?).map_err(|_| ())?;
    if release.draft || release.prerelease {
        return Err(());
    }
    Ok(from_tag(&release.tag_name))
}

fn fetch_preview(client: &reqwest::blocking::Client, current: &str) -> Result<VersionInfo, ()> {
    let releases: Vec<GhRelease> =
        serde_json::from_str(&get_text(client, RELEASES_URL)?).map_err(|_| ())?;
    let chosen = choose_release(&releases, current).ok_or(())?;
    let url = format!(
        "https://github.com/aula-id/koma/releases/download/{}/release.json",
        chosen.tag_name
    );
    if let Ok(info) = fetch_manifest(client, &url) {
        return Ok(info);
    }
    Ok(from_tag(&chosen.tag_name))
}

fn fetch_manifest(client: &reqwest::blocking::Client, url: &str) -> Result<VersionInfo, ()> {
    parse_manifest(&get_text(client, url)?)
}

fn get_text(client: &reqwest::blocking::Client, url: &str) -> Result<String, ()> {
    let mut request = client.get(url);
    if url.contains("api.github.com") {
        request = request.header("Accept", "application/vnd.github+json");
    }
    let response = request.send().map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    response.text().map_err(|_| ())
}

#[derive(Debug, serde::Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

fn from_tag(tag: &str) -> VersionInfo {
    VersionInfo {
        version: canonical_version(tag),
        message: None,
        sha: None,
    }
}

fn choose_release<'a>(releases: &'a [GhRelease], current: &str) -> Option<&'a GhRelease> {
    let preview_channel = is_preview(current);
    releases
        .iter()
        .filter(|release| !release.draft && parse_semver(&release.tag_name).is_some())
        .filter(|release| preview_channel || !release.prerelease && !is_preview(&release.tag_name))
        .max_by(|left, right| cmp_version(&left.tag_name, &right.tag_name))
}

fn is_preview(version: &str) -> bool {
    canonical_version(version).contains('-')
}

/// "preview" for a pre-release version (contains '-'), else "stable".
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub fn channel(version: &str) -> &'static str {
    if is_preview(version) {
        "preview"
    } else {
        "stable"
    }
}

/// First 7 hex chars of a stamped commit, or "unknown".
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub fn short_sha(sha: &str) -> String {
    let sha = sha.trim();
    match sha.get(..7) {
        Some(head) if sha.bytes().all(|byte| byte.is_ascii_hexdigit()) => head.to_ascii_lowercase(),
        _ => "unknown".to_string(),
    }
}

/// UTC "YYYY-MM-DD" for a unix-seconds string; None when not a number.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub fn format_build_date(epoch: &str) -> Option<String> {
    let secs: i64 = epoch.trim().parse().ok().filter(|secs| *secs >= 0)?;
    // Civil-from-days (Howard Hinnant), days counted from 0000-03-01.
    let z = secs / 86_400 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// Pure assembly so tests can pass any inputs.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub fn build_info_json_from(
    version: &str,
    sha: &str,
    target: &str,
    epoch: &str,
    os: &str,
) -> String {
    serde_json::json!({
        "version": version,
        "channel": channel(version),
        "sha": short_sha(sha),
        "target": target,
        "os": os,
        "buildDate": format_build_date(epoch),
    })
    .to_string()
}

/// Compile-time facts of THIS binary, as JSON for `window.__komaBuild`.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub fn build_info_json() -> String {
    build_info_json_from(
        env!("CARGO_PKG_VERSION"),
        env!("KOMA_GIT_SHA"),
        env!("KOMA_TERM_TARGET"),
        env!("KOMA_BUILD_EPOCH"),
        std::env::consts::OS,
    )
}

fn canonical_version(raw: &str) -> String {
    let trimmed = raw.trim();
    trimmed.strip_prefix('v').unwrap_or(trimmed).to_string()
}

fn normalize_sha(raw: &str) -> Option<String> {
    let sha = raw.trim().to_ascii_lowercase();
    (sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(sha)
}

fn parse_manifest(body: &str) -> Result<VersionInfo, ()> {
    let mut info: VersionInfo = serde_json::from_str(body).map_err(|_| ())?;
    info.version = canonical_version(&info.version);
    if parse_semver(&info.version).is_none() {
        return Err(());
    }
    info.sha = info.sha.as_deref().and_then(normalize_sha);
    Ok(info)
}

fn parse_semver(raw: &str) -> Option<Semver> {
    let raw = canonical_version(raw);
    let (core, pre) = match raw.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (raw.as_str(), None),
    };
    let core = core.split_once('+').map(|(core, _)| core).unwrap_or(core);
    if core.is_empty() {
        return None;
    }
    let core = core
        .split('.')
        .map(parse_number)
        .collect::<Option<Vec<_>>>()?;
    let pre = match pre {
        None => Vec::new(),
        Some(pre) => {
            let pre = pre.split_once('+').map(|(pre, _)| pre).unwrap_or(pre);
            if pre.is_empty() {
                return None;
            }
            pre.split('.')
                .map(parse_ident)
                .collect::<Option<Vec<_>>>()?
        }
    };
    Some(Semver { core, pre })
}

fn parse_number(raw: &str) -> Option<u64> {
    if raw.is_empty() || (raw.len() > 1 && raw.starts_with('0')) {
        return None;
    }
    raw.parse().ok()
}

fn parse_ident(raw: &str) -> Option<Ident> {
    if raw.is_empty()
        || !raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return None;
    }
    match parse_number(raw) {
        Some(number) if raw.bytes().all(|byte| byte.is_ascii_digit()) => Some(Ident::Num(number)),
        _ if raw.bytes().all(|byte| byte.is_ascii_digit()) => None,
        _ => Some(Ident::Text(raw.to_string())),
    }
}

fn cmp_version(left: &str, right: &str) -> Ordering {
    match (parse_semver(left), parse_semver(right)) {
        (Some(left), Some(right)) => cmp_semver(&left, &right),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
}

fn cmp_semver(left: &Semver, right: &Semver) -> Ordering {
    let len = left.core.len().max(right.core.len());
    for index in 0..len {
        let order = left
            .core
            .get(index)
            .unwrap_or(&0)
            .cmp(right.core.get(index).unwrap_or(&0));
        if order != Ordering::Equal {
            return order;
        }
    }
    match (left.pre.is_empty(), right.pre.is_empty()) {
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ => {
            let len = left.pre.len().max(right.pre.len());
            for index in 0..len {
                match (left.pre.get(index), right.pre.get(index)) {
                    (None, Some(_)) => return Ordering::Less,
                    (Some(_), None) => return Ordering::Greater,
                    (Some(Ident::Num(left)), Some(Ident::Num(right))) => {
                        let order = left.cmp(right);
                        if order != Ordering::Equal {
                            return order;
                        }
                    }
                    (Some(Ident::Text(left)), Some(Ident::Text(right))) => {
                        let order = left.cmp(right);
                        if order != Ordering::Equal {
                            return order;
                        }
                    }
                    // Semver: a numeric identifier is older than a non-numeric one.
                    (Some(Ident::Num(_)), Some(Ident::Text(_))) => return Ordering::Less,
                    (Some(Ident::Text(_)), Some(Ident::Num(_))) => return Ordering::Greater,
                    (None, None) => {}
                }
            }
            Ordering::Equal
        }
    }
}

/// Spawn a NON-BLOCKING background check for a newer koma release.
pub fn spawn_check(tx: tokio::sync::mpsc::UnboundedSender<VersionInfo>) {
    spawn_check_unless(update_check_disabled(), tx);
}

/// Start the fetch thread unless `disabled`. A disabled check drops `tx` at once
/// and makes no network request.
fn spawn_check_unless(disabled: bool, tx: tokio::sync::mpsc::UnboundedSender<VersionInfo>) {
    if disabled {
        return;
    }
    std::thread::spawn(move || {
        if let Ok(info) = fetch_latest() {
            let _ = tx.send(info);
        }
    });
}

/// `KOMA_NO_UPDATE_CHECK=1` turns the update check off in every build, for hosts
/// with no network (containers, on-prem). Debug builds also honour
/// `KOMA_TEST_OFFLINE=1`, which tests and smokes use for the same purpose.
fn update_check_disabled() -> bool {
    let opted_out = offline_switch_set(std::env::var_os("KOMA_NO_UPDATE_CHECK").as_deref());
    #[cfg(debug_assertions)]
    let opted_out =
        opted_out || offline_switch_set(std::env::var_os("KOMA_TEST_OFFLINE").as_deref());
    opted_out
}

fn offline_switch_set(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|value| value == "1")
}

/// Minimum gap between attach-triggered checks, so a reconnect storm sends one request.
const REENTRY_GAP: Duration = Duration::from_secs(30);

/// [`spawn_check`] for a client attach. Skipped when another attach checked within
/// [`REENTRY_GAP`]; session spawns keep calling [`spawn_check`] directly.
pub fn spawn_check_on_reentry(tx: tokio::sync::mpsc::UnboundedSender<VersionInfo>) {
    if update_check_disabled() {
        return;
    }
    use std::sync::Mutex;
    use std::time::Instant;
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if last.is_some_and(|at| at.elapsed() < REENTRY_GAP) {
        return;
    }
    *last = Some(Instant::now());
    drop(last);
    spawn_check(tx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_date_formats_utc_days() {
        assert_eq!(format_build_date("0").as_deref(), Some("1970-01-01"));
        assert_eq!(
            format_build_date("951782400").as_deref(),
            Some("2000-02-29")
        );
        assert_eq!(
            format_build_date("1709164800").as_deref(),
            Some("2024-02-29")
        );
        assert_eq!(
            format_build_date("1767225599").as_deref(),
            Some("2025-12-31")
        );
        assert_eq!(
            format_build_date("1767225600").as_deref(),
            Some("2026-01-01")
        );
        for bad in ["abc", "", "-5", "unknown"] {
            assert_eq!(format_build_date(bad), None, "{bad}");
        }
    }

    #[test]
    fn short_sha_takes_seven_hex_or_unknown() {
        assert_eq!(
            short_sha("ABCDEF0123456789abcdef0123456789abcdef01"),
            "abcdef0"
        );
        assert_eq!(short_sha("unknown"), "unknown");
        assert_eq!(short_sha(""), "unknown");
        assert_eq!(short_sha("ZZZZZZZZ"), "unknown");
    }

    #[test]
    fn channel_follows_prerelease_suffix() {
        assert_eq!(channel("0.4.15"), "stable");
        assert_eq!(channel("0.4.15-preview.2"), "preview");
        assert_eq!(channel("v0.4.15"), "stable");
    }

    #[test]
    fn build_info_json_assembles_fields() {
        let raw = build_info_json_from(
            "0.4.15",
            "unknown",
            "x86_64-pc-windows-msvc",
            "unknown",
            "windows",
        );
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["sha"], "unknown");
        assert_eq!(v["target"], "x86_64-pc-windows-msvc");
        assert_eq!(v["os"], "windows");
        assert_eq!(v["channel"], "stable");
        assert!(v["buildDate"].is_null());

        let sha = "0123456789abcdef0123456789abcdef01234567";
        let raw = build_info_json_from(
            "0.4.15-preview.1",
            sha,
            "aarch64-apple-darwin",
            "1767225600",
            "macos",
        );
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["buildDate"], "2026-01-01");
        assert_eq!(v["sha"], "0123456");
        assert_eq!(v["channel"], "preview");
    }

    #[test]
    fn build_info_json_escapes_hostile_values() {
        let raw = build_info_json_from(
            "1.0\"\\\n",
            "zz",
            "tar\"get\\\r\n\u{2028}",
            "-5",
            "o\"s",
        );
        // Raw newlines or quotes would end the JS string/statement early in
        // `window.__komaBuild=<json>;`.
        assert!(!raw.contains('\n') && !raw.contains('\r'));
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["target"], "tar\"get\\\r\n\u{2028}");
        assert_eq!(v["os"], "o\"s");
        assert!(v["buildDate"].is_null());
        assert_eq!(v["sha"], "unknown");
    }

    fn release(tag: &str, prerelease: bool) -> GhRelease {
        GhRelease {
            tag_name: tag.into(),
            draft: false,
            prerelease,
        }
    }

    #[test]
    fn semver_orders_previews_under_the_release() {
        assert!(is_newer("v0.4.6", "0.4.5"));
        assert!(!is_newer("0.4.5", "0.4.5"));
        assert!(!is_newer("0.4.4", "0.4.5"));
        assert!(!is_newer("1.2", "1.2.0"));
        assert!(is_newer("1.10", "1.9.9"));
        assert!(is_newer("0.4.7", "0.4.7-preview.2"));
        assert!(!is_newer("0.4.7-preview.2", "0.4.7"));
        assert!(is_newer("0.4.7-preview.2", "0.4.7-preview.1"));
        assert!(is_newer("0.4.7-preview.10", "0.4.7-preview.2"));
        assert!(is_newer("0.4.8-preview.1", "0.4.7"));
        assert!(!is_newer("not-a-version", "0.4.7"));
    }

    #[test]
    fn manifest_keeps_a_stamped_sha_and_rejects_a_bad_one() {
        let info = parse_manifest(
            r#"{"version":"v0.4.7","sha":"ABCDEF0123456789ABCDEF0123456789ABCDEF01"}"#,
        )
        .unwrap();
        assert_eq!(info.version, "0.4.7");
        assert_eq!(
            info.sha.as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef01")
        );
        let info = parse_manifest(r#"{"version":"0.4.7-preview.2","sha":"nope"}"#).unwrap();
        assert_eq!(info.version, "0.4.7-preview.2");
        assert_eq!(info.sha, None);
        assert!(parse_manifest("broken").is_err());
        assert!(parse_manifest(r#"{"version":""}"#).is_err());
    }

    #[test]
    fn a_rebuilt_release_supersedes_the_same_version() {
        let same = VersionInfo {
            version: "0.4.7".into(),
            message: None,
            sha: Some("a".repeat(40)),
        };
        assert!(!release_supersedes(&same, "0.4.7", "unknown"));
        assert!(!release_supersedes(&same, "0.4.7", &"a".repeat(40)));
        assert!(release_supersedes(&same, "0.4.7", &"b".repeat(40)));
        assert!(release_supersedes(&same, "0.4.6", "unknown"));
        assert!(!release_supersedes(&same, "0.4.8", &"b".repeat(40)));
    }

    #[test]
    fn stable_channel_skips_previews_and_preview_channel_takes_the_highest() {
        let releases = vec![
            release("v0.4.6", false),
            release("v0.4.7-preview.2", true),
            release("v0.4.7", false),
            GhRelease {
                tag_name: "v0.4.9".into(),
                draft: true,
                prerelease: false,
            },
        ];
        assert_eq!(
            choose_release(&releases, "0.4.6").unwrap().tag_name,
            "v0.4.7"
        );
        assert_eq!(
            choose_release(&releases, "0.4.7-preview.1")
                .unwrap()
                .tag_name,
            "v0.4.7"
        );
        let ahead = vec![release("v0.4.7", false), release("v0.4.8-preview.1", true)];
        assert_eq!(
            choose_release(&ahead, "0.4.7-preview.1").unwrap().tag_name,
            "v0.4.8-preview.1"
        );
    }

    #[test]
    fn the_offline_switch_needs_exactly_one() {
        assert!(offline_switch_set(Some("1".as_ref())));
        assert!(!offline_switch_set(None));
        assert!(!offline_switch_set(Some("".as_ref())));
        assert!(!offline_switch_set(Some("0".as_ref())));
        assert!(!offline_switch_set(Some("true".as_ref())));
    }

    #[test]
    fn a_disabled_check_starts_no_fetch_thread() {
        // A started thread would hold the sender until its fetch ends, so the
        // channel would still be open here.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        spawn_check_unless(offline_switch_set(Some("1".as_ref())), tx);
        assert_eq!(
            rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
        );
    }

    #[test]
    fn snapshot_only_advertises_newer_versions() {
        let mut state = crate::app::state::AppState::new(crate::app::mode::Mode::Chat);
        let current = env!("CARGO_PKG_VERSION");
        for version in [None, Some(current), Some("0.0.0"), Some("9999.0.0")] {
            state.rest.latest_version = version.map(|v| VersionInfo {
                version: v.into(),
                message: None,
                sha: None,
            });
            let snap = crate::ipc::snapshot::build_snapshot(&state);
            assert_eq!(snap.global.current_version, current);
            assert_eq!(
                snap.global.latest_version.as_deref(),
                version.filter(|v| is_newer(v, current))
            );
        }
    }
}
