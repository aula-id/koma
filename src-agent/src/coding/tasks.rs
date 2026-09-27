//! Explicit project tasks, retained independently of chat and UI selection.
use super::sync::CheckedMutex;
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
    env_remove: Vec<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    continue_on_error: bool,
    #[serde(default)]
    interactive: bool,
    #[serde(default)]
    problem_matchers: Vec<super::problems::Matcher>,
}
fn default_group() -> String {
    "run".into()
}
fn default_cwd() -> String {
    ".".into()
}

fn load(root: &Path) -> Result<(Vec<Task>, String)> {
    let config = super::workspace::read_config(root)?;
    let mut tasks: Vec<Task> =
        serde_json::from_value(config.get("tasks").cloned().unwrap_or(json!([])))
            .context("Invalid tasks in .koma/coding.json")?;
    anyhow::ensure!(
        tasks.len() <= 100,
        "At most 100 project tasks are supported"
    );
    discover(root, &mut tasks)?;
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
            task.env_remove.len() <= 100
                && task
                    .env_remove
                    .iter()
                    .all(|key| !key.is_empty() && !key.contains(['=', '\0'])),
            "Invalid removed environment keys"
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
enum TaskChild {
    Pipe(Box<dyn ChildWrapper>),
    Pty(super::pty::Process),
}
impl TaskChild {
    fn start_kill(&mut self) -> std::io::Result<()> {
        match self {
            Self::Pipe(child) => child.start_kill(),
            Self::Pty(child) => child.kill(),
        }
    }
    fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        match self {
            Self::Pipe(child) => child
                .try_wait()
                .map(|status| status.map(|s| s.code().unwrap_or(-1))),
            Self::Pty(child) => child.try_wait(),
        }
    }
}
pub(super) type OutputObserver = Arc<dyn Fn(&str, &str) -> Result<()> + Send + Sync>;
struct Run {
    observer: Option<OutputObserver>,
    problems: Mutex<super::problems::Collector>,
    id: String,
    workspace: WorkspaceRef,
    task: Task,
    reserved: Vec<String>,
    started: u64,
    // A single lock serializes Stop against process exit; output uses a separate lock.
    child: Mutex<Option<TaskChild>>,
    canceled: std::sync::atomic::AtomicBool,
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
    matches!(state.status, "queued" | "running" | "stopping") || state.readers > 0
}

impl Run {
    fn summary(&self) -> Result<Value> {
        let s = self.state.checked_lock()?;
        Ok(
            json!({"id":self.id,"workspace":self.workspace,"taskId":self.task.id,"label":self.task.label,
            "group":self.task.group,"interactive":self.task.interactive,"command":self.task.command,"args":self.task.args,"cwd":self.task.cwd,
            "started":self.started,"ended":s.ended,"status":s.status,"exitCode":s.exit_code,
            "error":s.error,"outputComplete":s.readers == 0,"sequence":s.sequence}),
        )
    }
    fn append(&self, stream: &'static str, text: String) -> Result<()> {
        if let Some(observer) = &self.observer {
            observer(stream, &text)?;
        }
        self.problems.checked_lock()?.ingest(stream, &text);
        let mut s = self.state.checked_lock()?;
        s.sequence += 1;
        let seq = s.sequence;
        s.bytes += text.len();
        s.chunks.push_back(Chunk { seq, stream, text });
        while s.bytes > OUTPUT_BYTES || s.chunks.len() > OUTPUT_CHUNKS {
            if let Some(chunk) = s.chunks.pop_front() {
                s.bytes -= chunk.text.len();
            }
        }
        Ok(())
    }
}

