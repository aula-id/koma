//! Bounded DAP sessions owned by a workspace, independent of panels and chats.
use super::WorkspaceRef;
use anyhow::{Context, Result};
use process_wrap::std::ChildWrapper;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
const FRAME: usize = 8 * 1024 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Profile {
    id: String,
    label: String,
    adapter: String,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    tcp: bool,
    #[serde(default = "launch")]
    request: String,
    #[serde(default)]
    configuration: Value,
    #[serde(default)]
    pre_launch_task: Option<String>,
    #[serde(default)]
    exception_filters: Option<Vec<String>>,
    #[serde(skip)]
    prepare_commands: Vec<Value>,
    #[serde(skip)]
    prepared_program: Option<PathBuf>,
}
fn launch() -> String {
    "launch".into()
}
fn profiles(root: &Path) -> Result<(Vec<Profile>, String)> {
    let config = super::workspace::read_config(root)?;
    let mut profiles: Vec<Profile> =
        serde_json::from_value(config.get("debug").cloned().unwrap_or(json!([])))?;
    let mut add = |id: &str, label: &str, adapter: &str, configuration: Value| {
        if !profiles.iter().any(|p| p.id == id) {
            profiles.push(Profile {
                id: id.into(),
                label: label.into(),
                adapter: adapter.into(),
                command: None,
                args: vec![],
                tcp: false,
                request: launch(),
                configuration,
                pre_launch_task: None,
                exception_filters: None,
                prepare_commands: Vec::new(),
                prepared_program: None,
            });
        }
    };
    add(
        "python:file",
        "Python: Current file",
        "debugpy",
        json!({"program":"${file}","console":"internalConsole"}),
    );
    add(
        "node:file",
        "Node: Current file",
        "js-debug",
        json!({"type":"pwa-node","program":"${file}","console":"internalConsole"}),
    );
    if root.join("go.mod").is_file() {
        add(
            "go:package",
            "Go: Workspace package",
            "delve",
            json!({"mode":"debug","program":"${workspaceFolder}"}),
        );
    }
    anyhow::ensure!(profiles.len() <= 100, "At most 100 debug profiles");
    let mut ids = std::collections::HashSet::new();
    for p in &profiles {
        anyhow::ensure!(
            !p.id.is_empty()
                && ids.insert(&p.id)
                && matches!(p.request.as_str(), "launch" | "attach")
                && (p.configuration.is_null() || p.configuration.is_object()),
            "Invalid debug profile"
        );
    }
    let hash = super::environment::fingerprint(&json!(profiles))?;
    Ok((profiles, hash))
}
pub(super) fn definitions(root: &Path) -> Result<Value> {
    let (profiles, fingerprint) = profiles(root)?;
    Ok(json!({"profiles":profiles,"fingerprint":fingerprint}))
}
struct State {
    status: String,
    error: Option<String>,
    initialized: bool,
    capabilities: Value,
    events: VecDeque<Value>,
    sequence: u64,
    generation: u64,
    breakpoints: BTreeMap<String, Value>,
    exception_filters: Vec<String>,
}
struct Session {
    id: String,
    workspace: WorkspaceRef,
    profile: Profile,
    writer: Mutex<Option<mpsc::SyncSender<Vec<u8>>>>,
    child: Mutex<Option<Box<dyn ChildWrapper>>>,
    pending: Mutex<BTreeMap<u64, mpsc::SyncSender<Result<Value, String>>>>,
    seq: AtomicU64,
    reverse_requests: AtomicU64,
    closed: AtomicBool,
    state: Mutex<State>,
    terminals: Mutex<Vec<String>>,
    preparation: Mutex<Option<String>>,
}
static SESSIONS: OnceLock<Mutex<VecDeque<Arc<Session>>>> = OnceLock::new();
fn registry() -> &'static Mutex<VecDeque<Arc<Session>>> {
    SESSIONS.get_or_init(Default::default)
}
fn get(workspace: &WorkspaceRef, id: &str) -> Result<Arc<Session>> {
    registry()
        .lock()
        .unwrap()
        .iter()
        .find(|s| s.id == id && &s.workspace == workspace)
        .cloned()
        .context("Debug session not found in this workspace")
}
fn snapshot(s: &Session) -> Value {
    let state = s.state.lock().unwrap();
    json!({"id":s.id,"workspace":s.workspace,"label":s.profile.label,"request":s.profile.request,"status":state.status,"error":state.error,"capabilities":state.capabilities,"sequence":state.sequence,"generation":state.generation,"breakpoints":state.breakpoints,"exceptionFilters":state.exception_filters,"terminalTaskIds":*s.terminals.lock().unwrap(),"preLaunchTaskId":*s.preparation.lock().unwrap()})
}
pub(super) fn sessions(workspace: &WorkspaceRef) -> Result<Value> {
    Ok(json!(registry()
        .lock()
        .unwrap()
        .iter()
        .filter(|s| &s.workspace == workspace)
        .map(|s| snapshot(s))
        .collect::<Vec<_>>()))
}
pub(super) fn events(workspace: &WorkspaceRef, id: &str, after: u64) -> Result<Value> {
    let s = get(workspace, id)?;
    let state = s.state.lock().unwrap();
    Ok(
        json!({"session":snapshot_unlocked(&s,&state),"events":state.events.iter().filter(|e|e["seq"].as_u64().unwrap_or(0)>after).collect::<Vec<_>>(),"next":state.sequence,"truncated":state.events.front().is_some_and(|e|e["seq"].as_u64().unwrap_or(0)>after+1)}),
    )
}
fn snapshot_unlocked(s: &Session, state: &State) -> Value {
    json!({"id":s.id,"status":state.status,"error":state.error,"generation":state.generation,"capabilities":state.capabilities,"breakpoints":state.breakpoints,"exceptionFilters":state.exception_filters,"terminalTaskIds":*s.terminals.lock().unwrap(),"preLaunchTaskId":*s.preparation.lock().unwrap()})
}
fn emit(s: &Session, event: &str, mut body: Value) {
    if let Some(output) = body
        .get("output")
        .and_then(Value::as_str)
        .map(str::to_owned)
    {
        if output.len() > 32768 {
            body["output"] = json!(
                output.chars().take(8192).collect::<String>() + "\n[Output event truncated]\n"
            );
        }
    }
    if serde_json::to_vec(&body).map_or(true, |v| v.len() > 65536) {
        body = json!({"message":"Event body exceeded retention limit"});
    }

    let mut state = s.state.lock().unwrap();
    state.sequence += 1;
    let seq = state.sequence;
    state
        .events
        .push_back(json!({"seq":seq,"event":event,"body":body}));
    while state.events.len() > 256 {
        state.events.pop_front();
    }
}
fn send(s: &Session, value: &Value) -> Result<()> {
    let data = serde_json::to_vec(value)?;
    anyhow::ensure!(data.len() <= FRAME, "DAP message exceeds limit");
    let mut frame = format!("Content-Length: {}\r\n\r\n", data.len()).into_bytes();
    frame.extend(data);
    s.writer
        .lock()
        .unwrap()
        .as_ref()
        .context("Debug adapter is not connected")?
        .try_send(frame)
        .map_err(|_| anyhow::anyhow!("Debug adapter write queue is full or disconnected"))?;
    Ok(())
}
fn connect_writer(s: &Arc<Session>, mut writer: Box<dyn Write + Send>) -> Result<()> {
    let (sender, receiver) = mpsc::sync_channel::<Vec<u8>>(8);
    *s.writer.lock().unwrap() = Some(sender);
    let owner = s.clone();
    std::thread::Builder::new()
        .name("coding-debug-writer".into())
        .spawn(move || {
            while let Ok(frame) = receiver.recv() {
                if let Err(error) = writer.write_all(&frame).and_then(|_| writer.flush()) {
                    if !owner.closed.load(Ordering::Acquire) {
                        end(&owner, Some(error.to_string()));
                        kill(&owner);
                    }
                    break;
                }
            }
        })?;
    Ok(())
}

