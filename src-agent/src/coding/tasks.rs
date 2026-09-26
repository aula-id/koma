//! Explicit project tasks, retained independently of chat and UI selection.
use super::WorkspaceRef;
use anyhow::{Context, Result};
use process_wrap::std::{ChildWrapper, CommandWrap, CommandWrapper};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const OUTPUT_BYTES: usize = 1024 * 1024;
const OUTPUT_CHUNKS: usize = 2048;
const MAX_RUNS: usize = 64;
const MAX_ACTIVE: usize = 8;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Task {
    id: String,
    label: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default = "default_group")]
    group: String,
    #[serde(default = "default_cwd")]
    cwd: String,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}
fn default_group() -> String {
    "run".into()
}
fn default_cwd() -> String {
    ".".into()
}

fn load(root: &Path) -> Result<(Vec<Task>, String)> {
    let config = super::workspace::read_config(root)?;
    let tasks: Vec<Task> =
        serde_json::from_value(config.get("tasks").cloned().unwrap_or(json!([])))
            .context("Invalid tasks in .koma/coding.json")?;
    anyhow::ensure!(
        tasks.len() <= 100,
        "At most 100 project tasks are supported"
    );
    let mut ids = std::collections::HashSet::new();
    for task in &tasks {
        anyhow::ensure!(
            serde_json::to_vec(task)?.len() <= 64 * 1024,
            "A task definition must not exceed 64 KiB"
        );
        anyhow::ensure!(
            !task.id.trim().is_empty() && task.id.len() <= 100 && ids.insert(&task.id),
            "Task IDs must be unique and nonempty (up to 100 bytes)"
        );
        anyhow::ensure!(
            !task.label.trim().is_empty() && task.label.len() <= 200,
            "Task labels must be nonempty (up to 200 bytes)"
        );
        anyhow::ensure!(
            !task.command.trim().is_empty()
                && !task.command.contains('\0')
                && task.command.len() <= 32768,
            "Invalid task executable"
        );
        anyhow::ensure!(
            matches!(task.group.as_str(), "run" | "build" | "test"),
            "Task group must be run, build or test"
        );
        anyhow::ensure!(
            task.args.len() <= 256
                && task
                    .args
                    .iter()
                    .all(|a| a.len() <= 32768 && !a.contains('\0')),
            "Invalid task arguments"
        );
        anyhow::ensure!(
            task.env.len() <= 100
                && task
                    .env
                    .iter()
                    .all(|(k, v)| !k.is_empty() && !k.contains(['=', '\0']) && !v.contains('\0')),
            "Invalid task environment"
        );
        anyhow::ensure!(
            task.timeout_ms
                .is_none_or(|v| (100..=86_400_000).contains(&v)),
            "Task timeoutMs must be between 100 and 86400000"
        );
        anyhow::ensure!(
            !Path::new(&task.cwd).is_absolute() && !task.cwd.contains('\0'),
            "Task cwd must be relative to the workspace"
        );
    }
    let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&tasks)?));
    Ok((tasks, fingerprint))
}