fn find(workspace: &WorkspaceRef, id: &str) -> Result<Arc<Run>> {
    registry()
        .checked_lock()?
        .iter()
        .find(|r| r.id == id && &r.workspace == workspace)
        .cloned()
        .context("Task run is no longer available in this workspace")
}
pub(super) fn runs(workspace: &WorkspaceRef) -> Result<Value> {
    Ok(json!(registry()
        .checked_lock()?
        .iter()
        .filter(|r| &r.workspace == workspace)
        .map(|r| r.summary())
        .collect::<Result<Vec<_>>>()?))
}
pub(super) fn output(workspace: &WorkspaceRef, id: &str, after: u64) -> Result<Value> {
    let run = find(workspace, id)?;
    let s = run.state.checked_lock()?;
    if s.readers == 0 && !active(&s) {
        run.problems.checked_lock()?.finish();
    }
    let first = s.chunks.front().map_or(s.sequence + 1, |c| c.seq);
    // Limit each RPC so a noisy task cannot monopolize the SSH service.
    let chunks: Vec<_> = s.chunks.iter().filter(|c| c.seq > after).take(64).collect();
    let next = chunks.last().map_or(after.min(s.sequence), |c| c.seq);
    Ok(
        json!({"chunks":chunks,"next":next,"truncated":after.saturating_add(1) < first,
        "more":next < s.sequence,"problems":run.problems.checked_lock()?.values}),
    )
}

