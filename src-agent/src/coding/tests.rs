//! Test discovery and structured results layered on the supervised task process.
use super::WorkspaceRef;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Profile {
    id: String,
    label: String,
    kind: String,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    discover_args: Vec<String>,
    #[serde(default)]
    debug_profile: Option<String>,
}
fn profiles(root: &Path) -> Result<(Vec<Profile>, String)> {
    let config = super::workspace::read_config(root)?;
    let mut profiles: Vec<Profile> =
        serde_json::from_value(config.get("tests").cloned().unwrap_or(json!([])))?;
    let mut add = |id: &str, label: &str, kind: &str| {
        if !profiles.iter().any(|p| p.id == id) {
            profiles.push(Profile {
                id: id.into(),
                label: label.into(),
                kind: kind.into(),
                command: None,
                args: vec![],
                discover_args: vec![],
                debug_profile: None,
            });
        }
    };
    if root.join("Cargo.toml").is_file() {
        add("cargo", "Rust tests", "cargo");
    }
    if root.join("go.mod").is_file() {
        add("go", "Go tests", "go");
    }
    if [
        "pyproject.toml",
        "pytest.ini",
        "setup.py",
        "requirements.txt",
    ]
    .iter()
    .any(|p| root.join(p).is_file())
    {
        add("pytest", "Python / pytest", "pytest");
    }
    if root.join("package.json").is_file() {
        add("node", "Node test runner", "node");
    }
    anyhow::ensure!(profiles.len() <= 50, "At most 50 test profiles");
    let mut ids = std::collections::HashSet::new();
    for p in &profiles {
        anyhow::ensure!(
            !p.id.is_empty()
                && ids.insert(&p.id)
                && matches!(p.kind.as_str(), "cargo" | "go" | "pytest" | "node" | "json"),
            "Invalid test profile"
        );
    }
    let fingerprint = super::environment::fingerprint(&json!(profiles))?;
    Ok((profiles, fingerprint))
}
pub(super) fn definitions(root: &Path) -> Result<Value> {
    let (profiles, fingerprint) = profiles(root)?;
    Ok(json!({"profiles":profiles,"fingerprint":fingerprint}))
}
#[derive(Default)]
struct Results {
    pending: String,
    items: BTreeMap<String, Value>,
    truncated: bool,
    go_names: Vec<String>,
    bytes: usize,
}
impl Results {
    fn record(&mut self, value: Value) {
        let Some(id) = value["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 2048)
            .map(str::to_owned)
        else {
            return;
        };
        if self.items.len() >= 5000 && !self.items.contains_key(&id) {
            self.truncated = true;
            return;
        }
        let mut next = self
            .items
            .get(&id)
            .cloned()
            .unwrap_or_else(|| json!({"id":id,"label":id,"status":"discovered"}));
        for field in ["label", "file", "suite", "message", "status"] {
            if let Some(text) = value[field].as_str() {
                if field == "status" && next["status"] == "failed" && text != "failed" {
                    continue;
                }
                next[field] = json!(text
                    .chars()
                    .take(if field == "message" { 1024 } else { 2048 })
                    .collect::<String>());
            }
        }
        for field in ["line", "durationMs", "nesting"] {
            if value[field].is_number() {
                next[field] = value[field].clone();
            }
        }
        let old_len = self.items.get(&id).map_or(0, |v| v.to_string().len());
        let new_len = next.to_string().len();
        if self.bytes - old_len + new_len > 4 * 1024 * 1024 {
            self.truncated = true;
            return;
        }
        self.bytes = self.bytes - old_len + new_len;
        self.items.insert(id, next);
    }

    fn ingest(&mut self, kind: &str, discover: bool, text: &str) {
        self.pending.push_str(text);
        while let Some(i) = self.pending.find('\n') {
            let line = self.pending[..i].trim_end_matches('\r').to_string();
            self.pending.drain(..=i);
            self.line(kind, discover, &line);
        }
        if self.pending.len() > 65536 {
            self.pending.clear();
            self.truncated = true;
        }
    }
    fn line(&mut self, kind: &str, discover: bool, line: &str) {
        if let Some(raw) = line.strip_prefix("KOMA_TEST ") {
            if let Ok(v) = serde_json::from_str::<Value>(raw) {
                self.record(v);
            }
            return;
        }
        if kind == "go" {
            if discover {
                if line.starts_with("Test")
                    || line.starts_with("Example")
                    || line.starts_with("Fuzz")
                {
                    if self.go_names.len() < 5000 && line.len() <= 2048 {
                        self.go_names.push(line.into());
                    } else {
                        self.truncated = true;
                    }
                } else if line.starts_with("ok\t")
                    || line.starts_with("ok ")
                    || line.starts_with("? ")
                    || line.starts_with("?\t")
                {
                    if let Some(package) = line.split_whitespace().nth(1) {
                        for name in std::mem::take(&mut self.go_names) {
                            self.record(json!({"id":format!("{package}::{name}"),"label":name,"suite":package,"status":"discovered"}));
                        }
                    }
                }
            } else if let Ok(v) = serde_json::from_str::<Value>(line) {
                let test = v["Test"].as_str().unwrap_or("");
                let package = v["Package"].as_str().unwrap_or("");
                if !test.is_empty() {
                    let status = match v["Action"].as_str() {
                        Some("run") => "running",
                        Some("pass") => "passed",
                        Some("fail") => "failed",
                        Some("skip") => "skipped",
                        _ => return,
                    };
                    self.record(json!({"id":format!("{package}::{test}"),"label":test,"suite":package,"status":status,"durationMs":v["Elapsed"].as_f64().map(|n|n*1000.)}));
                }
            }
        } else if kind == "cargo" {
            if discover {
                if let Some(name) = line.strip_suffix(": test") {
                    self.record(json!({"id":name,"status":"discovered"}));
                }
            } else if let Some(rest) = line.strip_prefix("test ") {
                if let Some((name, status)) = rest.rsplit_once(" ... ") {
                    let status = match status {
                        "ok" => "passed",
                        "FAILED" => "failed",
                        v if v.starts_with("ignored") => "skipped",
                        _ => return,
                    };
                    self.record(json!({"id":name,"status":status}));
                }
            }
        }
    }
}
struct Run {
    id: String,
    workspace: WorkspaceRef,
    profile: Profile,
    mode: String,
    task: Option<String>,
    results: Arc<Mutex<Results>>,
}
static RUNS: OnceLock<Mutex<VecDeque<Arc<Run>>>> = OnceLock::new();
fn registry() -> &'static Mutex<VecDeque<Arc<Run>>> {
    RUNS.get_or_init(Default::default)
}
fn find(workspace: &WorkspaceRef, id: &str) -> Result<Arc<Run>> {
    registry()
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.id == id && &r.workspace == workspace)
        .cloned()
        .context("Test run is no longer available in this workspace")
}
fn summary(run: &Run) -> Value {
    let task = run.task.as_ref().map(|id| {
        super::tasks::summary(&run.workspace, id).unwrap_or(json!({"status":"unavailable"}))
    });
    json!({"id":run.id,"profileId":run.profile.id,"label":run.profile.label,"mode":run.mode,"task":task})
}
pub(super) fn runs(workspace: &WorkspaceRef) -> Result<Value> {
    let runs = registry()
        .lock()
        .unwrap()
        .iter()
        .filter(|r| &r.workspace == workspace)
        .cloned()
        .collect::<Vec<_>>();
    Ok(json!(runs.iter().map(|r| summary(r)).collect::<Vec<_>>()))
}
pub(super) fn results(workspace: &WorkspaceRef, id: &str) -> Result<Value> {
    let run = find(workspace, id)?;
    let summary = summary(&run);
    let mut result = run.results.lock().unwrap();
    if summary["task"]["outputComplete"] == true && !result.pending.is_empty() {
        let line = std::mem::take(&mut result.pending);
        result.line(&run.profile.kind, run.mode == "discover", &line);
    }
    Ok(
        json!({"run":summary,"items":result.items.values().collect::<Vec<_>>(),"truncated":result.truncated}),
    )
}
fn node_files(root: &Path) -> Vec<String> {
    ignore::WalkBuilder::new(root)
        .max_depth(Some(20))
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|e| {
            let path = e
                .path()
                .strip_prefix(root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            let filename = e.file_name().to_string_lossy();
            if filename.ends_with(".test.js")
                || filename.ends_with(".test.mjs")
                || filename.ends_with(".test.cjs")
                || filename.ends_with(".spec.js")
                || path.starts_with("test/")
                    && (filename.ends_with(".js") || filename.ends_with(".mjs"))
            {
                Some(path)
            } else {
                None
            }
        })
        .take(5000)
        .collect()
}
fn command(program: &str, args: Vec<String>) -> Value {
    json!({"command":program,"args":args,"group":"test","timeoutMs":3_600_000,"continueOnError":true})
}
fn recipe(root: &Path, p: &Profile, discover: bool, items: &[Value]) -> Result<Vec<Value>> {
    let selected: Vec<String> = items
        .iter()
        .filter_map(|i| i["id"].as_str().map(str::to_owned))
        .collect();
    let program = p.command.as_deref();
    let mut commands = Vec::new();
    match p.kind.as_str() {
        "pytest" => {
            let mut args = vec![
                "-u".into(),
                "-c".into(),
                include_str!("test_runner.py").into(),
                "-q".into(),
            ];
            if discover {
                args.push("--collect-only".into());
            }
            args.extend(p.args.clone());
            if !selected.is_empty() {
                args.push("--".into());
                args.extend(selected);
            }
            commands.push(command(
                program.unwrap_or(if cfg!(windows) { "python" } else { "python3" }),
                args,
            ));
        }
        "go" => {
            if discover {
                let mut args = vec!["test".into(), "-list".into(), ".".into()];
                args.extend(p.args.clone());
                args.push("./...".into());
                commands.push(command(program.unwrap_or("go"), args));
            } else if selected.is_empty() {
                let mut args = vec!["test".into(), "-json".into(), "-count=1".into()];
                args.extend(p.args.clone());
                args.push("./...".into());
                commands.push(command(program.unwrap_or("go"), args));
            } else {
                let mut packages: BTreeMap<String, Vec<String>> = BTreeMap::new();
                for item in items {
                    let package = item["suite"].as_str().context("Go test has no package")?;
                    packages.entry(package.into()).or_default().push(
                        item["label"]
                            .as_str()
                            .context("Go test has no name")?
                            .into(),
                    );
                }
                for (package, names) in packages {
                    let pattern = format!(
                        "^({})$",
                        names
                            .iter()
                            .map(|n| regex::escape(n))
                            .collect::<Vec<_>>()
                            .join("|")
                    );
                    let mut args = vec![
                        "test".into(),
                        "-json".into(),
                        "-count=1".into(),
                        "-run".into(),
                        pattern,
                    ];
                    args.extend(p.args.clone());
                    args.push(package);
                    commands.push(command(program.unwrap_or("go"), args));
                }
            }
        }
        "cargo" => {
            let targets: Vec<Option<String>> = if selected.is_empty() {
                vec![None]
            } else {
                selected.into_iter().map(Some).collect()
            };
            for target in targets {
                let mut args = vec!["test".into()];
                args.extend(p.args.clone());
                if let Some(ref target) = target {
                    args.push(target.clone());
                }
                args.push("--".into());
                if discover {
                    args.extend(["--list".into(), "--format".into(), "terse".into()]);
                } else {
                    args.extend(["--color".into(), "never".into()]);
                    if target.is_some() {
                        args.push("--exact".into());
                    }
                }
                commands.push(command(program.unwrap_or("cargo"), args));
            }
        }
        "node" => {
            let dir = crate::model::store::base_dir()?.join("coding/runners");
            std::fs::create_dir_all(&dir)?;
            let reporter = dir.join("node-reporter.mjs");
            super::persistence::atomic_write(&reporter, include_bytes!("test_reporter.mjs"))?;
            let mut args = vec![
                "--test".into(),
                format!("--test-reporter={}", reporter.to_string_lossy()),
            ];
            args.extend(p.args.clone());
            let files = if items.is_empty() {
                node_files(root)
            } else {
                items
                    .iter()
                    .filter_map(|i| i["file"].as_str().map(str::to_owned))
                    .collect()
            };
            let mut seen = std::collections::HashSet::new();
            for file in files {
                let path = root.join(&file).canonicalize()?;
                anyhow::ensure!(
                    path.starts_with(root) && path.is_file(),
                    "Test file is outside workspace"
                );
                if seen.insert(path.clone()) {
                    args.push(path.to_string_lossy().into_owned());
                }
            }
            commands.push(command(program.unwrap_or("node"), args));
        }
        "json" => {
            let mut args = if discover {
                p.discover_args.clone()
            } else {
                p.args.clone()
            };
            if !discover {
                args.extend(selected);
            }
            commands.push(command(
                program.context("Custom test runner requires command")?,
                args,
            ));
        }
        _ => unreachable!(),
    }
    anyhow::ensure!(commands.len() <= 100, "Select at most 100 test invocations");
    Ok(commands)
}
pub(super) fn start(
    workspace: &WorkspaceRef,
    root: &Path,
    profile_id: &str,
    fingerprint: &str,
    mode: &str,
    previous: Option<&str>,
    selection: &[String],
) -> Result<Value> {
    anyhow::ensure!(
        matches!(mode, "discover" | "run" | "failed"),
        "Invalid test operation"
    );
    anyhow::ensure!(selection.len() <= 5000, "Too many selected tests");
    let (profiles, current) = profiles(root)?;
    anyhow::ensure!(
        current == fingerprint,
        "Test definitions changed; refresh before starting"
    );
    let profile = profiles
        .into_iter()
        .find(|p| p.id == profile_id)
        .context("Unknown test profile")?;
    let mut items = Vec::new();
    if !selection.is_empty() || mode == "failed" {
        let previous = find(
            workspace,
            previous.context("Discover or run tests before selecting cases")?,
        )?;
        anyhow::ensure!(
            previous.profile.id == profile.id,
            "Selected tests belong to a different profile"
        );
        let result = previous.results.lock().unwrap();
        if mode == "failed" {
            items.extend(
                result
                    .items
                    .values()
                    .filter(|v| v["status"] == "failed")
                    .cloned(),
            );
            anyhow::ensure!(!items.is_empty(), "No failed tests to rerun");
        } else {
            for id in selection {
                items.push(
                    result
                        .items
                        .get(id)
                        .context("Selected test no longer exists")?
                        .clone(),
                );
            }
        }
    }
    let discover = mode == "discover";
    let results = Arc::new(Mutex::new(Results::default()));
    let id = uuid::Uuid::new_v4().to_string();
    let task = if discover && profile.kind == "node" {
        for file in node_files(root) {
            results
                .lock()
                .unwrap()
                .record(json!({"id":file,"label":file,"file":file,"status":"discovered"}));
        }
        None
    } else {
        let commands = recipe(root, &profile, discover, &items)?;
        let collector = results.clone();
        let kind = profile.kind.clone();
        let observer = Arc::new(move |stream: &str, text: &str| {
            if stream == "stdout" {
                collector.lock().unwrap().ingest(&kind, discover, text);
            }
        });
        let task = super::tasks::run_observed(
            workspace,
            root,
            &format!("test:{id}"),
            &profile.label,
            commands,
            Some(observer),
        )?;
        Some(task["id"].as_str().unwrap().to_string())
    };
    let run = Arc::new(Run {
        id,
        workspace: workspace.clone(),
        profile,
        mode: mode.into(),
        task,
        results,
    });
    let value = summary(&run);
    let mut registry = registry().lock().unwrap();
    while registry.len() >= 64 {
        let index = registry
            .iter()
            .position(|r| {
                let value = summary(r);
                !matches!(
                    value["task"]["status"].as_str(),
                    Some("queued" | "running" | "stopping")
                )
            })
            .context("Test history is full")?;
        registry.remove(index);
    }
    registry.push_back(run);
    Ok(value)
}
pub(super) fn stop(workspace: &WorkspaceRef, id: &str) -> Result<Value> {
    let run = find(workspace, id)?;
    if let Some(task) = &run.task {
        super::tasks::stop(workspace, task)?;
    }
    Ok(summary(&run))
}