pub(super) fn definitions(root: &Path) -> Result<Value> {
    let (tasks, fingerprint) = load(root)?;
    Ok(json!({"tasks":tasks,"fingerprint":fingerprint}))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Chunk {
    seq: u64,
    stream: &'static str,
    text: String,
}

struct State {
    status: &'static str,
    exit_code: Option<i32>,
    error: Option<String>,
    ended: Option<u64>,
    chunks: VecDeque<Chunk>,
    bytes: usize,
    sequence: u64,
    readers: usize,
}
struct Run {
    id: String,
    workspace: WorkspaceRef,
    task: Task,
    started: u64,
    clock: Instant,
    // A single lock serializes Stop against process exit; output uses a separate lock.
    child: Mutex<Option<Box<dyn ChildWrapper>>>,
    state: Mutex<State>,
}
static RUNS: OnceLock<Mutex<VecDeque<Arc<Run>>>> = OnceLock::new();
fn registry() -> &'static Mutex<VecDeque<Arc<Run>>> {
    RUNS.get_or_init(Default::default)
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn active(state: &State) -> bool {
    matches!(state.status, "running" | "stopping") || state.readers > 0
}

impl Run {
    fn summary(&self) -> Value {
        let s = self.state.lock().unwrap();
        json!({"id":self.id,"workspace":self.workspace,"taskId":self.task.id,"label":self.task.label,
            "group":self.task.group,"command":self.task.command,"args":self.task.args,"cwd":self.task.cwd,
            "started":self.started,"ended":s.ended,"status":s.status,"exitCode":s.exit_code,
            "error":s.error,"outputComplete":s.readers == 0,"sequence":s.sequence})
    }
    fn append(&self, stream: &'static str, text: String) {
        let mut s = self.state.lock().unwrap();
        s.sequence += 1;
        let seq = s.sequence;
        s.bytes += text.len();
        s.chunks.push_back(Chunk { seq, stream, text });
        while s.bytes > OUTPUT_BYTES || s.chunks.len() > OUTPUT_CHUNKS {
            if let Some(chunk) = s.chunks.pop_front() {
                s.bytes -= chunk.text.len();
            }
        }
    }
}

fn find(workspace: &WorkspaceRef, id: &str) -> Result<Arc<Run>> {
    registry()
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.id == id && &r.workspace == workspace)
        .cloned()
        .context("Task run is no longer available in this workspace")
}
pub(super) fn runs(workspace: &WorkspaceRef) -> Result<Value> {
    Ok(json!(registry()
        .lock()
        .unwrap()
        .iter()
        .filter(|r| &r.workspace == workspace)
        .map(|r| r.summary())
        .collect::<Vec<_>>()))
}
pub(super) fn output(workspace: &WorkspaceRef, id: &str, after: u64) -> Result<Value> {
    let run = find(workspace, id)?;
    let s = run.state.lock().unwrap();
    let first = s.chunks.front().map_or(s.sequence + 1, |c| c.seq);
    // Limit each RPC so a noisy task cannot monopolize the SSH service.
    let chunks: Vec<_> = s.chunks.iter().filter(|c| c.seq > after).take(64).collect();
    let next = chunks.last().map_or(after.min(s.sequence), |c| c.seq);
    Ok(
        json!({"chunks":chunks,"next":next,"truncated":after.saturating_add(1) < first,
        "more":next < s.sequence}),
    )
}