// Protect the raw child even if a later process-wrap hook fails (notably Windows
// job assignment). std::process::Child alone would leak the suspended process.
type GuardedChild = scopeguard::ScopeGuard<Box<dyn ChildWrapper>, fn(Box<dyn ChildWrapper>)>;
#[derive(Debug)]
struct ReapChild(GuardedChild);
fn reap_child(mut child: Box<dyn ChildWrapper>) {
    if !matches!(child.try_wait(), Ok(Some(_))) {
        let _ = child.start_kill();
        let _ = child.wait();
    }
}
impl ChildWrapper for ReapChild {
    fn inner(&self) -> &dyn ChildWrapper {
        self.0.as_ref()
    }
    fn inner_mut(&mut self) -> &mut dyn ChildWrapper {
        self.0.as_mut()
    }
    fn into_inner(self: Box<Self>) -> Box<dyn ChildWrapper> {
        scopeguard::ScopeGuard::into_inner(self.0)
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
        Ok(Box::new(ReapChild(scopeguard::guard(
            child,
            reap_child as fn(Box<dyn ChildWrapper>),
        ))))
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
        .iter()
        .find(|t| t.id == id)
        .context("Unknown project task")?
        .clone();
    let mut ordered = Vec::new();
    let mut visiting = std::collections::HashSet::new();
    let mut done = std::collections::HashSet::new();
    let all = tasks;
    fn visit(
        id: &str,
        tasks: &[Task],
        visiting: &mut std::collections::HashSet<String>,
        done: &mut std::collections::HashSet<String>,
        ordered: &mut Vec<Task>,
    ) -> Result<()> {
        if done.contains(id) {
            return Ok(());
        }
        anyhow::ensure!(visiting.insert(id.into()), "Task dependency cycle at {id}");
        let task = tasks
            .iter()
            .find(|t| t.id == id)
            .with_context(|| format!("Unknown task dependency: {id}"))?;
        for dep in &task.depends_on {
            visit(dep, tasks, visiting, done, ordered)?;
        }
        visiting.remove(id);
        done.insert(id.into());
        ordered.push(task.clone());
        Ok(())
    }
    visit(id, &all, &mut visiting, &mut done, &mut ordered)?;
    start_recipe(workspace, root, task, ordered, None)
}

/// Internal callers supply a concrete, already-reviewed sequence of commands.
/// The public task RPC still accepts only IDs plus a definition fingerprint.
pub(super) fn run_commands(
    workspace: &WorkspaceRef,
    root: &Path,
    id: &str,
    label: &str,
    commands: Vec<Value>,
) -> Result<Value> {
    anyhow::ensure!(
        !commands.is_empty() && commands.len() <= 100,
        "Invalid command sequence"
    );
    run_observed(workspace, root, id, label, commands, None)
}
pub(super) fn run_observed(
    workspace: &WorkspaceRef,
    root: &Path,
    id: &str,
    label: &str,
    commands: Vec<Value>,
    observer: Option<OutputObserver>,
) -> Result<Value> {
    anyhow::ensure!(
        !commands.is_empty() && commands.len() <= 100,
        "Invalid command sequence"
    );
    let mut steps = Vec::new();
    for (i, mut value) in commands.into_iter().enumerate() {
        value["id"] = json!(format!("{id}:{i}"));
        value["label"] = json!(label);
        steps.push(serde_json::from_value::<Task>(value)?);
    }
    let mut task = steps[0].clone();
    task.id = id.into();
    task.label = label.into();
    start_recipe(workspace, root, task, steps, observer)
}

fn start_recipe(
    workspace: &WorkspaceRef,
    root: &Path,
    mut task: Task,
    steps: Vec<Task>,
    observer: Option<OutputObserver>,
) -> Result<Value> {
    task.interactive = steps.iter().any(|step| step.interactive);
    for step in &steps {
        super::problems::compile(&step.problem_matchers)?;
        let cwd = root
            .join(&step.cwd)
            .canonicalize()
            .context("Task working directory is unavailable")?;
        anyhow::ensure!(
            cwd.starts_with(root) && cwd.is_dir(),
            "Task working directory must be inside the workspace"
        );
    }
    let mut registry = registry().checked_lock()?;
    anyhow::ensure!(
        !super::SHUTTING_DOWN.load(std::sync::atomic::Ordering::Acquire),
        "Coding service is shutting down"
    );
    let mut active_ids = std::collections::HashSet::new();
    for run in registry.iter() {
        if active(&*run.state.checked_lock()?) {
            active_ids.insert(run.id.clone());
        }
    }
    anyhow::ensure!(
        active_ids.len() < MAX_ACTIVE,
        "At most eight tasks can run at once on this host"
    );
    anyhow::ensure!(
        !registry.iter().any(|r| &r.workspace == workspace
            && (r.task.id == task.id || steps.iter().any(|step| r.reserved.contains(&step.id)))
            && active_ids.contains(&r.id)),
        "This task is already running in this workspace"
    );
    anyhow::ensure!(
        !task.id.starts_with("pack:")
            || !registry
                .iter()
                .any(|r| r.task.id.starts_with("pack:") && active_ids.contains(&r.id)),
        "Another language pack installation is running on this host"
    );
    while registry.len() >= MAX_RUNS {
        let i = registry
            .iter()
            .position(|r| !active_ids.contains(&r.id))
            .context("Task history is full")?;
        registry.remove(i);
    }
    let run = Arc::new(Run {
        observer,
        problems: Mutex::new(Default::default()),
        id: uuid::Uuid::new_v4().to_string(),
        workspace: workspace.clone(),
        task,
        reserved: steps.iter().map(|s| s.id.clone()).collect(),
        started: now(),
        child: Mutex::new(None),
        canceled: std::sync::atomic::AtomicBool::new(false),
        state: Mutex::new(State {
            status: "queued",
            exit_code: None,
            error: None,
            ended: None,
            chunks: VecDeque::new(),
            bytes: 0,
            sequence: 0,
            readers: 0,
        }),
    });
    let summary = run.summary()?;
    let watched = Arc::clone(&run);
    let root = root.to_path_buf();
    std::thread::Builder::new()
        .name("coding-task".into())
        .spawn(move || supervise(watched, root, steps))?;
    registry.push_back(Arc::clone(&run));
    Ok(summary)
}

pub(super) fn spawn_command(command: Command) -> std::io::Result<Box<dyn ChildWrapper>> {
    let mut command = CommandWrap::from(command);
    command.wrap(Reap);
    #[cfg(unix)]
    command.wrap(ProcessTree);
    #[cfg(windows)]
    command
        .wrap(process_wrap::std::JobObject)
        .wrap(HiddenSuspended);
    command.spawn()
}

fn launch_step(run: &Arc<Run>, root: &Path, task: &Task) -> Result<()> {
    let cwd = root.join(&task.cwd).canonicalize()?;
    anyhow::ensure!(
        cwd.starts_with(root),
        "Task working directory moved outside the workspace"
    );
    run.problems
        .checked_lock()?
        .start(&cwd, &task.problem_matchers)?;
    let executable =
        if task.command.contains(['/', '\\']) && !Path::new(&task.command).is_absolute() {
            cwd.join(&task.command).into_os_string()
        } else {
            super::environment::executable(root, &task.command)?.into_os_string()
        };
    if task.interactive {
        let mut env = super::environment::variables(root, &task.command)?;
        env.extend(task.env.clone());
        let mut slot = run.child.checked_lock()?;
        anyhow::ensure!(
            !run.canceled.load(std::sync::atomic::Ordering::Acquire),
            "Task stopped"
        );
        let (child, reader) = super::pty::Process::spawn(
            Path::new(&executable),
            &task.args,
            &cwd,
            &env,
            &task.env_remove,
        )?;
        *slot = Some(TaskChild::Pty(child));
        {
            let mut state = run.state.checked_lock()?;
            state.status = "running";
            state.readers += 1;
        }
        drop(slot);
        let reader_run = run.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("coding-task-pty".into())
            .spawn(move || read_output(reader_run, "terminal", reader))
        {
            run.state.checked_lock()?.readers -= 1;
            let _ = stop_run(run, Some(error.to_string()));
            return Err(error.into());
        }
        return Ok(());
    }
    let mut command = Command::new(executable);
    command
        .args(&task.args)
        .current_dir(cwd)
        .envs(super::environment::variables(root, &task.command)?)
        .envs(&task.env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in &task.env_remove {
        command.env_remove(key);
    }
    let mut slot = run.child.checked_lock()?;
    anyhow::ensure!(
        !run.canceled.load(std::sync::atomic::Ordering::Acquire),
        "Task stopped"
    );
    let mut child =
        spawn_command(command).with_context(|| format!("Cannot start {}", task.command))?;
    let stdout = child
        .stdout()
        .take()
        .context("Task stdout is unavailable")?;
    let stderr = child
        .stderr()
        .take()
        .context("Task stderr is unavailable")?;
    *slot = Some(TaskChild::Pipe(child));
    {
        let mut s = run.state.checked_lock()?;
        s.status = "running";
        s.readers += 2;
    }
    drop(slot);
    for (stream, pipe) in [
        ("stdout", Box::new(stdout) as Box<dyn Read + Send>),
        ("stderr", Box::new(stderr) as Box<dyn Read + Send>),
    ] {
        let capture = Arc::clone(run);
        if let Err(error) = std::thread::Builder::new()
            .name(format!("task-{stream}"))
            .spawn(move || read_output(capture, stream, pipe))
        {
            run.state.checked_lock()?.readers -= 1;
            stop_run(run, Some(format!("Cannot capture task output: {error}")))?;
        }
    }
    Ok(())
}

fn read_output(run: Arc<Run>, stream: &'static str, mut pipe: Box<dyn Read + Send>) {
    let result = (|| -> Result<()> {
        let mut bytes = [0; 8192];
        let mut pending = Vec::new();
        loop {
            match pipe.read(&mut bytes) {
                Ok(0) => break,
                Ok(n) => {
                    pending.extend_from_slice(&bytes[..n]);
                    let text = decode_output(&mut pending);
                    if !text.is_empty() {
                        run.append(stream, text)?;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                #[cfg(unix)]
                Err(e) if stream == "terminal" && e.raw_os_error() == Some(libc::EIO) => break,
                Err(e) => {
                    run.append("system", format!("\nOutput read failed: {e}\n"))?;
                    break;
                }
            }
        }
        if !pending.is_empty() {
            run.append(stream, String::from_utf8_lossy(&pending).into_owned())?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        fail_run(&run, &error);
    }
    let mut state = run.state.cleanup_lock();
    state.readers = state.readers.saturating_sub(1);
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
                text.push_str(&String::from_utf8_lossy(&pending[consumed..end]));
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
    let mut child = run.child.checked_lock()?;
    let mut s = run.state.checked_lock()?;
    if !matches!(s.status, "queued" | "running" | "stopping") {
        return Ok(());
    }
    run.canceled
        .store(true, std::sync::atomic::Ordering::Release);
    if let Some(child) = child.as_mut() {
        child
            .start_kill()
            .context("Cannot stop task process group")?;
    }
    s.status = "stopping";
    if reason.is_some() {
        s.error = reason;
    }
    Ok(())
}
pub(super) fn stop(workspace: &WorkspaceRef, id: &str) -> Result<Value> {
    let run = find(workspace, id)?;
    if let Err(error) = stop_run(&run, None) {
        fail_run(&run, &error);
        return Err(error);
    }
    run.summary()
}
fn supervise(run: Arc<Run>, root: std::path::PathBuf, steps: Vec<Task>) {
    if let Err(error) = supervise_steps(&run, &root, &steps) {
        fail_run(&run, &error);
    }
}

// Recovery is restricted to stopping the failed run; poisoned state is never
// used to launch another command or to report a successful result.
fn fail_run(run: &Run, error: &anyhow::Error) {
    run.canceled
        .store(true, std::sync::atomic::Ordering::Release);
    if let Some(mut child) = run.child.cleanup_lock().take() {
        if let Err(error) = child.start_kill() {
            eprintln!("Cannot terminate failed task: {error}");
        }
    }
    let mut state = run.state.cleanup_lock();
    state.status = "failed";
    state.error = Some(format!("{error:#}"));
    state.ended = Some(now());
}

fn supervise_steps(run: &Arc<Run>, root: &Path, steps: &[Task]) -> Result<()> {
    let mut outcome = Ok(0);
    let mut previous_failure = None;
    for step in steps {
        run.problems.checked_lock()?.finish();
        if run.canceled.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        if steps.len() > 1 {
            run.append("system", format!("\n▶ {}\n", step.label))?;
        }
        if let Err(error) = launch_step(run, root, step) {
            if !run.canceled.load(std::sync::atomic::Ordering::Acquire) {
                outcome = Err(format!("{error:#}"));
            }
            break;
        }
        let clock = Instant::now();
        loop {
            if step
                .timeout_ms
                .is_some_and(|ms| clock.elapsed() >= Duration::from_millis(ms))
                && !run.canceled.load(std::sync::atomic::Ordering::Acquire)
            {
                let _ = stop_run(run, Some("Task exceeded timeoutMs".into()));
            }
            let exited = {
                let mut slot = run.child.checked_lock()?;
                let child = slot
                    .as_mut()
                    .context("Task process disappeared while running")?;
                match child.try_wait() {
                    Ok(None) => false,
                    Ok(Some(exit)) => {
                        let _ = child.start_kill();
                        outcome = Ok(exit);
                        slot.take();
                        true
                    }
                    Err(error) => {
                        let _ = child.start_kill();
                        outcome = Err(error.to_string());
                        slot.take();
                        true
                    }
                }
            };
            if exited {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // Drain each step before launching the next so streaming observers see
        // one complete command at a time, including its trailing output.
        while run.state.checked_lock()?.readers > 0 {
            std::thread::sleep(Duration::from_millis(10));
        }
        if !matches!(outcome, Ok(0)) {
            if !step.continue_on_error {
                break;
            }
            previous_failure = Some(outcome.clone());
        }
    }
    if matches!(outcome, Ok(0)) {
        if let Some(failure) = previous_failure {
            outcome = failure;
        }
    }
    let mut s = run.state.checked_lock()?;
    s.exit_code = outcome.as_ref().ok().copied();
    if s.error.is_none() {
        s.error = outcome.as_ref().err().cloned();
    }
    s.status = if s.error.is_some() {
        "failed"
    } else if run.canceled.load(std::sync::atomic::Ordering::Acquire) {
        "stopped"
    } else if matches!(outcome, Ok(0)) {
        "succeeded"
    } else {
        "failed"
    };
    s.ended = Some(now());
    Ok(())
}
pub(super) fn shutdown() {
    let runs: Vec<_> = registry().cleanup_lock().iter().cloned().collect();
    for run in runs {
        if let Err(error) = stop_run(&run, None) {
            fail_run(&run, &error);
        }
    }
}

fn discover(root: &Path, tasks: &mut Vec<Task>) -> Result<()> {
    let mut add =
        |id: &str, label: &str, group: &str, command: &str, args: Vec<String>| -> Result<()> {
            if tasks.len() >= 100 || tasks.iter().any(|t| t.id == id) {
                return Ok(());
            }
            tasks.push(serde_json::from_value(
                json!({"id":id,"label":label,"group":group,"command":command,"args":args}),
            )?);
            Ok(())
        };
    let package = root.join("package.json");
    if package.is_file()
        && package.metadata()?.len() <= 1024 * 1024
        && package.canonicalize()?.starts_with(root)
    {
        if let Ok(value) = serde_json::from_slice::<Value>(&std::fs::read(package)?) {
            if let Some(scripts) = value.get("scripts").and_then(Value::as_object) {
                for (name, script) in scripts.iter().take(100) {
                    if !script.is_string() || name.len() > 80 {
                        continue;
                    }
                    let group = if name.starts_with("test") {
                        "test"
                    } else if name.starts_with("build") {
                        "build"
                    } else {
                        "run"
                    };
                    add(
                        &format!("npm:{name}"),
                        &format!("npm: {name}"),
                        group,
                        if cfg!(windows) { "npm.cmd" } else { "npm" },
                        vec!["run".into(), name.clone()],
                    )?;
                }
            }
        }
    }
    if root.join("Cargo.toml").is_file() {
        for group in ["build", "test", "run"] {
            add(
                &format!("cargo:{group}"),
                &format!("cargo {group}"),
                group,
                "cargo",
                vec![group.into()],
            )?;
        }
    }
    if root.join("go.mod").is_file() {
        for group in ["build", "test"] {
            add(
                &format!("go:{group}"),
                &format!("go {group}"),
                group,
                "go",
                vec![group.into(), "./...".into()],
            )?;
        }
    }
    if root.join("pytest.ini").is_file() || root.join("pyproject.toml").is_file() {
        let python = super::environment::test_python(root)?
            .to_string_lossy()
            .into_owned();
        add(
            "python:test",
            "Python: pytest",
            "test",
            &python,
            vec!["-m".into(), "pytest".into()],
        )?;
    }
    if root.join("Makefile").is_file() {
        add("make:build", "Make", "build", "make", vec![])?;
    }
    Ok(())
}

pub(super) fn summary(workspace: &WorkspaceRef, id: &str) -> Result<Value> {
    find(workspace, id)?.summary()
}

// On poison, keep the service alive rather than incorrectly declaring it idle.
pub(super) fn has_active() -> bool {
    registry().lock().map_or(true, |runs| {
        runs.iter()
            .any(|run| run.state.lock().map_or(true, |state| active(&state)))
    })
}

pub(super) fn input(workspace: &WorkspaceRef, id: &str, data: &str) -> Result<Value> {
    anyhow::ensure!(data.len() <= 65536, "Terminal input exceeds 64 KiB");
    let run = find(workspace, id)?;
    let mut slot = run.child.checked_lock()?;
    match slot.as_mut() {
        Some(TaskChild::Pty(child)) => child.input(data)?,
        _ => anyhow::bail!("This task has no active terminal"),
    };
    Ok(json!({}))
}
pub(super) fn resize(workspace: &WorkspaceRef, id: &str, rows: u16, cols: u16) -> Result<Value> {
    let run = find(workspace, id)?;
    let slot = run.child.checked_lock()?;
    match slot.as_ref() {
        Some(TaskChild::Pty(child)) => child.resize(rows, cols)?,
        _ => anyhow::bail!("This task has no active terminal"),
    };
    Ok(json!({}))
}

pub(super) fn process_id(workspace: &WorkspaceRef, id: &str) -> Result<Option<u32>> {
    let run = find(workspace, id)?;
    let slot = run.child.checked_lock()?;
    Ok(match slot.as_ref() {
        Some(TaskChild::Pipe(child)) => Some(child.id()),
        Some(TaskChild::Pty(child)) => child.process_id(),
        None => None,
    })
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
        assert_eq!(run.summary().unwrap()["status"], "stopped");
        // Repeated Stop is harmless and preserves terminal status.
        assert_eq!(stop(&workspace, id).unwrap()["status"], "stopped");
        let started = start(&workspace, &project.0, "echo", &fingerprint).unwrap();
        let run = find(&workspace, started["id"].as_str().unwrap()).unwrap();
        await_done(&run);
        assert_eq!(run.summary().unwrap()["exitCode"], 7);
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
                assert_eq!(run.summary().unwrap()["status"], "failed");
                assert_eq!(run.summary().unwrap()["error"], "Task exceeded timeoutMs");
            } else {
                assert_eq!(run.summary().unwrap()["status"], "succeeded");
            }
            for _ in 0..OUTPUT_CHUNKS + 10 {
                run.append("stdout", "a".into()).expect("append task output");
            }
            let page = output(&workspace, &run.id, 0).unwrap();
            assert_eq!(page["truncated"], true);
            assert!(page["chunks"].as_array().unwrap().len() <= 64);
        }
    }
}
