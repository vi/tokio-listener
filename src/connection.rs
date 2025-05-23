#[allow(unused_imports)]
use std::{
    ffi::c_int,
    fmt::Display,
    net::SocketAddr,
    path::PathBuf,
    pin::Pin,
    str::FromStr,
    sync::Arc,
    task::{ready, Context, Poll},
    time::Duration,
};

use pin_project::pin_project;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    sync::oneshot::Sender,
};
use tracing::{debug, warn};

#[cfg(unix)]
use tokio::net::UnixStream;

/// Socket-like thing supported by tokio_listner.
///
/// With `boxed` crate feature `Pin<Box<dyn AsyncReadWrite + Send>>` can be used as a [`Connection`] variant.
pub trait AsyncReadWrite: AsyncRead + AsyncWrite + std::fmt::Debug {}
impl<T: AsyncRead + AsyncWrite + std::fmt::Debug> AsyncReadWrite for T {}

#[cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))]
use tokio_vsock::VsockStream;

/// Accepted connection, which can be a TCP socket, AF_UNIX stream socket or a stdin/stdout pair.
///
/// Although inner enum is private, you can use methods or `From` impls to convert this to/from usual Tokio types.
#[pin_project]
pub struct Connection(#[pin] pub(crate) ConnectionImpl);

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            ConnectionImpl::Tcp(_) => f.write_str("Connection(tcp)"),
            #[cfg(all(feature = "unix", unix))]
            ConnectionImpl::Unix(_) => f.write_str("Connection(unix)"),
            #[cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))]
            ConnectionImpl::Vsock(_) => f.write_str("Connection(vsock)"),
            #[cfg(feature = "inetd")]
            ConnectionImpl::Stdio(_, _, _) => f.write_str("Connection(stdio)"),
            #[cfg(feature = "duplex_variant")]
            ConnectionImpl::Duplex(_) => f.write_str("Connection(DuplexStream)"),
            #[cfg(feature = "boxed_variant")]
            ConnectionImpl::Boxed(ref b) => f.debug_struct("Connection").field("0", b).finish(),
            #[cfg(feature = "dummy_variant")]
            ConnectionImpl::Dummy(_) => f.write_str("Connection(Dummy)"),
        }
    }
}