pub(super) fn debug(
    workspace: &WorkspaceRef,
    root: &Path,
    run_id: &str,
    item_id: &str,
    breakpoints: &Value,
) -> Result<Value> {
    let run = find(workspace, run_id)?;
    let (profiles, _) = profiles(root)?;
    let profile = profiles
        .into_iter()
        .find(|p| p.id == run.profile.id)
        .context("Test profile was removed")?;
    let item = run
        .results
        .lock()
        .unwrap()
        .items
        .get(item_id)
        .cloned()
        .context("Test not found")?;
    if let Some(profile_id) = profile.debug_profile {
        let definitions = super::debug::definitions(root)?;
        return super::debug::start(
            workspace,
            root,
            &profile_id,
            definitions["fingerprint"].as_str().unwrap(),
            item["file"].as_str(),
            breakpoints,
        );
    }
    let (adapter, configuration) = match profile.kind.as_str() {
        "pytest" => {
            let mut args = profile.args;
            args.push("--".into());
            args.push(item_id.into());
            (
                "debugpy",
                json!({"module":"pytest","args":args,"console":"internalConsole"}),
            )
        }
        "node" => {
            let file = item["file"].as_str().context("Test has no source file")?;
            let file = root.join(file).canonicalize()?;
            anyhow::ensure!(
                file.starts_with(root) && file.is_file(),
                "Test file is outside workspace"
            );
            (
                "js-debug",
                json!({"type":"pwa-node","program":file,"runtimeArgs":["--test","--test-concurrency=1"],"console":"internalConsole"}),
            )
        }
        "go" => {
            let package = item["suite"].as_str().context("Go test has no package")?;
            let manifest = std::fs::read_to_string(root.join("go.mod"))?;
            let module = manifest
                .lines()
                .find_map(|l| l.trim().strip_prefix("module "))
                .context("Missing Go module path")?
                .trim();
            let relative = if package == module {
                ""
            } else {
                package
                    .strip_prefix(&format!("{module}/"))
                    .context("Test belongs to a different Go module")?
            };
            let program = root.join(relative).canonicalize()?;
            anyhow::ensure!(
                program.starts_with(root),
                "Go test package is outside workspace"
            );
            (
                "delve",
                json!({"mode":"test","program":program,"args":["-test.run",format!("^{}$",regex::escape(item["label"].as_str().unwrap_or("")))]}),
            )
        }
        _ => anyhow::bail!(
            "Set debugProfile in this test profile to debug its compiled test executable"
        ),
    };
    super::debug::start_custom(
        workspace,
        root,
        json!({"id":format!("test:{}",profile.id),"label":format!("Test: {}",item["label"].as_str().unwrap_or(item_id)),"adapter":adapter,"configuration":configuration}),
        None,
        breakpoints,
    )
}

