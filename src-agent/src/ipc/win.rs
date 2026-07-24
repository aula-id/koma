//! Windows named-pipe backend for the IPC transport (phase B1).
//!
//! Provides the SAME shapes the unix side exposes as pure type aliases in
//! [`super`] — [`IpcStream`], [`IpcListener`], [`SyncIpcStream`] — but over
//! `tokio::net::windows::named_pipe` instead of unix-domain sockets. The rest of the
//! IPC stack (framing, server accept loop, client connect, the sync management
//! clients) is transport-agnostic and consumes these through the aliases, so nothing
//! else changes shape.
//!
//! # Why a pipe name is the rendezvous, not a socket file
//!
//! A unix daemon binds `~/.koma/run/<id>.sock`; a Windows daemon instead owns the
//! named pipe `\\.\pipe\koma-<id>` (see [`crate::model::store`]). A named pipe is NOT
//! a filesystem object: it has no parent directory to create, leaves no stale file to
//! unlink, and is released the instant its owning process dies. The FIRST server
//! instance is created with `first_pipe_instance(true)`, which FAILS if a live daemon
//! already holds the name — that failure IS the bind-as-oracle liveness contract
//! (someone else is already the daemon), exactly mirroring a unix `bind` failing with
//! `AddrInUse`.
//!
//! # DACL hardening (ship-blocker)
//!
//! A default Windows named pipe is readable by `Everyone`/`Anonymous`. Every server
//! instance created here — the first AND every pre-armed successor — carries a
//! hardened security descriptor that grants access ONLY to `SYSTEM`, the builtin
//! `Administrators`, and the creating user (owner-rights), with NO `Everyone` or
//! `Anonymous` ACE. See [`SecurityDescriptor`].

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

/// Raw OS error `ERROR_PIPE_BUSY` (231) — "all pipe instances are busy". Returned by
/// a client `open` while the server is momentarily between pre-armed instances (the
/// hand-off window in [`IpcListener::accept`]); the fix is a brief back-off + retry,
/// never a hard failure. A truly-absent pipe returns `NotFound`, which is NOT retried
/// (it is the bind-as-oracle "no daemon" signal).
const ERROR_PIPE_BUSY: i32 = 231;

/// Client-connect retry budget on `ERROR_PIPE_BUSY`: ~100 * 50ms ≈ 5s, comfortably
/// covering the sub-millisecond re-arm window while still giving up on a wedged pipe.
const CONNECT_BUSY_RETRIES: usize = 100;
/// Back-off between `ERROR_PIPE_BUSY` retries.
const CONNECT_BUSY_BACKOFF: Duration = Duration::from_millis(50);

/// A connected named-pipe endpoint — the server-accepted end (from
/// [`IpcListener::accept`]) or the client end (from [`IpcStream::connect`]). Both
/// inner tokio types implement `AsyncRead`/`AsyncWrite`; this enum delegates to
/// whichever it holds, so callers treat it exactly like the unix `UnixStream`.
pub enum IpcStream {
    /// The server side of an accepted connection.
    Server(NamedPipeServer),
    /// The client side of a `connect`.
    Client(NamedPipeClient),
}

