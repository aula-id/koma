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
}
struct Session {
    id: String,
    workspace: WorkspaceRef,
    profile: Profile,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    child: Mutex<Option<Box<dyn ChildWrapper>>>,
    pending: Mutex<BTreeMap<u64, mpsc::SyncSender<Result<Value, String>>>>,
    seq: AtomicU64,
    closed: AtomicBool,
    state: Mutex<State>,
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
    json!({"id":s.id,"workspace":s.workspace,"label":s.profile.label,"request":s.profile.request,"status":state.status,"error":state.error,"capabilities":state.capabilities,"sequence":state.sequence,"generation":state.generation,"breakpoints":state.breakpoints})
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
    json!({"id":s.id,"status":state.status,"error":state.error,"generation":state.generation,"capabilities":state.capabilities,"breakpoints":state.breakpoints})
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
    let mut writer = s.writer.lock().unwrap();
    let writer = writer.as_mut().context("Debug adapter is not connected")?;
    write!(writer, "Content-Length: {}\r\n\r\n", data.len())?;
    writer.write_all(&data)?;
    writer.flush()?;
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
                let seq = s.seq.fetch_add(1, Ordering::Relaxed) + 1;
                let _ = send(
                    &s,
                    &json!({"seq":seq,"type":"response","request_seq":msg["seq"],"command":msg["command"],"success":false,"message":"This client does not support adapter reverse requests"}),
                );
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
            let mut cmd = Command::new(binary("debugpy")?);
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
    let mut profile = profiles
        .into_iter()
        .find(|p| p.id == profile_id)
        .context("Unknown debug profile")?;
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
        closed: AtomicBool::new(false),
        state: Mutex::new(State {
            status: "starting".into(),
            error: None,
            initialized: false,
            capabilities: json!({}),
            events: VecDeque::new(),
            sequence: 0,
            generation: 0,
            breakpoints: BTreeMap::new(),
        }),
    });
    let worker = s.clone();
    let root = root.to_path_buf();
    std::thread::Builder::new()
        .name("coding-debug-start".into())
        .spawn(move || {
            if let Err(e) = setup(&worker, &root, points) {
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
fn setup(s: &Arc<Session>, root: &Path, points: BTreeMap<String, Value>) -> Result<()> {
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
        *s.writer.lock().unwrap() = Some(Box::new(stream));
    } else {
        reader = Box::new(stdout);
        *s.writer.lock().unwrap() = Some(Box::new(stdin));
    }
    let owner = s.clone();
    std::thread::spawn(move || read_loop(owner, reader));
    let capabilities = call(
        s,
        "initialize",
        json!({"clientID":"koma","clientName":"Koma","adapterID":s.profile.adapter,"pathFormat":"path","linesStartAt1":true,"columnsStartAt1":true,"supportsVariableType":true,"supportsRunInTerminalRequest":false,"supportsStartDebuggingRequest":false,"supportsProgressReporting":false}),
    )?;
    s.state.lock().unwrap().capabilities = capabilities.clone();
    let launching = begin(s, &s.profile.request, s.profile.configuration.clone())?;
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
        ),
        "Unsupported debugger request"
    );
    let stopped = matches!(
        command,
        "stackTrace"
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
    let value = call(&s, command, args.clone())?;
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
        let _ = call(
            &s,
            "disconnect",
            json!({"terminateDebuggee":s.profile.request=="launch","suspendDebuggee":false}),
        );
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
