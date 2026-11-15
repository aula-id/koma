//! Public JSONL adapter. Mutations are serialized through a single IPC owner.
mod binding;
mod coding;
pub(crate) mod configure;
mod connection;
mod dto;
mod events;
mod host;
mod methods;
mod protocol;
mod reconcile;
mod room;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_config;
#[cfg(test)]
mod tests_room_runtime;
#[cfg(test)]
mod tests_room_team;
#[cfg(test)]
mod tests_sessions;
#[cfg(test)]
mod tests_skills;
#[cfg(test)]
mod tests_usage;
use anyhow::{Context, Result};
use binding::Binding;
use protocol::*;
use serde_json::{json, Value};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    sync::{mpsc, watch},
};

/// The only stdout writer. Responses are bounded (one in flight); unsent complete
/// state events coalesce before sequence assignment, independently of responses.
fn event_frames(previous: &mut Option<State>, state: State, seq: &mut u64) -> Vec<Value> {
    let mut frames = Vec::new();
    if previous.as_ref() == Some(&state) {
        return frames;
    }
    let append = previous.as_ref().and_then(|old| {
        let mut same = old.clone();
        same.streaming = state.streaming.clone();
        same.revision = state.revision;
        let before = old.streaming.as_deref().unwrap_or("");
        let after = state.streaming.as_deref().unwrap_or("");
        if same == state
            && state.revision == old.revision + 1
            && after.starts_with(before)
            && after.len() > before.len()
        {
            Some((before.len(), after[before.len()..].to_string()))
        } else {
            None
        }
    });
    *seq += 1;
    if let Some((offset, text)) = append {
        frames.push(json!({"type":"event","event":"output_append","seq":*seq,"revision":state.revision,"offset":offset,"text":text}));
    } else {
        let resynced = previous
            .as_ref()
            .map_or(state.resync > 0, |old| old.resync != state.resync);
        frames.push(
            json!({"type":"event","event":"state","seq":*seq,"state":state,"resynced":resynced}),
        );
        if previous.as_ref().and_then(|s| s.turn.as_ref()) != state.turn.as_ref() {
            *seq += 1;
            frames.push(json!({"type":"event","event":"turn","seq":*seq,"revision":state.revision,"turn":state.turn}));
        }
    }
    *previous = Some(state);
    frames
}
/// Room events come from `queue` and share the envelope `seq`. Those queued
/// before a response are written before it, so a `room.send` echo precedes
/// its response. A held queue opens only once the response that claimed the
/// hold is out, never after an earlier response that was still being written.
async fn writer<W: tokio::io::AsyncWrite + Unpin>(
    mut out: W,
    mut responses: mpsc::Receiver<(Value, Option<u64>)>,
    mut states: watch::Receiver<Option<State>>,
    queue: std::sync::Arc<events::EventQueue>,
) -> Result<()> {
    let mut seq = 0u64;
    let mut previous = None;
    loop {
        let mut released = None;
        let frames = tokio::select! {biased;
         r=responses.recv()=>match r{Some((r,hold))=>{released=hold;let mut f=queue.take_ready(&mut seq);f.push(r);f},None=>break},
         changed=states.changed()=>{if changed.is_err(){break}let s=states.borrow_and_update().clone();let Some(s)=s else{continue};event_frames(&mut previous,s,&mut seq)}
         _=queue.ready()=>queue.take_ready(&mut seq),
        };
        for frame in frames {
            let mut bytes = serde_json::to_vec(&frame)?;
            anyhow::ensure!(bytes.len() <= MAX_LINE, "outbound frame exceeds 40 MiB");
            bytes.push(b'\n');
            tokio::time::timeout(connection::BUDGET, async {
                out.write_all(&bytes).await?;
                out.flush().await
            })
            .await
            .context("stdout stalled")??;
        }
        if let Some(generation) = released {
            queue.response_written(generation);
        }
    }
    Ok(())
}
pub async fn serve() -> Result<()> {
    // Emit and flush hello before even constructing the stdin reader.
    let hello = protocol::hello(&std::env::current_exe()?.to_string_lossy());
    let mut stdout = tokio::io::stdout();
    stdout.write_all(&serde_json::to_vec(&hello)?).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await?;
    drop(stdout);
    let (tx, rx) = mpsc::channel(1);
    let (events, states) = watch::channel(None);
    let mut adapter = Adapter::new(events);
    let mut output = tokio::spawn(writer(
        tokio::io::stdout(),
        rx,
        states,
        std::sync::Arc::clone(&adapter.queue),
    ));
    let (input_tx, mut input_rx) = mpsc::channel(1);
    let input = tokio::spawn(async move {
        let mut stdin = BufReader::new(tokio::io::stdin());
        loop {
            let value = match line(&mut stdin).await {
                Ok(Some(b)) => {
                    serde_json::from_slice::<Request>(&b).context("malformed JSONL request")
                }
                Ok(None) => break,
                Err(e) => Err(e),
            };
            let fatal = value.is_err();
            if input_tx.send(value).await.is_err() || fatal {
                break;
            }
        }
    });
    let mut seen = std::collections::HashSet::new();
    let result:Result<()>=async {
 loop {
  let request=if let Some(c)=adapter.binding.coding_mut(){tokio::select!{
   r=input_rx.recv()=>r,
   f=c.receive()=>{f?;c.flush_resync().await?;continue},
   w=&mut output=>{return w.context("writer task")?;}
  }}else{tokio::select!{r=input_rx.recv()=>r,w=&mut output=>{return w.context("writer task")?;}}};
  let Some(r)=request else{break};let r=r?;
  let (frame,end)=respond(r,&mut seen,&mut adapter).await?;
  // A hold set while this request was answered is released by this frame only.
  let hold=adapter.queue.claim_hold();
  tx.send((frame,hold)).await.context("stdout writer closed")?;
  // The bound coding connection failed while host work ran: end as v1 does.
  if let Some(lost)=adapter.lost.take(){return Err(lost)}
  if end{break}
 }
 Ok(())
 }.await;
    // Closing this socket is sufficient. Never send QuitDaemon, even on errors.
    // A live room is detached with `quit`, which leaves the room running.
    if let Some(room) = adapter.binding.room_mut() {
        room.quit().await;
    }
    let Adapter {
        binding, events, ..
    } = adapter;
    drop(binding);
    input.abort();
    drop(tx);
    drop(events);
    let written = if output.is_finished() {
        Ok(())
    } else {
        match tokio::time::timeout(connection::BUDGET, &mut output).await {
            Ok(r) => r.context("writer task")?,
            Err(e) => {
                output.abort();
                Err(e.into())
            }
        }
    };
    result.and(written)
}
/// What one adapter process holds while it serves requests.
pub struct Adapter {
    pub binding: Binding,
    pub events: watch::Sender<Option<State>>,
    /// Set when the bound coding connection failed while host work ran. The
    /// loop answers the request, then ends with this error.
    pub lost: Option<anyhow::Error>,
    /// Where config reads and writes go.
    pub config: host::config::ConfigStore,
    /// Room events on their way to the stdout writer.
    pub queue: std::sync::Arc<events::EventQueue>,
    /// The secret values scrubbed from room text, kept across bindings.
    pub scrub: room::scrub::SharedScrubber,
    /// Where rooms are found, started and stopped.
    pub room_env: room::RoomEnv,
}
impl Adapter {
    pub fn new(events: watch::Sender<Option<State>>) -> Self {
        Self {
            binding: Binding::Unbound,
            events,
            lost: None,
            config: host::config::ConfigStore::home(),
            queue: std::sync::Arc::new(events::EventQueue::new()),
            scrub: room::scrub::shared(),
            room_env: room::RoomEnv::home(),
        }
    }
    /// Runs blocking host work (disk, SQLite, zip, waiting on another
    /// process) on the blocking pool. Meanwhile the bound coding connection is
    /// still drained, so its daemon never stalls on a full socket and state
    /// events keep flowing. A failure of that connection is kept in `lost`
    /// and the work still runs to completion.
    pub async fn blocking<T, F>(&mut self, work: F) -> Result<T, RpcError>
    where
        F: FnOnce() -> Result<T, RpcError> + Send + 'static,
        T: Send + 'static,
    {
        let mut task = tokio::task::spawn_blocking(work);
        if self.lost.is_none() {
            if let Binding::Coding(c) = &mut self.binding {
                loop {
                    tokio::select! {
                        joined = &mut task => return joined.map_err(|e| RpcError::new("internal", e.to_string()))?,
                        frame = c.receive() => {
                            let drained = match frame {
                                Ok(_) => c.flush_resync().await,
                                Err(e) => Err(e),
                            };
                            if let Err(e) = drained {
                                self.lost = Some(e);
                                break;
                            }
                        }
                    }
                }
            }
        }
        task.await
            .map_err(|e| RpcError::new("internal", e.to_string()))?
    }
}
/// Answers one request. Returns the response frame and whether the process
/// ends after it: always after `detach`, even a failed one, and after an
/// `unknown` outcome on the bound coding connection.
async fn respond(
    r: Request,
    seen: &mut std::collections::HashSet<String>,
    adapter: &mut Adapter,
) -> Result<(Value, bool)> {
    anyhow::ensure!(!r.id.is_empty(), "empty request ID");
    // Bound lifetime correlation storage; never silently forget IDs and allow reuse.
    anyhow::ensure!(
        seen.len() < REQUEST_IDS,
        "request lifetime limit reached; detach and reconnect"
    );
    let id = r.id.clone();
    let method = r.method.clone();
    let legacy = methods::find(&method).is_some_and(|route| route.legacy());
    let response = if !seen.insert(id.clone()) {
        Err(RpcError::new("invalid_request", "duplicate request id"))
    } else {
        dispatch(r, adapter).await
    };
    let fatal = response
        .as_ref()
        .err()
        .is_some_and(|e| e.ends_process(legacy));
    let frame = match response {
        Ok(result) => json!({"type":"response","id":id,"result":result}),
        Err(error) => json!({"type":"response","id":id,"error":error}),
    };
    let frame = if serde_json::to_vec(&frame)?.len() > MAX_LINE {
        json!({"type":"response","id":id,"error":oversized(legacy)})
    } else {
        frame
    };
    Ok((frame, method == "detach" || fatal))
}
/// The error that replaces a response too large for one frame. v1 methods keep
/// their v1 text; a new method's oversized result is a read that sent nothing.
fn oversized(legacy: bool) -> RpcError {
    if legacy {
        RpcError::new("unavailable", "state exceeds frame limit")
    } else {
        RpcError::reason(Reason::FrameLimit, "result exceeds frame limit")
    }
}
async fn dispatch(r: Request, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let Some(route) = methods::find(&r.method) else {
        return Err(RpcError::new("unsupported_method", "unsupported method"));
    };
    route.check(adapter.binding.state())?;
    if r.method == "detach" && adapter.binding.room().is_some() {
        let _: serde_json::Map<String, Value> = params(r.params)?;
        return room::detach(adapter).await;
    }
    (route.handler)(r, adapter).await
}