fn begin(
    s: &Session,
    command: &str,
    args: Value,
) -> Result<(u64, mpsc::Receiver<Result<Value, String>>)> {
    anyhow::ensure!(!s.closed.load(Ordering::Acquire), "Debug session has ended");
    let seq = s.seq.fetch_add(1, Ordering::Relaxed) + 1;
    let (tx, rx) = mpsc::sync_channel(1);
    {
        let mut pending = s.pending.lock().unwrap();
        anyhow::ensure!(pending.len() < 64, "Too many pending debugger requests");
        pending.insert(seq, tx);
    }
    if let Err(e) = send(
        s,
        &json!({"seq":seq,"type":"request","command":command,"arguments":args}),
    ) {
        s.pending.lock().unwrap().remove(&seq);
        return Err(e);
    }
    Ok((seq, rx))
}
fn finish(
    s: &Session,
    pending: (u64, mpsc::Receiver<Result<Value, String>>),
    timeout: Duration,
) -> Result<Value> {
    let result = pending.1.recv_timeout(timeout);
    s.pending.lock().unwrap().remove(&pending.0);
    result
        .context("Debug adapter request timed out")?
        .map_err(anyhow::Error::msg)
}
fn call(s: &Session, command: &str, args: Value) -> Result<Value> {
    finish(s, begin(s, command, args)?, Duration::from_secs(15))
}
fn end(s: &Session, error: Option<String>) {
    s.closed.store(true, Ordering::Release);
    s.writer.lock().unwrap().take();
    let mut state = s.state.lock().unwrap();
    if let Some(error) = error {
        state.error = Some(error);
        state.status = "failed".into();
    } else if state.status != "failed" {
        state.status = "terminated".into();
    }
    drop(state);
    for (_, tx) in std::mem::take(&mut *s.pending.lock().unwrap()) {
        let _ = tx.send(Err("Debug adapter disconnected".into()));
    }
    if let Some(task) = s.preparation.lock().unwrap().as_ref() {
        let _ = super::tasks::stop(&s.workspace, task);
    }
    if s.profile.request == "launch" {
        for task in s.terminals.lock().unwrap().iter() {
            let _ = super::tasks::stop(&s.workspace, task);
        }
    }
}
fn kill(s: &Session) {
    if let Some(child) = s.child.lock().unwrap().as_mut() {
        let _ = child.kill();
    }
}
fn read_message(reader: &mut impl BufRead) -> Result<Value> {
    let mut length = None;
    let mut header_bytes = 0;
    loop {
        let mut line = String::new();
        let n = reader.take(8193).read_line(&mut line)?;
        anyhow::ensure!(n > 0, "Debug adapter closed its output");
        header_bytes += n;
        anyhow::ensure!(header_bytes <= 8192, "DAP header exceeds limit");
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("Content-Length") {
                anyhow::ensure!(length.is_none(), "Duplicate DAP length");
                length = Some(value.trim().parse::<usize>()?);
            }
        }
    }
    let len = length.context("DAP message has no length")?;
    anyhow::ensure!(len <= FRAME, "DAP message exceeds limit");
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}
fn read_loop(s: Arc<Session>, reader: Box<dyn Read + Send>) {
    let mut reader = BufReader::new(reader);
    loop {
        let msg = match read_message(&mut reader) {
            Ok(v) => v,
            Err(e) => {
                if !s.closed.load(Ordering::Acquire) {
                    end(&s, Some(e.to_string()));
                    kill(&s);
                }
                break;
            }
        };
        match msg["type"].as_str() {
            Some("response") => {
                if let Some(id) = msg["request_seq"].as_u64() {
                    if let Some(tx) = s.pending.lock().unwrap().remove(&id) {
                        let value = if msg["success"] == true {
                            Ok(msg["body"].clone())
                        } else {
                            Err(msg["message"]
                                .as_str()
                                .unwrap_or("Debug request failed")
                                .into())
                        };
                        let _ = tx.send(value);
                    }
                }
            }
            Some("event") => {
                let event = msg["event"].as_str().unwrap_or("");
                let body = msg["body"].clone();
                {
                    let mut state = s.state.lock().unwrap();
                    match event {
                        "initialized" => state.initialized = true,
                        "stopped" => {
                            state.status = "stopped".into();
                            state.generation += 1;
                        }
                        "continued" => {
                            state.status = "running".into();
                            state.generation += 1;
                        }
                        "terminated" => state.status = "terminated".into(),
                        "capabilities" => {
                            if let Some(extra) = body["capabilities"].as_object() {
                                if let Some(caps) = state.capabilities.as_object_mut() {
                                    caps.extend(extra.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                emit(&s, event, body);
                if event == "terminated" {
                    s.closed.store(true, Ordering::Release);
                    kill(&s);
                }
            }
            Some("request") => {
                if msg["command"] != "runInTerminal"
                    || s.reverse_requests
                        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                            if n < 4 {
                                Some(n + 1)
                            } else {
                                None
                            }
                        })
                        .is_err()
                {
                    let seq = s.seq.fetch_add(1, Ordering::Relaxed) + 1;
                    let _ = send(
                        &s,
                        &json!({"seq":seq,"type":"response","request_seq":msg["seq"],"command":msg["command"],"success":false,"message":"Unsupported or busy adapter reverse request"}),
                    );
                    continue;
                }
                let owner = s.clone();
                std::thread::spawn(move || {
                    let result = if msg["command"] == "runInTerminal" {
                        run_terminal(&owner, &msg["arguments"])
                    } else {
                        Err(anyhow::anyhow!("Unsupported adapter reverse request"))
                    };
                    let seq = owner.seq.fetch_add(1, Ordering::Relaxed) + 1;
                    let response = match result {
                        Ok(body) => {
                            json!({"seq":seq,"type":"response","request_seq":msg["seq"],"command":msg["command"],"success":true,"body":body})
                        }
                        Err(error) => {
                            json!({"seq":seq,"type":"response","request_seq":msg["seq"],"command":msg["command"],"success":false,"message":error.to_string()})
                        }
                    };
                    let _ = send(&owner, &response);
                    owner.reverse_requests.fetch_sub(1, Ordering::AcqRel);
                });
            }
            _ => {}
        }
    }
}
fn expand(value: &mut Value, root: &str, file: Option<&str>) -> Result<()> {
    match value {
        Value::String(s) => {
            if s.contains("${file}") {
                *s = s.replace(
                    "${file}",
                    file.context("Open a file in this workspace before launching this profile")?,
                );
            }
            *s = s.replace("${workspaceFolder}", root);
        }
        Value::Array(items) => {
            for v in items {
                expand(v, root, file)?
            }
        }
        Value::Object(items) => {
            for v in items.values_mut() {
                expand(v, root, file)?
            }
        }
        _ => {}
    }
    Ok(())
}
fn adapter(profile: &Profile, root: &Path) -> Result<(Command, bool)> {
    if let Some(command) = &profile.command {
        let mut cmd = Command::new(super::environment::executable(root, command)?);
        cmd.args(&profile.args);
        return Ok((cmd, profile.tcp));
    }
    let binary = |id: &str| {
        super::provision::component_binary(id)
            .or_else(|| crate::lsp::resolve::find_on_path(id))
            .with_context(|| {
                format!("Install {id} in Language Packs or configure an adapter command")
            })
    };
    let (mut cmd, tcp) = match profile.adapter.as_str() {
        "debugpy" => {
            let python = super::provision::component_binary("debugpy").unwrap_or(
                super::environment::executable(
                    root,
                    if cfg!(windows) { "python" } else { "python3" },
                )?,
            );
            let mut cmd = Command::new(python);
            cmd.args(["-m", "debugpy.adapter"]);
            (cmd, false)
        }
        "delve" => {
            let mut cmd = Command::new(
                super::provision::component_binary("delve")
                    .or_else(|| crate::lsp::resolve::find_on_path("dlv"))
                    .context("Install Delve in Language Packs")?,
            );
            cmd.args(["dap", "--listen=127.0.0.1:{port}"]);
            (cmd, true)
        }
        "js-debug" => {
            let mut cmd = Command::new(super::environment::executable(root, "node")?);
            cmd.arg(binary("js-debug")?).args(["{port}", "127.0.0.1"]);
            (cmd, true)
        }
        "php-debug" | "bash-debug" | "lua-debug" => {
            let mut cmd = Command::new(super::environment::executable(root, "node")?);
            cmd.arg(binary(&profile.adapter)?);
            (cmd, false)
        }
        "lldb-dap" => (Command::new(binary("lldb-dap")?), false),
        _ => anyhow::bail!(
            "Unknown adapter; provide command and args in project debug configuration"
        ),
    };
    cmd.args(&profile.args);
    Ok((cmd, tcp))
}
pub(super) fn start(
    workspace: &WorkspaceRef,
    root: &Path,
    profile_id: &str,
    fingerprint: &str,
    file: Option<&str>,
    breakpoints: &Value,
) -> Result<Value> {
    let (profiles, current) = profiles(root)?;
    anyhow::ensure!(
        current == fingerprint,
        "Debug profiles changed; refresh before starting"
    );
    let profile = profiles
        .into_iter()
        .find(|p| p.id == profile_id)
        .context("Unknown debug profile")?;
    start_profile(workspace, root, profile, file, breakpoints)
}
pub(super) fn start_custom(
    workspace: &WorkspaceRef,
    root: &Path,
    profile: Value,
    file: Option<&str>,
    breakpoints: &Value,
) -> Result<Value> {
    start_profile(
        workspace,
        root,
        serde_json::from_value(profile)?,
        file,
        breakpoints,
    )
}
pub(super) fn start_prepared_test(
    workspace: &WorkspaceRef,
    root: &Path,
    profile: Value,
    commands: Vec<Value>,
    program: PathBuf,
    breakpoints: &Value,
) -> Result<Value> {
    let mut profile: Profile = serde_json::from_value(profile)?;
    profile.prepare_commands = commands;
    profile.prepared_program = Some(program);
    start_profile(workspace, root, profile, None, breakpoints)
}
fn start_profile(
    workspace: &WorkspaceRef,
    root: &Path,
    mut profile: Profile,
    file: Option<&str>,
    breakpoints: &Value,
) -> Result<Value> {
    let file = if let Some(file) = file {
        let p = root.join(file).canonicalize()?;
        anyhow::ensure!(
            p.starts_with(root) && p.is_file(),
            "Debug file is outside workspace"
        );
        Some(p.to_string_lossy().into_owned())
    } else {
        None
    };
    if profile.configuration.is_null() {
        profile.configuration = json!({});
    }
    expand(
        &mut profile.configuration,
        &root.to_string_lossy(),
        file.as_deref(),
    )?;
    if profile.configuration.get("cwd").is_none() {
        profile.configuration["cwd"] = json!(root);
    }
    if profile.adapter == "debugpy" && profile.configuration.get("python").is_none() {
        profile.configuration["python"] = json!(super::environment::executable(
            root,
            if cfg!(windows) { "python" } else { "python3" }
        )?);
    }
    if profile.adapter == "js-debug" && profile.configuration.get("runtimeExecutable").is_none() {
        profile.configuration["runtimeExecutable"] =
            json!(super::environment::executable(root, "node")?);
    }
    let points = validate_breakpoints(root, breakpoints)?;
    let preparation = if let Some(task) = &profile.pre_launch_task {
        let definitions = super::tasks::definitions(root)?;
        anyhow::ensure!(
            definitions["tasks"]
                .as_array()
                .is_some_and(|tasks| tasks.iter().any(|v| v["id"] == *task)),
            "Unknown preLaunchTask"
        );
        Some((
            task.clone(),
            definitions["fingerprint"].as_str().unwrap().to_string(),
        ))
    } else {
        None
    };
    let mut registry = registry().lock().unwrap();
    anyhow::ensure!(
        !super::SHUTTING_DOWN.load(Ordering::Acquire),
        "Coding service is shutting down"
    );
    anyhow::ensure!(
        registry
            .iter()
            .filter(|s| !s.closed.load(Ordering::Acquire))
            .count()
            < 8,
        "At most eight debug sessions on this host"
    );
    while registry.len() >= 32 {
        let i = registry
            .iter()
            .position(|s| s.closed.load(Ordering::Acquire))
            .context("Debug history is full")?;
        registry.remove(i);
    }
    let s = Arc::new(Session {
        id: uuid::Uuid::new_v4().to_string(),
        workspace: workspace.clone(),
        profile,
        writer: Mutex::new(None),
        child: Mutex::new(None),
        pending: Mutex::new(BTreeMap::new()),
        seq: AtomicU64::new(0),
        reverse_requests: AtomicU64::new(0),
        closed: AtomicBool::new(false),
        terminals: Mutex::new(Vec::new()),
        preparation: Mutex::new(None),
        state: Mutex::new(State {
            status: "starting".into(),
            error: None,
            initialized: false,
            capabilities: json!({}),
            events: VecDeque::new(),
            sequence: 0,
            generation: 0,
            breakpoints: BTreeMap::new(),
            exception_filters: Vec::new(),
        }),
    });
    let worker = s.clone();
    let root = root.to_path_buf();
    std::thread::Builder::new()
        .name("coding-debug-start".into())
        .spawn(move || {
            if let Err(e) = setup(&worker, &root, points, preparation) {
                if !worker.closed.load(Ordering::Acquire) {
                    end(&worker, Some(format!("{e:#}")));
                    kill(&worker);
                }
            }
        })?;
    let result = snapshot(&s);
    registry.push_back(s);
    Ok(result)
}
fn validate_breakpoints(root: &Path, value: &Value) -> Result<BTreeMap<String, Value>> {
    let mut result = BTreeMap::new();
    let object = value
        .as_object()
        .context("Breakpoints must be a path map")?;
    anyhow::ensure!(object.len() <= 200, "Too many breakpoint files");
    for (path, points) in object {
        let file = root.join(path).canonicalize()?;
        anyhow::ensure!(
            file.starts_with(root) && file.is_file(),
            "Breakpoint source is outside workspace"
        );
        let points = points.as_array().context("Breakpoints must be an array")?;
        anyhow::ensure!(
            points.len() <= 500
                && points.iter().all(|p| p["line"]
                    .as_u64()
                    .is_some_and(|n| n > 0 && n <= u32::MAX as u64)),
            "Invalid breakpoint lines"
        );
        result.insert(file.to_string_lossy().into_owned(), json!(points));
    }
    Ok(result)
}
fn setup(
    s: &Arc<Session>,
    root: &Path,
    points: BTreeMap<String, Value>,
    preparation: Option<(String, String)>,
) -> Result<()> {
    if let Some((task, fingerprint)) = preparation {
        let mut slot = s.preparation.lock().unwrap();
        anyhow::ensure!(
            !s.closed.load(Ordering::Acquire),
            "Debug session was canceled"
        );
        let run = super::tasks::start(&s.workspace, root, &task, &fingerprint)?;
        let id = run["id"].as_str().unwrap().to_string();
        *slot = Some(id.clone());
        drop(slot);
        loop {
            anyhow::ensure!(
                !s.closed.load(Ordering::Acquire),
                "Debug session was canceled"
            );
            let run = super::tasks::summary(&s.workspace, &id)?;
            match run["status"].as_str() {
                Some("succeeded") => break,
                Some("queued" | "running" | "stopping") => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                _ => anyhow::bail!("preLaunchTask did not succeed; inspect its Tasks output"),
            }
        }
    }
    let mut configuration = s.profile.configuration.clone();
    if !s.profile.prepare_commands.is_empty() {
        let mut slot = s.preparation.lock().unwrap();
        anyhow::ensure!(!s.closed.load(Ordering::Acquire), "Debug session canceled");
        let run = super::tasks::run_commands(
            &s.workspace,
            root,
            &format!("debug-build:{}", s.id),
            "Build debug test",
            s.profile.prepare_commands.clone(),
        )?;
        let id = run["id"].as_str().unwrap().to_string();
        *slot = Some(id.clone());
        drop(slot);
        loop {
            anyhow::ensure!(!s.closed.load(Ordering::Acquire), "Debug session canceled");
            let run = super::tasks::summary(&s.workspace, &id)?;
            match run["status"].as_str() {
                Some("succeeded") => break,
                Some("queued" | "running" | "stopping") => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                _ => anyhow::bail!("Test build failed; inspect Tasks output"),
            }
        }
        let path = s
            .profile
            .prepared_program
            .as_ref()
            .context("Missing test artifact record")?;
        let file = std::fs::File::open(path)?;
        let mut bytes = Vec::new();
        file.take(65537).read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() <= 65536, "Test artifact record too large");
        let record: Value = serde_json::from_slice(&bytes)?;
        let program = Path::new(
            record["program"]
                .as_str()
                .context("Missing test executable")?,
        )
        .canonicalize()?;
        anyhow::ensure!(program.is_file(), "Test executable is unavailable");
        configuration["program"] = json!(program);
        configuration["args"] = record["args"].clone();
        let _ = std::fs::remove_file(path);
    }
    let (mut command, tcp) = adapter(&s.profile, root)?;
    let mut port = 0;
    if tcp {
        let reservation = TcpListener::bind("127.0.0.1:0")?;
        port = reservation.local_addr()?.port();
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().replace("{port}", &port.to_string()))
            .collect();
        let exe = command.get_program().to_owned();
        command = Command::new(exe);
        command.args(args);
        drop(reservation);
    }
    command
        .current_dir(root)
        .envs(super::environment::variables(
            root,
            match s.profile.adapter.as_str() {
                "debugpy" => "python3",
                "delve" => "go",
                "js-debug" => "node",
                "lldb-dap" => "clang",
                "php-debug" => "php",
                "bash-debug" => "bash",
                "lua-debug" => "lua",
                _ => "",
            },
        )?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut guard = s.child.lock().unwrap();
    anyhow::ensure!(
        !s.closed.load(Ordering::Acquire),
        "Debug session was canceled"
    );
    let mut child = super::tasks::spawn_command(command)?;
    let stdin = child.stdin().take().context("Missing adapter stdin")?;
    let stdout = child.stdout().take().context("Missing adapter stdout")?;
    let stderr = child.stderr().take().context("Missing adapter stderr")?;
    *guard = Some(child);
    drop(guard);
    let owner = s.clone();
    std::thread::spawn(move || loop {
        let status = {
            let mut child = owner.child.lock().unwrap();
            child.as_mut().map(|c| c.try_wait())
        };
        match status {
            Some(Ok(Some(_))) => {
                end(&owner, None);
                owner.child.lock().unwrap().take();
                break;
            }
            Some(Err(e)) => {
                end(&owner, Some(e.to_string()));
                kill(&owner);
                if let Some(mut child) = owner.child.lock().unwrap().take() {
                    let _ = child.wait();
                }
                break;
            }
            None => break,
            _ => std::thread::sleep(Duration::from_millis(100)),
        }
    });
    capture(s.clone(), Box::new(stderr), "stderr");
    let reader: Box<dyn Read + Send>;
    if tcp {
        drop(stdin);
        capture(s.clone(), Box::new(stdout), "stdout");
        let deadline = Instant::now() + Duration::from_secs(10);
        let stream = loop {
            anyhow::ensure!(
                !s.closed.load(Ordering::Acquire),
                "Debug session ended while connecting"
            );
            if let Ok(stream) = TcpStream::connect_timeout(
                &format!("127.0.0.1:{port}").parse()?,
                Duration::from_millis(100),
            ) {
                break stream;
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "Debug adapter TCP connection timed out"
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        reader = Box::new(stream.try_clone()?);
        connect_writer(s, Box::new(stream))?;
    } else {
        reader = Box::new(stdout);
        connect_writer(s, Box::new(stdin))?;
    }
    let owner = s.clone();
    std::thread::spawn(move || read_loop(owner, reader));
    let capabilities = call(
        s,
        "initialize",
        json!({"clientID":"koma","clientName":"Koma","adapterID":s.profile.adapter,"pathFormat":"path","linesStartAt1":true,"columnsStartAt1":true,"supportsVariableType":true,"supportsRunInTerminalRequest":true,"supportsStartDebuggingRequest":false,"supportsProgressReporting":false}),
    )?;
    s.state.lock().unwrap().capabilities = capabilities.clone();
    let launching = begin(s, &s.profile.request, configuration)?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if s.state.lock().unwrap().initialized {
            break;
        }
        anyhow::ensure!(
            !s.closed.load(Ordering::Acquire) && Instant::now() < deadline,
            "Adapter did not initialize the debug session"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    for (path, points) in points {
        set_points(s, &path, points)?;
    }
    if let Some(filters) = capabilities["exceptionBreakpointFilters"].as_array() {
        let selected = s.profile.exception_filters.clone().unwrap_or_else(|| {
            filters
                .iter()
                .filter(|f| f["default"] == true)
                .filter_map(|f| f["filter"].as_str().map(str::to_owned))
                .collect()
        });
        anyhow::ensure!(
            selected
                .iter()
                .all(|id| filters.iter().any(|f| f["filter"] == *id)),
            "Unsupported exception filter in debug profile"
        );
        call(s, "setExceptionBreakpoints", json!({"filters":selected}))?;
        s.state.lock().unwrap().exception_filters = selected;
    }
    if capabilities["supportsConfigurationDoneRequest"] == true {
        call(s, "configurationDone", json!({}))?;
    }
    finish(s, launching, Duration::from_secs(120))?;
    let mut state = s.state.lock().unwrap();
    if state.status == "starting" {
        state.status = "running".into();
    }
    Ok(())
}
fn capture(s: Arc<Session>, mut reader: Box<dyn Read + Send>, category: &'static str) {
    std::thread::spawn(move || {
        let mut bytes = [0; 4096];
        while let Ok(n) = reader.read(&mut bytes) {
            if n == 0 {
                break;
            }
            emit(
                &s,
                "output",
                json!({"category":category,"output":String::from_utf8_lossy(&bytes[..n])}),
            );
        }
    });
}
fn set_points(s: &Session, path: &str, points: Value) -> Result<Value> {
    let value = call(
        s,
        "setBreakpoints",
        json!({"source":{"path":path},"breakpoints":points}),
    )?;
    s.state
        .lock()
        .unwrap()
        .breakpoints
        .insert(path.into(), value.clone());
    Ok(value)
}
pub(super) fn request(
    workspace: &WorkspaceRef,
    id: &str,
    command: &str,
    args: &Value,
    generation: Option<u64>,
) -> Result<Value> {
    let s = get(workspace, id)?;
    anyhow::ensure!(
        matches!(
            command,
            "threads"
                | "stackTrace"
                | "scopes"
                | "variables"
                | "evaluate"
                | "continue"
                | "pause"
                | "next"
                | "stepIn"
                | "stepOut"
                | "setBreakpoints"
                | "setExceptionBreakpoints"
                | "source"
        ),
        "Unsupported debugger request"
    );
    let stopped = matches!(
        command,
        "stackTrace"
            | "source"
            | "scopes"
            | "variables"
            | "evaluate"
            | "next"
            | "stepIn"
            | "stepOut"
            | "continue"
    );
    if stopped {
        let state = s.state.lock().unwrap();
        anyhow::ensure!(
            state.status == "stopped" && generation == Some(state.generation),
            "The paused state changed; refresh debugger data"
        );
    }
    if command == "setBreakpoints" {
        let path = args["path"].as_str().context("Missing source path")?;
        let points = validate_breakpoints(
            &PathBuf::from(&workspace.root).canonicalize()?,
            &json!({path:args["breakpoints"]}),
        )?;
        let (path, points) = points.into_iter().next().unwrap();
        return set_points(&s, &path, points);
    }
    let exception_filters = if command == "setExceptionBreakpoints" {
        let filters: Vec<String> = serde_json::from_value(args["filters"].clone())?;
        let state = s.state.lock().unwrap();
        anyhow::ensure!(
            filters.len() <= 100
                && filters
                    .iter()
                    .all(|id| state.capabilities["exceptionBreakpointFilters"]
                        .as_array()
                        .is_some_and(|all| all.iter().any(|f| f["filter"] == *id))),
            "Unsupported exception breakpoint filter"
        );
        Some(filters)
    } else {
        None
    };
    let value = call(&s, command, args.clone())?;
    if let Some(filters) = exception_filters {
        s.state.lock().unwrap().exception_filters = filters;
    }
    if stopped {
        let mut state = s.state.lock().unwrap();
        if matches!(command, "continue" | "next" | "stepIn" | "stepOut") {
            if generation == Some(state.generation) {
                state.status = "running".into();
                state.generation += 1;
            }
        } else {
            anyhow::ensure!(
                generation == Some(state.generation),
                "Debugger data became stale after resuming"
            );
        }
    }
    Ok(value)
}
pub(super) fn stop(workspace: &WorkspaceRef, id: &str) -> Result<Value> {
    let s = get(workspace, id)?;
    if !s.closed.load(Ordering::Acquire) {
        if let Ok(request) = begin(
            &s,
            "disconnect",
            json!({"terminateDebuggee":s.profile.request=="launch","suspendDebuggee":false}),
        ) {
            let _ = finish(&s, request, Duration::from_secs(2));
        }
        end(&s, None);
        kill(&s);
    }
    Ok(snapshot(&s))
}
pub(super) fn shutdown() {
    for s in registry().lock().unwrap().iter() {
        end(s, None);
        kill(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dap_frames_are_length_delimited_and_bounded() {
        let payload = br#"{"type":"event","event":"stopped"}"#;
        let frame = format!(
            "Content-Length: {}\r\n\r\n{}",
            payload.len(),
            String::from_utf8_lossy(payload)
        );
        assert_eq!(
            read_message(&mut std::io::Cursor::new(frame)).unwrap()["event"],
            "stopped"
        );
        assert!(
            read_message(&mut std::io::Cursor::new("Content-Length: 8388609\r\n\r\n")).is_err()
        );
        assert!(read_message(&mut std::io::Cursor::new(
            "Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}"
        ))
        .is_err());
    }
    #[test]
    fn missing_file_cannot_silently_launch_another_target() {
        let mut config = json!({"program":"${file}","cwd":"${workspaceFolder}"});
        assert!(expand(&mut config, "/project", None).is_err());
        expand(&mut config, "/project", Some("/project/main.py")).unwrap();
        assert_eq!(config["program"], "/project/main.py");
        assert_eq!(config["cwd"], "/project");
    }
}

pub(super) fn has_active() -> bool {
    registry()
        .lock()
        .unwrap()
        .iter()
        .any(|session| !session.closed.load(Ordering::Acquire))
}

fn run_terminal(s: &Session, args: &Value) -> Result<Value> {
    anyhow::ensure!(
        args["kind"]
            .as_str()
            .is_none_or(|kind| kind == "integrated"),
        "Use the integrated debug terminal"
    );
    anyhow::ensure!(
        args["argsCanBeInterpretedByShell"] != true,
        "Shell-interpreted adapter arguments are not supported"
    );
    let values: Vec<String> = serde_json::from_value(args["args"].clone())?;
    anyhow::ensure!(
        !values.is_empty()
            && values.len() <= 256
            && values.iter().all(|v| v.len() <= 32768 && !v.contains('\0')),
        "Invalid debug terminal arguments"
    );
    let root = PathBuf::from(&s.workspace.root).canonicalize()?;
    let cwd = PathBuf::from(args["cwd"].as_str().unwrap_or(&s.workspace.root)).canonicalize()?;
    anyhow::ensure!(
        cwd.starts_with(&root) && cwd.is_dir(),
        "Debug terminal cwd must be inside the workspace"
    );
    let mut env = BTreeMap::new();
    let mut remove = Vec::new();
    if let Some(fields) = args["env"].as_object() {
        anyhow::ensure!(fields.len() <= 100, "Too many debug environment entries");
        for (key, value) in fields {
            anyhow::ensure!(
                !key.is_empty() && !key.contains(['=', '\0']),
                "Invalid debug environment key"
            );
            if value.is_null() {
                remove.push(key.clone());
            } else {
                let value = value
                    .as_str()
                    .context("Debug environment values must be strings or null")?;
                anyhow::ensure!(!value.contains('\0'), "Invalid debug environment value");
                env.insert(key.clone(), value.to_string());
            }
        }
    }
    let mut terminals = s.terminals.lock().unwrap();
    anyhow::ensure!(
        !s.closed.load(Ordering::Acquire) && terminals.len() < 4,
        "Debug session is closed or has too many terminals"
    );
    let run = super::tasks::run_commands(
        &s.workspace,
        &root,
        &format!("debug-terminal:{}:{}", s.id, terminals.len()),
        args["title"].as_str().unwrap_or("Debug terminal"),
        vec![
            json!({"command":values[0],"args":&values[1..],"cwd":cwd.strip_prefix(&root)?.to_string_lossy(),"env":env,"envRemove":remove,"interactive":true}),
        ],
    )?;
    let id = run["id"].as_str().unwrap().to_string();
    terminals.push(id.clone());
    drop(terminals);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        anyhow::ensure!(!s.closed.load(Ordering::Acquire), "Debug session ended");
        if let Some(pid) = super::tasks::process_id(&s.workspace, &id)? {
            #[cfg(windows)]
            return Ok(json!({"shellProcessId":pid}));
            #[cfg(not(windows))]
            return Ok(json!({"processId":pid}));
        }
        let summary = super::tasks::summary(&s.workspace, &id)?;
        anyhow::ensure!(
            matches!(summary["status"].as_str(), Some("queued" | "running"))
                && Instant::now() < deadline,
            "Debug terminal failed to start: {}",
            summary["error"]
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}