// Protect the raw child even if a later process-wrap hook fails (notably Windows
// job assignment). std::process::Child alone would leak the suspended process.
#[derive(Debug)]
struct ReapChild(Option<Box<dyn ChildWrapper>>);
impl Drop for ReapChild {
    fn drop(&mut self) {
        if let Some(c) = self.0.as_mut() {
            if matches!(c.try_wait(), Ok(None)) {
                let _ = c.start_kill();
                let _ = c.wait();
            }
        }
    }
}
impl ChildWrapper for ReapChild {
    fn inner(&self) -> &dyn ChildWrapper {
        self.0.as_ref().unwrap().as_ref()
    }
    fn inner_mut(&mut self) -> &mut dyn ChildWrapper {
        self.0.as_mut().unwrap().as_mut()
    }
    fn into_inner(mut self: Box<Self>) -> Box<dyn ChildWrapper> {
        self.0.take().unwrap()
    }
}
#[derive(Debug)]
struct Reap;
impl CommandWrapper for Reap {
    fn wrap_child(
        &mut self,
        child: Box<dyn ChildWrapper>,
        _: &CommandWrap,
    ) -> std::io::Result<Box<dyn ChildWrapper>> {
        Ok(Box::new(ReapChild(Some(child))))
    }
}
#[cfg(unix)]
#[derive(Debug)]
struct ProcessTree;
#[cfg(unix)]
impl CommandWrapper for ProcessTree {
    fn pre_spawn(&mut self, cmd: &mut Command, _: &CommandWrap) -> std::io::Result<()> {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
        Ok(())
    }
    fn wrap_child(
        &mut self,
        child: Box<dyn ChildWrapper>,
        _: &CommandWrap,
    ) -> std::io::Result<Box<dyn ChildWrapper>> {
        Ok(Box::new(ProcessTreeChild {
            child,
            reaped: false,
        }))
    }
}
#[cfg(unix)]
#[derive(Debug)]
struct ProcessTreeChild {
    child: Box<dyn ChildWrapper>,
    reaped: bool,
}
#[cfg(unix)]
impl ChildWrapper for ProcessTreeChild {
    fn inner(&self) -> &dyn ChildWrapper {
        self.child.as_ref()
    }
    fn inner_mut(&mut self) -> &mut dyn ChildWrapper {
        self.child.as_mut()
    }
    fn into_inner(self: Box<Self>) -> Box<dyn ChildWrapper> {
        self.child
    }
    fn start_kill(&mut self) -> std::io::Result<()> {
        if self.reaped {
            return Ok(());
        }
        // The unreaped child reserves the group ID; never signal a recycled PID.
        let result = unsafe { libc::kill(-(self.child.id() as i32), libc::SIGKILL) };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    }
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        if !self.reaped {
            // Observe exit without reaping, terminate leftover group members,
            // then let std::Child reap and cache the parent's actual status.
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.child.id(),
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if result != 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    return Ok(None);
                }
                // An external reaper means we no longer own the numeric PID.
                if error.raw_os_error() == Some(libc::ECHILD) {
                    self.reaped = true;
                }
                return Err(error);
            }
            if unsafe { info.si_pid() } == 0 {
                return Ok(None);
            }
            self.start_kill()?;
        }
        let status = self.child.try_wait()?;
        self.reaped |= status.is_some();
        Ok(status)
    }
}
#[cfg(windows)]
#[derive(Debug)]
struct HiddenSuspended;
#[cfg(windows)]
impl CommandWrapper for HiddenSuspended {
    fn pre_spawn(&mut self, cmd: &mut Command, _: &CommandWrap) -> std::io::Result<()> {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};
        cmd.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
        Ok(())
    }
}

