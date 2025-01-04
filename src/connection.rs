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
    io::{AsyncRead, AsyncWrite, Stdin, Stdout},
    net::TcpStream,
    sync::oneshot::Sender,
};
use tracing::{debug, warn};

#[cfg(unix)]
use tokio::net::UnixStream;

/// Socket-like things supported by tokio_listner.
/// 
/// With `extra_connection_variants` crate feature `Box<dyn AsyncReadWrite + Send>` can be used as a [`Connection`] variant.
pub trait AsyncReadWrite : AsyncRead + AsyncWrite + std::fmt::Debug {}
impl<T: AsyncRead + AsyncWrite + std::fmt::Debug> AsyncReadWrite for T {}

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
            #[cfg(feature = "inetd")]
            ConnectionImpl::Stdio(_, _, _) => f.write_str("Connection(stdio)"),
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImpl::Duplex(_)  => f.write_str("Connection(DuplexStream)"),
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImpl::Boxed(ref b)  => f.debug_struct("Connection").field("0", b).finish(),
            #[cfg(feature = "extra_connection_variants")]
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
    #[cfg(feature = "extra_connection_variants")]
    Duplex(#[pin] tokio::io::DuplexStream),
    #[cfg(feature = "extra_connection_variants")]
    Dummy(#[pin] tokio::io::Empty),
    #[cfg(feature = "extra_connection_variants")]
    Boxed(Pin<Box<dyn AsyncReadWrite + Send>>),
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
    pub fn try_into_stdio(self) -> Result<(Stdin, Stdout, Option<Sender<()>>), Self> {
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
    pub fn try_borrow_stdio(&self) -> Option<(&Stdin, &Stdout)> {
        if let ConnectionImpl::Stdio(ref i, ref o, ..) = self.0 {
            Some((i, o))
        } else {
            None
        }
    }

    #[cfg(feature = "extra_connection_variants")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "extra_connection_variants")))]
    pub fn try_into_duplex(self) -> Result<tokio::io::DuplexStream, Self> {
        if let ConnectionImpl::Duplex(s) = self.0 {
            Ok(s)
        } else {
            Err(self)
        }
    }
    #[cfg(feature = "extra_connection_variants")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "extra_connection_variants")))]
    pub fn try_borrow_duplex(&self) -> Option<&tokio::io::DuplexStream> {
        if let ConnectionImpl::Duplex(ref s) = self.0 {
            Some(s)
        } else {
            None
        }
    }

    #[cfg(feature = "extra_connection_variants")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "extra_connection_variants")))]
    pub fn is_dummy(&self) -> bool {
        if let ConnectionImpl::Dummy(ref _s) = self.0 {
            true
        } else {
            false
        }
    }

    #[cfg(feature = "extra_connection_variants")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "extra_connection_variants")))]
    pub fn try_into_boxed(self) -> Result<Pin<Box<dyn AsyncReadWrite + Send>>, Self> {
        if let ConnectionImpl::Boxed(s) = self.0 {
            Ok(s)
        } else {
            Err(self)
        }
    }
    #[cfg(feature = "extra_connection_variants")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "extra_connection_variants")))]
    pub fn try_borrow_boxed(&self) -> Option<&Pin<Box<dyn AsyncReadWrite + Send>>> {
        if let ConnectionImpl::Boxed(ref s) = self.0 {
            Some(s)
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
impl From<(Stdin, Stdout, Option<Sender<()>>)> for Connection {
    fn from(s: (Stdin, Stdout, Option<Sender<()>>)) -> Self {
        Connection(ConnectionImpl::Stdio(s.0, s.1, s.2))
    }
}

#[cfg(feature = "extra_connection_variants")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "extra_connection_variants")))]
impl From<tokio::io::DuplexStream> for Connection {
    fn from(s: tokio::io::DuplexStream) -> Self {
        Connection(ConnectionImpl::Duplex(s))
    }
}
#[cfg(feature = "extra_connection_variants")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "extra_connection_variants")))]
impl From<tokio::io::Empty> for Connection {
    fn from(s: tokio::io::Empty) -> Self {
        Connection(ConnectionImpl::Dummy(s))
    }
}
#[cfg(feature = "extra_connection_variants")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "extra_connection_variants")))]
impl From<Pin<Box<dyn AsyncReadWrite + Send>>> for Connection {
    fn from(s: Pin<Box<dyn AsyncReadWrite + Send>>) -> Self {
        Connection(ConnectionImpl::Boxed(s))
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
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImplProj::Duplex(s) => s.poll_read(cx, buf),
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImplProj::Boxed(s) => s.as_mut().poll_read(cx, buf),
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImplProj::Dummy(s) => s.poll_read(cx, buf),
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
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImplProj::Duplex(s) => s.poll_write(cx, buf),
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImplProj::Boxed(s) => s.as_mut().poll_write(cx, buf),
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImplProj::Dummy(s) => s.poll_write(cx, buf),
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
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImplProj::Duplex(s) => s.poll_flush(cx),
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImplProj::Boxed(s) => s.as_mut().poll_flush(cx),
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImplProj::Dummy(s) => s.poll_flush(cx),
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
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImplProj::Duplex(s) => s.poll_shutdown(cx),
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImplProj::Boxed(s) => s.as_mut().poll_shutdown(cx),
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImplProj::Dummy(s) => s.poll_shutdown(cx),
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
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImplProj::Duplex(s) => s.poll_write_vectored(cx, bufs),
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImplProj::Boxed(s) => s.as_mut().poll_write_vectored(cx, bufs),
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImplProj::Dummy(s) => s.poll_write_vectored(cx, bufs),
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
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImpl::Duplex(s) => s.is_write_vectored(),
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImpl::Boxed(s) => s.as_ref().is_write_vectored(),
            #[cfg(feature = "extra_connection_variants")]
            ConnectionImpl::Dummy(s) => s.is_write_vectored(),
        }
    }
}
