//! Unit tests for the stdio↔sock frame proxy.
//!
//! Uses tokio unix socket pairs + duplex stdio — no real daemon, no SSH.

use super::*;

#[cfg(unix)]
use crate::ipc::frame::{self, FrameReader};
#[cfg(unix)]
use std::os::unix::net::UnixStream as StdUnixStream;
#[cfg(unix)]
use tokio::io::{duplex, AsyncReadExt};
#[cfg(unix)]
use tokio::net::UnixStream;

#[cfg(unix)]
fn framed(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Pair of connected tokio [`UnixStream`]s (the real [`IpcStream`] on unix).
#[cfg(unix)]
async fn unix_pair() -> (UnixStream, UnixStream) {
    let (a, b) = StdUnixStream::pair().expect("unix pair");
    a.set_nonblocking(true).unwrap();
    b.set_nonblocking(true).unwrap();
    (
        UnixStream::from_std(a).expect("tokio a"),
        UnixStream::from_std(b).expect("tokio b"),
    )
}

#[cfg(unix)]
#[tokio::test]
async fn proxy_forwards_frames_both_ways() {
    let (sock_bridge, mut sock_daemon) = unix_pair().await;
    let (mut client_stdin_w, client_stdin_r) = duplex(64 * 1024);
    let (client_stdout_w, mut client_stdout_r) = duplex(64 * 1024);

    let bridge = tokio::spawn(async move {
        proxy_frames(sock_bridge, &mut { client_stdin_r }, &mut { client_stdout_w })
            .await
            .unwrap();
    });

    let client_to_daemon = b"{\"type\":\"Attach\"}";
    let daemon_to_client = b"{\"type\":\"Hello\"}";

    // Client → daemon via bridge
    {
        use tokio::io::AsyncWriteExt;
        client_stdin_w
            .write_all(&framed(client_to_daemon))
            .await
            .unwrap();
        client_stdin_w.flush().await.unwrap();
    }

    let mut sock_reader = FrameReader::new();
    let got = frame::read_frame_from(&mut sock_daemon, &mut sock_reader)
        .await
        .unwrap();
    assert_eq!(got, client_to_daemon);

    // Daemon → client via bridge
    frame::write_frame_to(&mut sock_daemon, daemon_to_client)
        .await
        .unwrap();

    let mut out_reader = FrameReader::new();
    let got = frame::read_frame_from(&mut client_stdout_r, &mut out_reader)
        .await
        .unwrap();
    assert_eq!(got, daemon_to_client);

    // Client EOF ends the bridge.
    drop(client_stdin_w);
    drop(sock_daemon);

    let _ = tokio::time::timeout(Duration::from_secs(2), bridge)
        .await
        .expect("bridge should finish after EOF")
        .expect("bridge task");
}

#[cfg(unix)]
#[tokio::test]
async fn stdio_eof_does_not_inject_quit_daemon() {
    // Capture every byte written toward the "socket" (daemon). After client
    // EOF, the bridge must not append a QuitDaemon frame.
    let (sock_bridge, mut sock_daemon) = unix_pair().await;
    let (mut client_stdin_w, client_stdin_r) = duplex(64 * 1024);
    let (client_stdout_w, _client_stdout_r) = duplex(64 * 1024);

    let bridge = tokio::spawn(async move {
        proxy_frames(sock_bridge, &mut { client_stdin_r }, &mut { client_stdout_w })
            .await
            .unwrap();
    });

    let detach = br#"{"type":"Detach"}"#;
    {
        use tokio::io::AsyncWriteExt;
        client_stdin_w.write_all(&framed(detach)).await.unwrap();
        client_stdin_w.flush().await.unwrap();
    }

    // Read the one Detach frame the client sent.
    let mut sock_reader = FrameReader::new();
    let got = frame::read_frame_from(&mut sock_daemon, &mut sock_reader)
        .await
        .unwrap();
    assert_eq!(got, detach);

    // Close client write half (stdio EOF) — bridge must exit without writing more.
    drop(client_stdin_w);

    // Collect any further bytes from the sock for a short window.
    let mut trailing = Vec::new();
    let mut buf = [0u8; 256];
    let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, sock_daemon.read(&mut buf)).await {
            Ok(Ok(0)) => break, // clean EOF from bridge shutdown
            Ok(Ok(n)) => trailing.extend_from_slice(&buf[..n]),
            Ok(Err(_)) => break,
            Err(_) => break,
        }
    }

    assert!(
        trailing.is_empty(),
        "bridge must not write after client EOF (got {} trailing bytes: {:?})",
        trailing.len(),
        String::from_utf8_lossy(&trailing)
    );
    let as_str = String::from_utf8_lossy(&trailing);
    assert!(
        !as_str.contains("QuitDaemon"),
        "EOF must not inject QuitDaemon: {as_str}"
    );

    drop(sock_daemon);
    let _ = tokio::time::timeout(Duration::from_secs(2), bridge)
        .await
        .expect("bridge should finish")
        .expect("bridge task");
}

#[test]
fn bridge_child_wait_is_bounded() {
    // Leave-path contract: never hang forever waiting on a wedged SSH child.
    assert!(BRIDGE_CHILD_WAIT <= Duration::from_secs(5));
    assert!(BRIDGE_CHILD_WAIT >= Duration::from_millis(100));
}