pub(super) fn start(
    workspace: &WorkspaceRef,
    root: &Path,
    id: &str,
    fingerprint: &str,
) -> Result<Value> {
    let (tasks, current) = load(root)?;
    anyhow::ensure!(
        current == fingerprint,
        "Tasks changed on disk. Refresh and select the task again."
    );
    let task = tasks
        .into_iter()
        .find(|t| t.id == id)
        .context("Unknown project task")?;
    let cwd = root
        .join(&task.cwd)
        .canonicalize()
        .context("Task working directory is unavailable")?;
    anyhow::ensure!(
        cwd.starts_with(root) && cwd.is_dir(),
        "Task working directory must be inside the workspace"
    );
    let mut registry = registry().lock().unwrap();
    anyhow::ensure!(
        !super::SHUTTING_DOWN.load(std::sync::atomic::Ordering::Acquire),
        "Coding service is shutting down"
    );
    anyhow::ensure!(
        registry
            .iter()
            .filter(|r| active(&r.state.lock().unwrap()))
            .count()
            < MAX_ACTIVE,
        "At most eight tasks can run at once on this host"
    );
    anyhow::ensure!(
        !registry.iter().any(|r| &r.workspace == workspace
            && r.task.id == id
            && active(&r.state.lock().unwrap())),
        "This task is already running in this workspace"
    );
    while registry.len() >= MAX_RUNS {
        let i = registry
            .iter()
            .position(|r| !active(&r.state.lock().unwrap()))
            .context("Task history is full")?;
        registry.remove(i);
    }
    let executable =
        if task.command.contains(['/', '\\']) && !Path::new(&task.command).is_absolute() {
            cwd.join(&task.command).into_os_string()
        } else {
            task.command.clone().into()
        };
    let mut command = Command::new(executable);
    command
        .args(&task.args)
        .current_dir(cwd)
        .envs(&task.env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut command = CommandWrap::from(command);
    command.wrap(Reap);
    #[cfg(unix)]
    command.wrap(ProcessTree);
    #[cfg(windows)]
    command
        .wrap(process_wrap::std::JobObject)
        .wrap(HiddenSuspended);
    let mut child = command
        .spawn()
        .with_context(|| format!("Cannot start {}", task.command))?;
    let stdout = child
        .stdout()
        .take()
        .context("Task stdout is unavailable")?;
    let stderr = child
        .stderr()
        .take()
        .context("Task stderr is unavailable")?;
    let run = Arc::new(Run {
        id: uuid::Uuid::new_v4().to_string(),
        workspace: workspace.clone(),
        task,
        started: now(),
        clock: Instant::now(),
        child: Mutex::new(Some(child)),
        state: Mutex::new(State {
            status: "running",
            exit_code: None,
            error: None,
            ended: None,
            chunks: VecDeque::new(),
            bytes: 0,
            sequence: 0,
            readers: 0,
        }),
    });
    // Start the supervisor before publishing. Thread creation failure kills and
    // reaps the child through the guard instead of losing a live process.
    let watched = Arc::clone(&run);
    if let Err(error) = std::thread::Builder::new()
        .name("coding-task".into())
        .spawn(move || supervise(watched))
    {
        if let Some(mut c) = run.child.lock().unwrap().take() {
            let _ = c.kill();
        }
        return Err(error.into());
    }
    for (stream, pipe) in [
        ("stdout", Box::new(stdout) as Box<dyn Read + Send>),
        ("stderr", Box::new(stderr) as Box<dyn Read + Send>),
    ] {
        run.state.lock().unwrap().readers += 1;
        let reader_run = Arc::clone(&run);
        if let Err(error) = std::thread::Builder::new()
            .name(format!("task-{stream}"))
            .spawn(move || read_output(reader_run, stream, pipe))
        {
            run.state.lock().unwrap().readers -= 1;
            let _ = stop_run(&run, Some(format!("Cannot capture task output: {error}")));
        }
    }
    registry.push_back(Arc::clone(&run));
    Ok(run.summary())
}

fn read_output(run: Arc<Run>, stream: &'static str, mut pipe: Box<dyn Read + Send>) {
    let mut bytes = [0; 8192];
    let mut pending = Vec::new();
    loop {
        match pipe.read(&mut bytes) {
            Ok(0) => break,
            Ok(n) => {
                pending.extend_from_slice(&bytes[..n]);
                let text = decode_output(&mut pending);
                if !text.is_empty() {
                    run.append(stream, text);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                run.append("system", format!("\nOutput read failed: {e}\n"));
                break;
            }
        }
    }
    if !pending.is_empty() {
        run.append(stream, String::from_utf8_lossy(&pending).into_owned());
    }
    run.state.lock().unwrap().readers -= 1;
}

/// Lossy for invalid bytes, but never split valid UTF-8 across pipe reads.
fn decode_output(pending: &mut Vec<u8>) -> String {
    let mut text = String::new();
    let mut consumed = 0;
    loop {
        match std::str::from_utf8(&pending[consumed..]) {
            Ok(valid) => {
                text.push_str(valid);
                consumed = pending.len();
                break;
            }
            Err(error) => {
                let end = consumed + error.valid_up_to();
                text.push_str(std::str::from_utf8(&pending[consumed..end]).unwrap());
                consumed = end;
                match error.error_len() {
                    Some(n) => {
                        text.push('\u{fffd}');
                        consumed += n;
                    }
                    None => break,
                }
            }
        }
    }
    pending.drain(..consumed);
    text
}

fn stop_run(run: &Run, reason: Option<String>) -> Result<()> {
    let mut child = run.child.lock().unwrap();
    if let Some(child) = child.as_mut() {
        // Force-stop is deliberate: bounded cancellation for build/watch tools.
        child
            .start_kill()
            .context("Cannot stop task process group")?;
        let mut s = run.state.lock().unwrap();
        s.status = "stopping";
        if reason.is_some() {
            s.error = reason;
        }
    }
    Ok(())
}
pub(super) fn stop(workspace: &WorkspaceRef, id: &str) -> Result<Value> {
    let run = find(workspace, id)?;
    stop_run(&run, None)?;
    Ok(run.summary())
}
fn supervise(run: Arc<Run>) {
    loop {
        if run
            .task
            .timeout_ms
            .is_some_and(|ms| run.clock.elapsed() >= Duration::from_millis(ms))
            && run.state.lock().unwrap().status == "running"
        {
            let _ = stop_run(&run, Some("Task exceeded timeoutMs".into()));
        }
        {
            let mut slot = run.child.lock().unwrap();
            let Some(child) = slot.as_mut() else { return };
            match child.try_wait() {
                Ok(None) => {}
                result => {
                    // A task owns its descendants; a parent exiting must not
                    // leave background watchers behind or stdout pipes open.
                    let _ = child.start_kill();
                    let mut s = run.state.lock().unwrap();
                    match result {
                        Ok(Some(exit)) => {
                            s.exit_code = exit.code();
                            s.status = if s.error.is_some() {
                                "failed"
                            } else if s.status == "stopping" {
                                "stopped"
                            } else if exit.success() {
                                "succeeded"
                            } else {
                                "failed"
                            };
                        }
                        Err(error) => {
                            s.status = "failed";
                            s.error = Some(error.to_string());
                        }
                        _ => unreachable!(),
                    }
                    s.ended = Some(now());
                    drop(s);
                    slot.take();
                    return;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
pub(super) fn shutdown() {
    let runs: Vec<_> = registry().lock().unwrap().iter().cloned().collect();
    for run in runs {
        let _ = stop_run(&run, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_preserves_split_unicode_after_invalid_bytes() {
        let mut pending = vec![0xff, 0xf0, 0x9f];
        assert_eq!(decode_output(&mut pending), "\u{fffd}");
        assert_eq!(pending, vec![0xf0, 0x9f]);
        pending.extend_from_slice(&[0x98, 0x80, b'!']);
        assert_eq!(decode_output(&mut pending), "😀!");
        assert!(pending.is_empty());
    }

    struct Project(std::path::PathBuf);
    impl Project {
        fn new(tasks: Value) -> Self {
            let path = std::env::temp_dir().join(format!("koma-tasks-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(path.join(".koma")).unwrap();
            let project = Self(path.canonicalize().unwrap());
            project.write(tasks);
            project
        }
        fn write(&self, tasks: Value) {
            std::fs::write(
                self.0.join(".koma/coding.json"),
                json!({"version":1,"tasks":tasks}).to_string(),
            )
            .unwrap();
        }
        fn workspace(&self) -> WorkspaceRef {
            WorkspaceRef {
                host_id: "local".into(),
                root: self.0.to_string_lossy().into_owned(),
            }
        }
    }
    impl Drop for Project {
        fn drop(&mut self) {
            let workspace = self.workspace();
            let runs: Vec<_> = registry()
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.workspace == workspace)
                .cloned()
                .collect();
            for run in runs {
                let _ = stop_run(&run, None);
            }
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn task_configuration_rejects_ambiguous_or_stale_execution() {
        let task = json!({"id":"build","label":"Build","command":"unused","args":["original"]});
        let project = Project::new(json!([task]));
        let (_, fingerprint) = load(&project.0).unwrap();
        project
            .write(json!([{"id":"build","label":"Build","command":"unused","args":["changed"]}]));
        let error = start(&project.workspace(), &project.0, "build", &fingerprint).unwrap_err();
        assert!(error.to_string().contains("changed on disk"));
        project.write(json!([task, task]));
        assert!(load(&project.0).is_err());
        project.write(json!([{"id":"build","label":"Build","command":"unused","shell":true}]));
        assert!(load(&project.0).is_err());
    }

    #[cfg(unix)]
    fn await_done(run: &Run) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while active(&run.state.lock().unwrap()) {
            assert!(
                Instant::now() < deadline,
                "Task did not finish and close its pipes"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    #[test]
    fn task_identity_cancellation_and_literal_arguments() {
        let project = Project::new(json!([
            {"id":"watch","label":"Watch","command":"/bin/sh","args":["-c","sleep 30 & wait"]},
            {"id":"echo","label":"Echo","command":"/bin/sh","args":["-c","printf '%s' \"$1\"; printf 'problem' >&2; exit 7","task","literal $HOME; 😀"]}
        ]));
        let workspace = project.workspace();
        let (_, fingerprint) = load(&project.0).unwrap();
        let started = start(&workspace, &project.0, "watch", &fingerprint).unwrap();
        let id = started["id"].as_str().unwrap();
        assert!(start(&workspace, &project.0, "watch", &fingerprint).is_err());
        let other = WorkspaceRef {
            host_id: "other".into(),
            ..workspace.clone()
        };
        assert!(stop(&other, id).is_err());
        assert!(output(&other, id, 0).is_err());
        stop(&workspace, id).unwrap();
        let run = find(&workspace, id).unwrap();
        await_done(&run);
        assert_eq!(run.summary()["status"], "stopped");
        // Repeated Stop is harmless and preserves terminal status.
        assert_eq!(stop(&workspace, id).unwrap()["status"], "stopped");
        let started = start(&workspace, &project.0, "echo", &fingerprint).unwrap();
        let run = find(&workspace, started["id"].as_str().unwrap()).unwrap();
        await_done(&run);
        assert_eq!(run.summary()["exitCode"], 7);
        let state = run.state.lock().unwrap();
        assert_eq!(
            state
                .chunks
                .iter()
                .filter(|c| c.stream == "stdout")
                .map(|c| c.text.as_str())
                .collect::<String>(),
            "literal $HOME; 😀"
        );
        assert_eq!(
            state
                .chunks
                .iter()
                .filter(|c| c.stream == "stderr")
                .map(|c| c.text.as_str())
                .collect::<String>(),
            "problem"
        );
    }

    #[cfg(unix)]
    #[test]
    fn task_deadline_and_parent_exit_close_descendant_pipes() {
        let project = Project::new(json!([
            {"id":"timeout","label":"Timeout","command":"/bin/sh","args":["-c","sleep 30 & wait"],"timeoutMs":100},
            {"id":"parent","label":"Parent exits","command":"/bin/sh","args":["-c","sleep 30 & exit 0"]}
        ]));
        let workspace = project.workspace();
        let (_, fingerprint) = load(&project.0).unwrap();
        for id in ["timeout", "parent"] {
            let started = start(&workspace, &project.0, id, &fingerprint).unwrap();
            let run = find(&workspace, started["id"].as_str().unwrap()).unwrap();
            await_done(&run);
            if id == "timeout" {
                assert_eq!(run.summary()["status"], "failed");
                assert_eq!(run.summary()["error"], "Task exceeded timeoutMs");
            } else {
                assert_eq!(run.summary()["status"], "succeeded");
            }
            for _ in 0..OUTPUT_CHUNKS + 10 {
                run.append("stdout", "a".into());
            }
            let page = output(&workspace, &run.id, 0).unwrap();
            assert_eq!(page["truncated"], true);
            assert!(page["chunks"].as_array().unwrap().len() <= 64);
        }
    }
}