#[derive(Debug)]
#[pin_project(project = ConnectionImplProj)]
pub(crate) enum ConnectionImpl {
    Tcp(#[pin] TcpStream),
    #[cfg(all(feature = "unix", unix))]
    Unix(#[pin] UnixStream),
    #[cfg(feature = "inetd")]
    Stdio(
        #[pin] tokio::io::Stdin,
        #[pin] tokio::io::Stdout,
        Option<Sender<()>>,
    ),
    #[cfg(feature = "duplex_variant")]
    Duplex(#[pin] tokio::io::DuplexStream),
    #[cfg(feature = "dummy_variant")]
    Dummy(#[pin] tokio::io::Empty),
    #[cfg(feature = "boxed_variant")]
    Boxed(Pin<Box<dyn AsyncReadWrite + Send>>),
    #[cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))]
    Vsock(#[pin] VsockStream),
}

#[allow(missing_docs)]
#[allow(clippy::missing_errors_doc)]
impl Connection {
    pub fn try_into_tcp(self) -> Result<TcpStream, Self> {
        if let ConnectionImpl::Tcp(s) = self.0 {
            Ok(s)
        } else {
            Err(self)
        }
    }
    #[cfg(all(feature = "unix", unix))]
    #[cfg_attr(docsrs_alt, doc(cfg(all(feature = "unix", unix))))]
    pub fn try_into_unix(self) -> Result<UnixStream, Self> {
        if let ConnectionImpl::Unix(s) = self.0 {
            Ok(s)
        } else {
            Err(self)
        }
    }
    #[cfg(feature = "inetd")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "inetd")))]
    /// Get parts of the connection in case of inted mode is used.
    ///
    /// Third tuple part (Sender) should be used to signal [`Listener`] to exit from listening loop,
    /// allowing proper timing of listening termination - without trying to wait for second client in inetd mode,
    /// but also without exiting prematurely, while the client is still being served, as exiting the listening loop may
    /// cause the whole process to finish.
    pub fn try_into_stdio(
        self,
    ) -> Result<(tokio::io::Stdin, tokio::io::Stdout, Option<Sender<()>>), Self> {
        if let ConnectionImpl::Stdio(i, o, f) = self.0 {
            Ok((i, o, f))
        } else {
            Err(self)
        }
    }

    pub fn try_borrow_tcp(&self) -> Option<&TcpStream> {
        if let ConnectionImpl::Tcp(ref s) = self.0 {
            Some(s)
        } else {
            None
        }
    }
    #[cfg(all(feature = "unix", unix))]
    #[cfg_attr(docsrs_alt, doc(cfg(all(feature = "unix", unix))))]
    pub fn try_borrow_unix(&self) -> Option<&UnixStream> {
        if let ConnectionImpl::Unix(ref s) = self.0 {
            Some(s)
        } else {
            None
        }
    }
    #[cfg(feature = "inetd")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "inetd")))]
    pub fn try_borrow_stdio(&self) -> Option<(&tokio::io::Stdin, &tokio::io::Stdout)> {
        if let ConnectionImpl::Stdio(ref i, ref o, ..) = self.0 {
            Some((i, o))
        } else {
            None
        }
    }

    #[cfg(feature = "duplex_variant")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "duplex_variant")))]
    pub fn try_into_duplex(self) -> Result<tokio::io::DuplexStream, Self> {
        if let ConnectionImpl::Duplex(s) = self.0 {
            Ok(s)
        } else {
            Err(self)
        }
    }
    #[cfg(feature = "duplex_variant")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "duplex_variant")))]
    pub fn try_borrow_duplex(&self) -> Option<&tokio::io::DuplexStream> {
        if let ConnectionImpl::Duplex(ref s) = self.0 {
            Some(s)
        } else {
            None
        }
    }

    #[cfg(feature = "dummy_variant")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "dummy_variant")))]
    pub fn is_dummy(&self) -> bool {
        if let ConnectionImpl::Dummy(ref _s) = self.0 {
            true
        } else {
            false
        }
    }

    #[cfg(feature = "boxed_variant")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "boxed_variant")))]
    pub fn try_into_boxed(self) -> Result<Pin<Box<dyn AsyncReadWrite + Send>>, Self> {
        if let ConnectionImpl::Boxed(s) = self.0 {
            Ok(s)
        } else {
            Err(self)
        }
    }
    #[cfg(feature = "boxed_variant")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "boxed_variant")))]
    pub fn try_borrow_boxed(&self) -> Option<&Pin<Box<dyn AsyncReadWrite + Send>>> {
        if let ConnectionImpl::Boxed(ref s) = self.0 {
            Some(s)
        } else {
            None
        }
    }
    
    #[cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))]
    #[cfg_attr(docsrs_alt, doc(cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))))]
    pub fn try_into_vsock(self) -> Result<VsockStream, Self> {
        if let ConnectionImpl::Vsock(vsock) = self.0 {
            Ok(vsock)
        } else {
            Err(self)
        }
    }
    #[cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))]
    #[cfg_attr(docsrs_alt, doc(cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))))]
    pub fn try_borrow_vsock(&self) -> Option<&VsockStream> {
        if let ConnectionImpl::Vsock(ref vsock) = self.0 {
            Some(vsock)
        } else {
            None
        }
    }
}

impl From<TcpStream> for Connection {
    fn from(s: TcpStream) -> Self {
        Connection(ConnectionImpl::Tcp(s))
    }
}
#[cfg(all(feature = "unix", unix))]
#[cfg_attr(docsrs_alt, doc(cfg(all(feature = "unix", unix))))]
impl From<UnixStream> for Connection {
    fn from(s: UnixStream) -> Self {
        Connection(ConnectionImpl::Unix(s))
    }
}
#[cfg(feature = "inetd")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "inetd")))]
impl From<(tokio::io::Stdin, tokio::io::Stdout, Option<Sender<()>>)> for Connection {
    fn from(s: (tokio::io::Stdin, tokio::io::Stdout, Option<Sender<()>>)) -> Self {
        Connection(ConnectionImpl::Stdio(s.0, s.1, s.2))
    }
}

#[cfg(feature = "duplex_variant")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "duplex_variant")))]
impl From<tokio::io::DuplexStream> for Connection {
    fn from(s: tokio::io::DuplexStream) -> Self {
        Connection(ConnectionImpl::Duplex(s))
    }
}
#[cfg(feature = "dummy_variant")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "dummy_variant")))]
impl From<tokio::io::Empty> for Connection {
    fn from(s: tokio::io::Empty) -> Self {
        Connection(ConnectionImpl::Dummy(s))
    }
}
#[cfg(feature = "boxed_variant")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "boxed_variant")))]
impl From<Pin<Box<dyn AsyncReadWrite + Send>>> for Connection {
    fn from(s: Pin<Box<dyn AsyncReadWrite + Send>>) -> Self {
        Connection(ConnectionImpl::Boxed(s))
    }
}

#[cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))]
#[cfg_attr(docsrs_alt, doc(cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))))]
impl From<VsockStream> for Connection {
    fn from(s: VsockStream) ->Self {
        Connection(ConnectionImpl::Vsock(s))
    }
}

impl AsyncRead for Connection {
    #[inline]
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let q: Pin<&mut ConnectionImpl> = self.project().0;
        match q.project() {
            ConnectionImplProj::Tcp(s) => s.poll_read(cx, buf),
            #[cfg(all(feature = "unix", unix))]
            ConnectionImplProj::Unix(s) => s.poll_read(cx, buf),
            #[cfg(feature = "inetd")]
            ConnectionImplProj::Stdio(s, _, _) => s.poll_read(cx, buf),
            #[cfg(feature = "duplex_variant")]
            ConnectionImplProj::Duplex(s) => s.poll_read(cx, buf),
            #[cfg(feature = "boxed_variant")]
            ConnectionImplProj::Boxed(s) => s.as_mut().poll_read(cx, buf),
            #[cfg(feature = "dummy_variant")]
            ConnectionImplProj::Dummy(s) => s.poll_read(cx, buf),
            #[cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))]
            ConnectionImplProj::Vsock(s) => s.poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Connection {
    #[inline]
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        let q: Pin<&mut ConnectionImpl> = self.project().0;
        match q.project() {
            ConnectionImplProj::Tcp(s) => s.poll_write(cx, buf),
            #[cfg(all(feature = "unix", unix))]
            ConnectionImplProj::Unix(s) => s.poll_write(cx, buf),
            #[cfg(feature = "inetd")]
            ConnectionImplProj::Stdio(_, s, _) => s.poll_write(cx, buf),
            #[cfg(feature = "duplex_variant")]
            ConnectionImplProj::Duplex(s) => s.poll_write(cx, buf),
            #[cfg(feature = "boxed_variant")]
            ConnectionImplProj::Boxed(s) => s.as_mut().poll_write(cx, buf),
            #[cfg(feature = "dummy_variant")]
            ConnectionImplProj::Dummy(s) => s.poll_write(cx, buf),
            #[cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))]
            ConnectionImplProj::Vsock(s) => s.poll_write(cx, buf),
        }
    }

    #[inline]
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        let q: Pin<&mut ConnectionImpl> = self.project().0;
        match q.project() {
            ConnectionImplProj::Tcp(s) => s.poll_flush(cx),
            #[cfg(all(feature = "unix", unix))]
            ConnectionImplProj::Unix(s) => s.poll_flush(cx),
            #[cfg(feature = "inetd")]
            ConnectionImplProj::Stdio(_, s, _) => s.poll_flush(cx),
            #[cfg(feature = "duplex_variant")]
            ConnectionImplProj::Duplex(s) => s.poll_flush(cx),
            #[cfg(feature = "boxed_variant")]
            ConnectionImplProj::Boxed(s) => s.as_mut().poll_flush(cx),
            #[cfg(feature = "dummy_variant")]
            ConnectionImplProj::Dummy(s) => s.poll_flush(cx),
            #[cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))]
            ConnectionImplProj::Vsock(s) => s.poll_flush(cx),
        }
    }

    #[inline]
    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        let q: Pin<&mut ConnectionImpl> = self.project().0;
        match q.project() {
            ConnectionImplProj::Tcp(s) => s.poll_shutdown(cx),
            #[cfg(all(feature = "unix", unix))]
            ConnectionImplProj::Unix(s) => s.poll_shutdown(cx),
            #[cfg(feature = "inetd")]
            ConnectionImplProj::Stdio(_, s, tx) => match s.poll_shutdown(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(ret) => {
                    if let Some(tx) = tx.take() {
                        if tx.send(()).is_err() {
                            warn!("stdout wrapper for inetd mode failed to notify the listener to abort listening loop");
                        } else {
                            debug!("stdout finished in inetd mode. Aborting the listening loop.");
                        }
                    }
                    Poll::Ready(ret)
                }
            },
            #[cfg(feature = "duplex_variant")]
            ConnectionImplProj::Duplex(s) => s.poll_shutdown(cx),
            #[cfg(feature = "boxed_variant")]
            ConnectionImplProj::Boxed(s) => s.as_mut().poll_shutdown(cx),
            #[cfg(feature = "dummy_variant")]
            ConnectionImplProj::Dummy(s) => s.poll_shutdown(cx),
            #[cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))]
            ConnectionImplProj::Vsock(s) => s.poll_shutdown(cx),
        }
    }

    #[inline]
    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<Result<usize, std::io::Error>> {
        let q: Pin<&mut ConnectionImpl> = self.project().0;
        match q.project() {
            ConnectionImplProj::Tcp(s) => s.poll_write_vectored(cx, bufs),
            #[cfg(all(feature = "unix", unix))]
            ConnectionImplProj::Unix(s) => s.poll_write_vectored(cx, bufs),
            #[cfg(feature = "inetd")]
            ConnectionImplProj::Stdio(_, s, _) => s.poll_write_vectored(cx, bufs),
            #[cfg(feature = "duplex_variant")]
            ConnectionImplProj::Duplex(s) => s.poll_write_vectored(cx, bufs),
            #[cfg(feature = "boxed_variant")]
            ConnectionImplProj::Boxed(s) => s.as_mut().poll_write_vectored(cx, bufs),
            #[cfg(feature = "dummy_variant")]
            ConnectionImplProj::Dummy(s) => s.poll_write_vectored(cx, bufs),
            #[cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))]
            ConnectionImplProj::Vsock(s) => s.poll_write_vectored(cx, bufs),
        }
    }

    #[inline]
    fn is_write_vectored(&self) -> bool {
        match &self.0 {
            ConnectionImpl::Tcp(s) => s.is_write_vectored(),
            #[cfg(all(feature = "unix", unix))]
            ConnectionImpl::Unix(s) => s.is_write_vectored(),
            #[cfg(feature = "inetd")]
            ConnectionImpl::Stdio(_, s, _) => s.is_write_vectored(),
            #[cfg(feature = "duplex_variant")]
            ConnectionImpl::Duplex(s) => s.is_write_vectored(),
            #[cfg(feature = "boxed_variant")]
            ConnectionImpl::Boxed(s) => s.as_ref().is_write_vectored(),
            #[cfg(feature = "dummy_variant")]
            ConnectionImpl::Dummy(s) => s.is_write_vectored(),
            #[cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))]
            ConnectionImpl::Vsock(s) => s.is_write_vectored(),
        }
    }
}