#[cfg(test)]
mod regression {
    use super::*;
    #[test]
    fn chunk_boundaries_and_teardown_failure_are_preserved() {
        let mut results = Results::default();
        results.ingest("pytest", false, "KOMA_TE");
        results.ingest("pytest",false,"ST {\"id\":\"a.py::test_a\",\"status\":\"passed\"}\nKOMA_TEST {\"id\":\"a.py::test_a\",\"status\":\"failed\"}\n");
        results.ingest(
            "pytest",
            false,
            "KOMA_TEST {\"id\":\"a.py::test_a\",\"status\":\"passed\"}\n",
        );
        assert_eq!(results.items["a.py::test_a"]["status"], "failed");
    }
    #[test]
    fn go_packages_keep_same_test_names_distinct() {
        let mut results = Results::default();
        results.ingest(
            "go",
            true,
            "TestOne\nok\tmodule/a\t0.0s\nTestOne\nok\tmodule/b\t0.0s\n",
        );
        assert_eq!(results.items.len(), 2);
        results.ingest(
            "go",
            false,
            "{\"Action\":\"fail\",\"Package\":\"module/a\",\"Test\":\"TestOne\"}\n",
        );
        assert_eq!(results.items["module/a::TestOne"]["status"], "failed");
        assert_eq!(results.items["module/b::TestOne"]["status"], "discovered");
    }
}