impl IpcStream {
    /// Client-side connect to the pipe at `path` (mirrors `UnixStream::connect`).
    ///
    /// Retries on `ERROR_PIPE_BUSY` (the server is between pre-armed instances),
    /// bounded by [`CONNECT_BUSY_RETRIES`]; any other error — notably `NotFound`, the
    /// "no daemon is listening" signal the spawn-or-attach logic keys on — fails fast.
    pub async fn connect(path: &Path) -> io::Result<Self> {
        let mut attempts: usize = 0;
        loop {
            match ClientOptions::new().open(path) {
                Ok(client) => return Ok(IpcStream::Client(client)),
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    attempts += 1;
                    if attempts > CONNECT_BUSY_RETRIES {
                        return Err(e);
                    }
                    tokio::time::sleep(CONNECT_BUSY_BACKOFF).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl AsyncRead for IpcStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            IpcStream::Server(s) => Pin::new(s).poll_read(cx, buf),
            IpcStream::Client(c) => Pin::new(c).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for IpcStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            IpcStream::Server(s) => Pin::new(s).poll_write(cx, buf),
            IpcStream::Client(c) => Pin::new(c).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            IpcStream::Server(s) => Pin::new(s).poll_flush(cx),
            IpcStream::Client(c) => Pin::new(c).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            IpcStream::Server(s) => Pin::new(s).poll_shutdown(cx),
            IpcStream::Client(c) => Pin::new(c).poll_shutdown(cx),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            IpcStream::Server(s) => Pin::new(s).poll_write_vectored(cx, bufs),
            IpcStream::Client(c) => Pin::new(c).poll_write_vectored(cx, bufs),
        }
    }

    fn is_write_vectored(&self) -> bool {
        match self {
            IpcStream::Server(s) => s.is_write_vectored(),
            IpcStream::Client(c) => c.is_write_vectored(),
        }
    }
}

/// Daemon-side listener over a named pipe: owns the pipe name plus ONE pre-armed
/// server instance waiting for the next client. Mirrors `UnixListener`, including an
/// `accept(&self)` shape (via interior mutability) so the shared `server.rs`
/// accept/accept-loop callers need no per-platform changes.
pub struct IpcListener {
    /// The pipe name (`\\.\pipe\koma-…`) every instance is created on.
    pipe_name: OsString,
    /// The pre-armed next server instance. Interior mutability keeps `accept` a
    /// `&self` method (matching `UnixListener::accept`); never actually contended,
    /// since a single loop drives accept per listener.
    armed: std::sync::Mutex<Option<NamedPipeServer>>,
}

impl IpcListener {
    /// Claim the pipe name and create the FIRST (DACL-hardened) server instance.
    ///
    /// `first_pipe_instance(true)` fails if a live daemon already owns the name — that
    /// failure IS the liveness oracle (do not become a second daemon), the same
    /// contract a unix `bind` gives via `AddrInUse`. Must be called within a tokio
    /// runtime context (the instance registers with the reactor), which every
    /// [`crate::ipc::server::bind`] call site already guarantees.
    pub fn bind(path: &Path) -> io::Result<Self> {
        let pipe_name = path.as_os_str().to_owned();
        let first = create_server_instance(&pipe_name, true)?;
        Ok(IpcListener {
            pipe_name,
            armed: std::sync::Mutex::new(Some(first)),
        })
    }

    /// Wait for the next client and return the connected endpoint, re-arming the pipe
    /// for the following client before returning.
    ///
    /// The returned `()` stands in for the unix peer address (anonymous for a named
    /// pipe, and ignored by every caller). Ordering matters: we connect the currently
    /// armed instance, then create the NEXT instance BEFORE handing the connected one
    /// back — the just-connected instance stays alive throughout, so the pipe name
    /// never drops to zero instances. A client racing into the hand-off window sees
    /// `ERROR_PIPE_BUSY` (which [`IpcStream::connect`] retries), never `NotFound`.
    pub async fn accept(&self) -> io::Result<(IpcStream, ())> {
        // Take the currently-armed instance (bind's first, or the previous accept's
        // re-arm). Defensive `None` arm never fires under the single-loop use.
        let server = {
            let mut g = self.armed.lock().unwrap_or_else(|e| e.into_inner());
            g.take()
        };
        let server = match server {
            Some(s) => s,
            None => create_server_instance(&self.pipe_name, false)?,
        };

        // Block until a client connects on this instance.
        server.connect().await?;

        // Re-arm with the next instance (same hardened DACL) so a following client is
        // never refused. Created BEFORE returning `server`, which is still alive here,
        // so there is no zero-instance window.
        let next = create_server_instance(&self.pipe_name, false)?;
        {
            let mut g = self.armed.lock().unwrap_or_else(|e| e.into_inner());
            *g = Some(next);
        }

        Ok((IpcStream::Server(server), ()))
    }
}

/// Blocking (std) counterpart of [`IpcStream`] for the sync management/probe/proxy
/// clients — the Windows twin of `std::os::unix::net::UnixStream`. Wraps a
/// `std::fs::File` opened on the pipe path in read+write mode.
pub struct SyncIpcStream {
    file: std::fs::File,
}

impl SyncIpcStream {
    /// Blocking connect to the pipe at `path` (mirrors `UnixStream::connect`).
    ///
    /// Opening a named pipe for read+write connects a client instance. Retries on
    /// `ERROR_PIPE_BUSY` with a `std::thread::sleep` back-off (bounded); any other
    /// error — including `NotFound`, the "no daemon" signal — propagates verbatim so
    /// callers' `ErrorKind` matching (refused/not-found vs. other) still works.
    pub fn connect(path: &Path) -> io::Result<Self> {
        let mut attempts: usize = 0;
        loop {
            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
            {
                Ok(file) => return Ok(SyncIpcStream { file }),
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    attempts += 1;
                    if attempts > CONNECT_BUSY_RETRIES {
                        return Err(e);
                    }
                    std::thread::sleep(CONNECT_BUSY_BACKOFF);
                }
                Err(e) => return Err(e),
            }
        }
    }

