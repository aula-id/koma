//! End-to-end self-test for the IPC transport (`koma --ipc-selftest`).
//!
//! Exercises [`super::frame`] + [`super::server`] + [`super::client`] together so
//! none of them rot into dead code before the daemon loop wires them up. It:
//!
//! 1. binds a unix listener on a tokio task (server side),
//! 2. connects a client to it,
//! 3. round-trips a real [`ClientRequest::ListSessions`] frame client → server,
//!    and a real [`DaemonFrame`] (`seq = 1`, [`DaemonEvent::Ack`]) server → client,
//! 4. asserts BYTE-EQUALITY of each payload after a serde encode → wire → decode →
//!    re-encode cycle (proving both the framing and the protocol serde are stable),
//! 5. tears the connection down and unlinks the socket,
//! 6. prints `OK` / `FAIL` and exits with status 0 / 1.
//!
//! A dedicated socket path (`~/.koma/ipc-selftest.sock`) is used so the test never
//! collides with a real daemon socket. The whole thing runs on a private tokio
//! runtime built here, mirroring the sync `main` entry point.

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use tokio::net::UnixStream;

use super::client;
use super::frame::FrameReader;
use super::proto::{ClientRequest, DaemonEvent, DaemonFrame};
use super::server;
use crate::model::store;

/// Dedicated socket path for the self-test, kept distinct from the real daemon
/// socket so running the test never disturbs a live daemon.
fn selftest_sock_path() -> Result<PathBuf> {
    Ok(store::base_dir()?.join("ipc-selftest.sock"))
}

/// Run the IPC self-test to completion. Prints `koma ipc-selftest: OK` and exits
/// 0 on success, or prints the failure and exits 1. Never returns normally — it
/// always terminates the process (it is a short-circuit CLI mode).
pub fn run() -> ! {
    let code = match run_inner() {
        Ok(()) => {
            println!("koma ipc-selftest: OK");
            0
        }
        Err(e) => {
            eprintln!("koma ipc-selftest: FAIL: {e:#}");
            1
        }
    };
    std::process::exit(code);
}

/// The fallible body: build a runtime, drive the round-trip, clean up. Returns
/// `Err` on any framing/serde/transport mismatch.
fn run_inner() -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    rt.block_on(roundtrip())
}

/// Bind a server, connect a client, and round-trip one frame each direction,
/// asserting byte-equality after serde on both. Unlinks the socket before
/// returning (on success or error).
async fn roundtrip() -> Result<()> {
    let path = selftest_sock_path()?;

    // Bind first (server side) so the client's connect cannot race ahead of it.
    let listener = server::bind(&path).context("bind selftest socket")?;

    // The exact bytes we expect to survive the framing + serde round-trip.
    let request = ClientRequest::ListSessions;
    let request_bytes = serde_json::to_vec(&request).context("encode ListSessions")?;

    let reply = DaemonFrame {
        seq: 1,
        event: DaemonEvent::Ack,
    };
    let reply_bytes = serde_json::to_vec(&reply).context("encode DaemonFrame Ack")?;

    // --- server task: accept one client, verify its request, reply with the Ack ---
    let server_expected = request_bytes.clone();
    let server_reply = reply_bytes.clone();
    let server = tokio::spawn(async move {
        let mut conn = server::accept(&listener)
            .await
            .context("accept client")?;
        let mut reader = FrameReader::new();

        // Read the client's request frame and re-encode it to compare bytes.
        let got = super::frame::read_frame(&mut conn, &mut reader)
            .await
            .context("server read request frame")?;
        let decoded: ClientRequest =
            serde_json::from_slice(&got).context("server decode request")?;
        let reencoded = serde_json::to_vec(&decoded).context("server re-encode request")?;
        if reencoded != server_expected {
            return Err(anyhow!(
                "request bytes mismatch after serde: got {} bytes, expected {}",
                reencoded.len(),
                server_expected.len()
            ));
        }

        // Reply with the Ack frame.
        super::frame::write_frame(&mut conn, &server_reply)
            .await
            .context("server write reply frame")?;
        Ok::<(), anyhow::Error>(())
    });

    // --- client side: connect, send the request, read + verify the reply ---
    let mut stream: UnixStream = client::connect(&path).await.context("client connect")?;
    let mut reader = FrameReader::new();

    client::send_frame(&mut stream, &request_bytes)
        .await
        .context("client send request")?;

    let reply_frame = client::recv_frame(&mut stream, &mut reader)
        .await
        .context("client recv reply")?;
    let decoded_reply: DaemonFrame =
        serde_json::from_slice(&reply_frame).context("client decode reply")?;
    let reencoded_reply =
        serde_json::to_vec(&decoded_reply).context("client re-encode reply")?;

    // Drop the client stream so the server task observes EOF and finishes cleanly.
    drop(stream);

    // Join the server task and surface any error it hit.
    server
        .await
        .context("join server task")?
        .context("server task error")?;

    // Clean up the socket file regardless (best-effort).
    let _ = std::fs::remove_file(&path);

    // Final byte-equality assertions across the wire.
    if reencoded_reply != reply_bytes {
        return Err(anyhow!(
            "reply bytes mismatch after serde: got {} bytes, expected {}",
            reencoded_reply.len(),
            reply_bytes.len()
        ));
    }

    // Sanity: the decoded reply is exactly the Ack at seq 1 we sent.
    if decoded_reply.seq != 1 || !matches!(decoded_reply.event, DaemonEvent::Ack) {
        return Err(anyhow!("reply was not DaemonFrame {{ seq: 1, Ack }}"));
    }

    Ok(())
}
