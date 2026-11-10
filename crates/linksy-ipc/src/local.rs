//! The local connection between Linksy's programs: a Unix socket on Linux, a named pipe on
//! Windows. Both are addressed by a path; on Windows a path that isn't already a pipe name
//! (`\\.\pipe\…`) becomes the pipe named after its file name.
//!
//! On Windows only the user who started the service (and the system) may open its pipes,
//! like the Unix socket's mode 0600, and remote clients are always refused.

use std::io;
use std::path::Path;

/// One connection, from either end.
#[cfg(unix)]
pub type Stream = tokio::net::UnixStream;
#[cfg(windows)]
pub use self::windows::Stream;

pub type ReadHalf = tokio::io::ReadHalf<Stream>;
pub type WriteHalf = tokio::io::WriteHalf<Stream>;

/// Split a connection into a half to read and a half to write, each usable on its own task.
pub fn split(stream: Stream) -> (ReadHalf, WriteHalf) {
    tokio::io::split(stream)
}

/// Connect to the program listening at `path`.
pub async fn connect(path: &Path) -> io::Result<Stream> {
    #[cfg(unix)]
    {
        tokio::net::UnixStream::connect(path).await
    }
    #[cfg(windows)]
    {
        windows::connect(path).await
    }
}

/// Waits for connections at a path, for this user only.
pub struct Listener {
    #[cfg(unix)]
    inner: tokio::net::UnixListener,
    #[cfg(windows)]
    inner: windows::PipeListener,
}

impl Listener {
    /// Listen at `path`, replacing a stale socket there. On Windows, fails if another
    /// program already serves the pipe.
    pub fn bind(path: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(path);
            let inner = tokio::net::UnixListener::bind(path)?;
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            Ok(Self { inner })
        }
        #[cfg(windows)]
        {
            Ok(Self { inner: windows::PipeListener::bind(path)? })
        }
    }

    pub async fn accept(&mut self) -> io::Result<Stream> {
        #[cfg(unix)]
        {
            self.inner.accept().await.map(|(s, _)| s)
        }
        #[cfg(windows)]
        {
            self.inner.accept().await
        }
    }
}

/// Clean up after a listener (the socket file on Linux; a pipe goes with its last handle).
pub fn remove(path: &Path) {
    #[cfg(unix)]
    let _ = std::fs::remove_file(path);
    #[cfg(windows)]
    let _ = path;
}

#[cfg(windows)]
mod windows {
    use std::ffi::OsString;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions};
    use windows_sys::Win32::Foundation::{ERROR_PIPE_BUSY, LocalFree};
    use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
    use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

    /// `\\.\pipe\…` for a path: itself if it is one, else named after its file name.
    pub fn pipe_name(path: &Path) -> OsString {
        let s = path.as_os_str().to_string_lossy();
        if s.starts_with(r"\\.\pipe\") || s.starts_with(r"\\?\pipe\") {
            return path.as_os_str().to_owned();
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "linksy".into());
        OsString::from(format!(r"\\.\pipe\{name}"))
    }

    pub async fn connect(path: &Path) -> io::Result<Stream> {
        let name = pipe_name(path);
        // Every instance busy (the service is between two accepts): wait a moment.
        for _ in 0..50 {
            match ClientOptions::new().open(&name) {
                Ok(client) => return Ok(Stream::Client(client)),
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => tokio::time::sleep(Duration::from_millis(20)).await,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, "the Linksy service is busy"))
    }

    pub struct PipeListener {
        name: OsString,
        next: NamedPipeServer,
    }

    impl PipeListener {
        pub fn bind(path: &Path) -> io::Result<Self> {
            let name = pipe_name(path);
            let next = create(&name, true)?;
            Ok(Self { name, next })
        }

        pub async fn accept(&mut self) -> io::Result<Stream> {
            self.next.connect().await?;
            let fresh = create(&self.name, false)?;
            Ok(Stream::Server(std::mem::replace(&mut self.next, fresh)))
        }
    }

    /// A pipe instance only this user (its owner) and the system may open.
    fn create(name: &OsString, first: bool) -> io::Result<NamedPipeServer> {
        // Protected DACL: full access for the pipe's owner (OW) and LocalSystem (SY), nobody else.
        let sddl: Vec<u16> = std::ffi::OsStr::new("D:P(A;;GA;;;OW)(A;;GA;;;SY)").encode_wide().chain(Some(0)).collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: a NUL-terminated SDDL string; the descriptor it allocates is freed below.
        let ok = unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut descriptor, std::ptr::null_mut()) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut attributes = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor, bInheritHandle: 0 };
        // SAFETY: `attributes` and the descriptor it points to outlive the call.
        let server = unsafe {
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(name, (&raw mut attributes).cast())
        };
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW
        unsafe { LocalFree(descriptor) };
        server
    }

    /// A named pipe, from the client's or the server's end.
    pub enum Stream {
        Client(NamedPipeClient),
        Server(NamedPipeServer),
    }

    impl AsyncRead for Stream {
        fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
            match self.get_mut() {
                Stream::Client(p) => Pin::new(p).poll_read(cx, buf),
                Stream::Server(p) => Pin::new(p).poll_read(cx, buf),
            }
        }
    }

    impl AsyncWrite for Stream {
        fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
            match self.get_mut() {
                Stream::Client(p) => Pin::new(p).poll_write(cx, buf),
                Stream::Server(p) => Pin::new(p).poll_write(cx, buf),
            }
        }

        fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            match self.get_mut() {
                Stream::Client(p) => Pin::new(p).poll_flush(cx),
                Stream::Server(p) => Pin::new(p).poll_flush(cx),
            }
        }

        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            match self.get_mut() {
                Stream::Client(p) => Pin::new(p).poll_shutdown(cx),
                Stream::Server(p) => Pin::new(p).poll_shutdown(cx),
            }
        }
    }
}