    // Accepted limitation (Windows port): named-pipe ReadFile/WriteFile on a
    // std::fs::File handle blocks the calling thread with no cancellation token;
    // implementing real timeouts requires switching to raw HANDLE + overlapped I/O
    // + WaitForSingleObject, which is a significant refactor. Callers treat timeout
    // failures as non-fatal (.ok()? / let _ =) and payloads are small framed
    // messages, so this is safe in practice.
    pub fn set_read_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
        Ok(())
    }
    pub fn set_write_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
        Ok(())
    }
}

impl Read for SyncIpcStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl Write for SyncIpcStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Create one DACL-hardened named-pipe server instance on `pipe_name`.
///
/// `first` sets `first_pipe_instance` — `true` only for the very first instance (the
/// bind-as-oracle claim), `false` for every pre-armed successor. EVERY instance
/// carries the SAME hardened security attributes, so a later client cannot connect to
/// a weaker successor instance. Must run inside a tokio runtime (registers with the
/// reactor).
fn create_server_instance(pipe_name: &OsStr, first: bool) -> io::Result<NamedPipeServer> {
    // Build the hardened descriptor; it must outlive the create call below (Windows
    // copies it into the kernel object at creation), so it is dropped (LocalFree'd)
    // only when this function returns.
    let sd = SecurityDescriptor::current_user_only()?;
    let mut sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.as_ptr(),
        bInheritHandle: 0,
    };

    let mut opts = ServerOptions::new();
    opts.first_pipe_instance(first);

    // SAFETY: `sa` is a fully-initialized SECURITY_ATTRIBUTES whose descriptor (`sd`)
    // is valid for the entire call and is NOT freed until after `create…` returns.
    // CreateNamedPipe copies the descriptor into the kernel object, so freeing `sd`
    // afterwards (on function return) is correct and leaks nothing.
    let server = unsafe {
        opts.create_with_security_attributes_raw(
            pipe_name,
            &mut sa as *mut SECURITY_ATTRIBUTES as *mut core::ffi::c_void,
        )
    }?;
    Ok(server)
}

/// An owned Windows security descriptor built from an SDDL string, freed on drop.
///
/// The descriptor grants `GENERIC_ALL` to only three trustees and no one else:
/// `SYSTEM` (`SY`), the builtin `Administrators` group (`BA`), and owner-rights
/// (`OW`, i.e. the creating user). The DACL is `P`rotected (no inherited ACEs) and
/// carries NO `Everyone`/`Anonymous` entry, so — unlike a default named pipe — the
/// endpoint is not world-readable.
struct SecurityDescriptor {
    /// `LocalAlloc`-backed descriptor pointer from
    /// `ConvertStringSecurityDescriptorToSecurityDescriptorW`; freed with `LocalFree`.
    psd: PSECURITY_DESCRIPTOR,
}

impl SecurityDescriptor {
    /// Build the current-user-only descriptor from its SDDL.
    fn current_user_only() -> io::Result<Self> {
        // D:  DACL follows
        // P   protected (do not inherit ACEs from a container)
        // (A;;GA;;;SY) allow GENERIC_ALL to Local System
        // (A;;GA;;;BA) allow GENERIC_ALL to builtin Administrators
        // (A;;GA;;;OW) allow GENERIC_ALL to owner-rights (the creating user)
        const SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)";
        let wide: Vec<u16> = SDDL.encode_utf16().chain(std::iter::once(0)).collect();

        let mut psd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `wide` is a valid NUL-terminated UTF-16 string held alive across the
        // call; on success the API allocates `*psd` via LocalAlloc, which `Drop` frees
        // exactly once with LocalFree. The size-out pointer is null (not needed).
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &mut psd,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(SecurityDescriptor { psd })
    }

    /// The raw descriptor pointer, for a `SECURITY_ATTRIBUTES.lpSecurityDescriptor`.
    fn as_ptr(&self) -> PSECURITY_DESCRIPTOR {
        self.psd
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        if !self.psd.is_null() {
            // SAFETY: `psd` was allocated by ConvertStringSecurityDescriptor…W (which
            // uses LocalAlloc); LocalFree is its matching deallocator, called exactly
            // once here on drop.
            unsafe {
                LocalFree(self.psd);
            }
        }
    }
}
