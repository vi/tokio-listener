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

#[cfg(unix)]
use std::os::fd::RawFd;

use futures_core::{Future, Stream};
#[cfg(feature = "inetd")]
use futures_util::{future::Fuse, FutureExt};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::oneshot::{channel, Receiver, Sender},
    time::Sleep,
};
use tracing::{debug, info, trace};

#[cfg(unix)]
use tokio::net::UnixListener;

use crate::{
    connection::ConnectionImpl, Connection, ListenerAddress, SomeSocketAddr, SystemOptions,
    UserOptions,
};

/// Configured TCP. `AF_UNIX` or other stream socket acceptor.
///
/// Based on extended hyper 0.14's `AddrIncoming` code.
pub struct Listener {
    pub(crate) i: ListenerImpl,
    sleep_on_errors: bool,
    timeout: Option<Pin<Box<Sleep>>>,
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.i {
            ListenerImpl::Tcp { .. } => f.write_str("tokio_listener::Listener(tcp)"),
            #[cfg(all(feature = "unix", unix))]
            ListenerImpl::Unix { .. } => f.write_str("tokio_listener::Listener(unix)"),
            #[cfg(feature = "inetd")]
            ListenerImpl::Stdio(_) => f.write_str("tokio_listener::Listener(stdio)"),
        }
    }
}

async fn listen_tcp(
    a: &SocketAddr,
    usr_opts: &UserOptions,
    sys_opts: &SystemOptions,
) -> Result<ListenerImpl, std::io::Error> {
    #[cfg(not(feature = "socket_options"))]
    let s = TcpListener::bind(a).await?;
    #[cfg(feature = "socket_options")]
    let s =
        if usr_opts.tcp_only_v6 || usr_opts.tcp_reuse_port || usr_opts.tcp_listen_backlog.is_some()
        {
            let s = socket2::Socket::new(
                socket2::Domain::for_address(*a),
                socket2::Type::STREAM,
                None,
            )?;
            if usr_opts.tcp_only_v6 {
                s.set_only_v6(true)?;
            }
            #[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos"))))]
            if usr_opts.tcp_reuse_port {
                s.set_reuse_port(true)?;
            }
            s.bind(&socket2::SockAddr::from(*a))?;
            let backlog = usr_opts.tcp_listen_backlog.unwrap_or(1024);
            let Ok(backlog): Result<c_int, _> = backlog.try_into() else {
                return Err(std::io::Error::other("Invalid socket listen backlog value"));
            };
            s.listen(backlog)?;
            s.set_nonblocking(true)?;
            TcpListener::from_std(std::net::TcpListener::from(s))?
        } else {
            TcpListener::bind(a).await?
        };
    Ok(ListenerImpl::Tcp(ListenerImplTcp {
        s,
        nodelay: sys_opts.nodelay,
        #[cfg(feature = "socket_options")]
        keepalive: usr_opts
            .tcp_keepalive
            .as_ref()
            .map(crate::TcpKeepaliveParams::to_socket2),
        #[cfg(feature = "socket_options")]
        recv_buffer_size: usr_opts.recv_buffer_size,
        #[cfg(feature = "socket_options")]
        send_buffer_size: usr_opts.send_buffer_size,
    }))
}

#[cfg(all(unix, feature = "unix"))]
#[allow(clippy::similar_names)]
fn listen_path(usr_opts: &UserOptions, p: &PathBuf) -> Result<ListenerImpl, std::io::Error> {
    #[cfg(feature = "unix_path_tools")]
    #[allow(clippy::collapsible_if)]
    if usr_opts.unix_listen_unlink {
        if std::fs::remove_file(p).is_ok() {
            debug!(file=?p, "removed UNIX socket before listening");
        }
    }
    let i = ListenerImpl::Unix(ListenerImplUnix {
        s: UnixListener::bind(p)?,
        #[cfg(feature = "socket_options")]
        recv_buffer_size: usr_opts.recv_buffer_size,
        #[cfg(feature = "socket_options")]
        send_buffer_size: usr_opts.send_buffer_size,
    });
    #[cfg(feature = "unix_path_tools")]
    {
        use crate::UnixChmodVariant;
        use std::os::unix::fs::PermissionsExt;
        if let Some(chmod) = usr_opts.unix_listen_chmod {
            let mode = match chmod {
                UnixChmodVariant::Owner => 0o006,
                UnixChmodVariant::Group => 0o066,
                UnixChmodVariant::Everybody => 0o666,
            };
            let perms = std::fs::Permissions::from_mode(mode);
            std::fs::set_permissions(p, perms)?;
        }
        if (usr_opts.unix_listen_uid, usr_opts.unix_listen_gid) != (None, None) {
            let uid = usr_opts.unix_listen_uid.map(Into::into);
            let gid = usr_opts.unix_listen_gid.map(Into::into);
            nix::unistd::chown(p, uid, gid)?;
        }
    }
    Ok(i)
}

#[cfg(all(feature = "unix", any(target_os = "linux", target_os = "android")))]
fn listen_abstract(a: &String, usr_opts: &UserOptions) -> Result<ListenerImpl, std::io::Error> {
    #[cfg(target_os = "android")]
    use std::os::android::net::SocketAddrExt;
    #[cfg(target_os = "linux")]
    use std::os::linux::net::SocketAddrExt;
    let a = std::os::unix::net::SocketAddr::from_abstract_name(a)?;
    let s = std::os::unix::net::UnixListener::bind_addr(&a)?;
    s.set_nonblocking(true)?;
    Ok(ListenerImpl::Unix(ListenerImplUnix {
        s: UnixListener::from_std(s)?,
        #[cfg(feature = "socket_options")]
        recv_buffer_size: usr_opts.recv_buffer_size,
        #[cfg(feature = "socket_options")]
        send_buffer_size: usr_opts.send_buffer_size,
    }))
}

#[cfg(all(feature = "sd_listen", unix))]
fn listen_from_fd(
    usr_opts: &UserOptions,
    fdnum: i32,
    sys_opts: &SystemOptions,
) -> Result<ListenerImpl, std::io::Error> {
    use std::os::fd::FromRawFd;

    use tracing::error;

    use std::os::fd::IntoRawFd;

    use crate::listener_address::check_env_for_fd;
    if !usr_opts.sd_accept_ignore_environment && check_env_for_fd(fdnum).is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Failed LISTEN_PID or LISTEN_FDS environment check for sd-listen mode",
        ));
    }
    let fd: RawFd = (fdnum).into();

    let s = unsafe { socket2::Socket::from_raw_fd(fd) };
    let sa = s.local_addr().map_err(|e| {
        error!("Failed to determine socket domain of file descriptor {fd}: {e}");
        e
    })?;
    let unix = sa.domain() == socket2::Domain::UNIX;
    let fd = s.into_raw_fd();

    if unix {
        #[cfg(not(feature = "unix"))] {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Socket inherited using sd-listen points to a UNIX-domain socket, but this feature was not selected at compile time in tokio-listener",
            ));
        }
        #[cfg(feature = "unix")] {
            let s = unsafe { std::os::unix::net::UnixListener::from_raw_fd(fd) };
            s.set_nonblocking(true)?;
            Ok(ListenerImpl::Unix(ListenerImplUnix {
                s: UnixListener::from_std(s)?,
                #[cfg(feature = "socket_options")]
                send_buffer_size: usr_opts.send_buffer_size,
        
                #[cfg(feature = "socket_options")]
                recv_buffer_size: usr_opts.recv_buffer_size,
            }))
        }
    } else {
        let s = unsafe { std::net::TcpListener::from_raw_fd(fd) };
        s.set_nonblocking(true)?;
        Ok(ListenerImpl::Tcp(ListenerImplTcp {
            s: TcpListener::from_std(s)?,
            nodelay: sys_opts.nodelay,
            #[cfg(feature = "socket_options")]
            keepalive: usr_opts
                .tcp_keepalive
                .as_ref()
                .map(crate::TcpKeepaliveParams::to_socket2),
            #[cfg(feature = "socket_options")]
            recv_buffer_size: usr_opts.recv_buffer_size,
            #[cfg(feature = "socket_options")]
            send_buffer_size: usr_opts.send_buffer_size,
        }))
    }
}


#[cfg(all(feature = "sd_listen", unix))]
fn listen_from_fd_named(
    usr_opts: &UserOptions,
    fdname: &str,
    sys_opts: &SystemOptions,
) -> Result<ListenerImpl, std::io::Error> {

    let listen_fdnames = match std::env::var("LISTEN_FDNAMES") {
        Ok(x) => x,
        Err(e) => match e {
            std::env::VarError::NotPresent => return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "tokio-listener requested to use named inherited file descriptor, but no LISTEN_FDNAMES environment variable present",
            )),
            std::env::VarError::NotUnicode(_) => return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "tokio-listener requested to use named inherited file descriptor, but LISTEN_FDNAMES environment variable contains non-unicode content",
            )),
        }
    };

    let mut fd : RawFd = crate::listener_address::SD_LISTEN_FDS_START as RawFd;
    for name in listen_fdnames.split(':') {
        debug!("Considering LISTEN_FDNAMES chunk {name}");
        if name == fdname {
            return listen_from_fd(usr_opts, fd, sys_opts);
        }
        fd += 1;
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::Other,
        format!("tokio-listener requested to use named inherited file descriptor {fdname}, but LISTEN_FDNAMES environment variable does not contain that chunk."),
    ))
}

impl Listener {
    #[allow(clippy::missing_errors_doc)]
    /// Creates listener corresponding specified to tokio-listener address and options.
    ///
    /// * For TCP addresses it tries to behave close to hyper 0.14's listener
    /// * For UNIX path addresses, it can unlink or change permissions of the socket based on user options
    /// * For raw fd sockets, it checkes `LISTEN_FD` and `LISTEN_PID` environment variables by default, unless opted out in user options
    /// * For inetd it accepts only one connection. However, reporting of the error of
    /// inability to accept the second connection is delayed until the first connection finishes, to avoid premature exit from process.
    ///
    /// With `hyper014` crate feature (default), the listener can be directly used as argument for `Server::builder`.
    ///
    /// Binding may fail due to unsupported address type, e.g. if trying to use UNIX addresses on Windows or abstract-namespaces sockets on Mac.
    pub async fn bind(
        addr: &ListenerAddress,
        sys_opts: &SystemOptions,
        usr_opts: &UserOptions,
    ) -> std::io::Result<Self> {
        let i: ListenerImpl = match addr {
            ListenerAddress::Tcp(a) => listen_tcp(a, usr_opts, sys_opts).await?,
            #[cfg(all(unix, feature = "unix"))]
            ListenerAddress::Path(p) => listen_path(usr_opts, p)?,
            #[cfg(all(feature = "unix", any(target_os = "linux", target_os = "android")))]
            ListenerAddress::Abstract(a) => listen_abstract(a, usr_opts)?,
            #[cfg(feature = "inetd")]
            ListenerAddress::Inetd => {
                let (tx, rx) = channel();
                ListenerImpl::Stdio(StdioListener {
                    rx: rx.fuse(),
                    token: Some(tx),
                })
            }
            #[cfg(all(feature = "sd_listen", unix))]
            ListenerAddress::FromFd(fdnum) => listen_from_fd(usr_opts, *fdnum, sys_opts)?,
            #[cfg(all(feature = "sd_listen", unix))]
            ListenerAddress::FromFdNamed(fdname) => listen_from_fd_named(usr_opts, fdname, sys_opts)?,
            #[allow(unreachable_patterns)]
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "This tokio-listener mode is not supported on this platform or is disabled during compilation",
                ));
            }
        };
        Ok(Listener {
            i,
            sleep_on_errors: sys_opts.sleep_on_errors,
            timeout: None,
        })
    }
}

#[cfg(feature = "inetd")]
pub(crate) struct StdioListener {
    rx: Fuse<Receiver<()>>,
    token: Option<Sender<()>>,
}

#[cfg(feature = "inetd")]
impl StdioListener {
    fn poll_accept(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<(Connection, SomeSocketAddr)>> {
        match self.token.take() {
            Some(tx) => {
                debug!(r#type = "stdio", "incoming connection");
                Poll::Ready(Ok((
                    Connection(ConnectionImpl::Stdio(
                        tokio::io::stdin(),
                        tokio::io::stdout(),
                        Some(tx),
                    )),
                    SomeSocketAddr::Stdio,
                )))
            }
            None => match Pin::new(&mut self.rx).poll(cx) {
                Poll::Ready(..) => {
                    trace!("finished waiting for liberation of stdout to stop listening loop");
                    Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "stdin/stdout pseudosocket is already used",
                    )))
                }
                Poll::Pending => Poll::Pending,
            },
        }
    }
}

#[allow(clippy::missing_errors_doc)]
#[allow(missing_docs)]
impl Listener {
    pub fn try_borrow_tcp_listener(&self) -> Option<&TcpListener> {
        if let ListenerImpl::Tcp(ListenerImplTcp { ref s, .. }) = self.i {
            Some(s)
        } else {
            None
        }
    }
    #[cfg(all(feature = "unix", unix))]
    #[cfg_attr(docsrs_alt, doc(cfg(all(feature = "unix", unix))))]
    pub fn try_borrow_unix_listener(&self) -> Option<&UnixListener> {
        if let ListenerImpl::Unix(ListenerImplUnix { s: ref x, .. }) = self.i {
            Some(x)
        } else {
            None
        }
    }

    pub fn try_into_tcp_listener(self) -> Result<TcpListener, Self> {
        if let ListenerImpl::Tcp(ListenerImplTcp { s, .. }) = self.i {
            Ok(s)
        } else {
            Err(self)
        }
    }
    #[cfg(all(feature = "unix", unix))]
    #[cfg_attr(docsrs_alt, doc(cfg(all(feature = "unix", unix))))]
    pub fn try_into_unix_listener(self) -> Result<UnixListener, Self> {
        if let ListenerImpl::Unix(ListenerImplUnix { s, .. }) = self.i {
            Ok(s)
        } else {
            Err(self)
        }
    }

    /// This listener is in inetd (stdin/stdout) more and the sole connection is already accepted
    #[allow(unreachable_code)]
    pub fn no_more_connections(&self) -> bool {
        #[cfg(feature = "inetd")]
        return if let ListenerImpl::Stdio(ref x) = self.i {
            x.token.is_none()
        } else {
            false
        };
        false
    }

    /// See main [`Listener::bind`] documentation for specifics of how it accepts connections
    pub fn poll_accept(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<(Connection, SomeSocketAddr)>> {
        loop {
            if let Some(ref mut to) = self.timeout {
                ready!(Pin::new(to).poll(cx));
            }
            self.timeout = None;

            let ret = match &mut self.i {
                ListenerImpl::Tcp(ti) => ti.poll_accept(cx),
                #[cfg(all(feature = "unix", unix))]
                ListenerImpl::Unix(ui) => ui.poll_accept(cx),
                #[cfg(feature = "inetd")]
                ListenerImpl::Stdio(x) => return x.poll_accept(cx),
            };
            let e: std::io::Error = match ret {
                Poll::Ready(Err(e)) => e,
                Poll::Ready(Ok(x)) => return Poll::Ready(Ok(x)),
                Poll::Pending => return Poll::Pending,
            };
            if is_connection_error(&e) {
                info!(action = "retry", "failed_accept");
                continue;
            }
            if self.sleep_on_errors {
                info!(action = "sleep_retry", "failed_accept");
                self.timeout = Some(Box::pin(tokio::time::sleep(Duration::from_secs(1))));
            } else {
                info!(action = "error", "failed_accept");
                return Poll::Ready(Err(e));
            }
        }
    }

    /// See main [`Listener::bind`] documentation for specifics of how it accepts connections
    pub async fn accept(&mut self) -> std::io::Result<(Connection, SomeSocketAddr)> {
        std::future::poll_fn(|cx| self.poll_accept(cx)).await
    }
}

impl Stream for Listener {
    type Item = std::io::Result<Connection>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.poll_accept(cx) {
            Poll::Ready(Ok((connection, _))) => Poll::Ready(Some(Ok(connection))),
            Poll::Ready(Err(err)) => Poll::Ready(Some(Err(err))),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// This function defines errors that are per-connection. Which basically
/// means that if we get this error from `accept()` system call it means
/// next connection might be ready to be accepted.
///
/// All other errors will incur a timeout before next `accept()` is performed.
/// The timeout is useful to handle resource exhaustion errors like ENFILE
/// and EMFILE. Otherwise, could enter into tight loop.
///
/// Based on <https://docs.rs/hyper/latest/src/hyper/server/tcp.rs.html#109-116>
pub(crate) fn is_connection_error(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
    )
}

pub(crate) struct ListenerImplTcp {
    pub(crate) s: TcpListener,
    nodelay: bool,
    #[cfg(feature = "socket_options")]
    keepalive: Option<socket2::TcpKeepalive>,
    #[cfg(feature = "socket_options")]
    recv_buffer_size: Option<usize>,
    #[cfg(feature = "socket_options")]
    send_buffer_size: Option<usize>,
}

#[cfg(all(feature = "unix", unix))]
pub(crate) struct ListenerImplUnix {
    pub(crate) s: UnixListener,
    #[cfg(feature = "socket_options")]
    recv_buffer_size: Option<usize>,
    #[cfg(feature = "socket_options")]
    send_buffer_size: Option<usize>,
}

pub(crate) enum ListenerImpl {
    Tcp(ListenerImplTcp),
    #[cfg(all(feature = "unix", unix))]
    Unix(ListenerImplUnix),
    #[cfg(feature = "inetd")]
    Stdio(StdioListener),
}

impl ListenerImplTcp {
    fn poll_accept(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<(Connection, SomeSocketAddr)>> {
        let ListenerImplTcp {
            s,
            nodelay,
            #[cfg(feature = "socket_options")]
            keepalive,
            #[cfg(feature = "socket_options")]
            recv_buffer_size,
            #[cfg(feature = "socket_options")]
            send_buffer_size,
        } = self;
        match s.poll_accept(cx) {
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Ready(Ok((c, a))) => {
                debug!(fromaddr=%a, r#type="tcp", "incoming connection");
                if *nodelay {
                    c.set_nodelay(true)?;
                }

                #[cfg(feature = "socket_options")]
                {
                    apply_tcp_keepalive_opts(&c, keepalive)?;
                    apply_socket_buf_opts(&c, recv_buffer_size, send_buffer_size)?;
                }

                Poll::Ready(Ok((
                    Connection(ConnectionImpl::Tcp(c)),
                    SomeSocketAddr::Tcp(a),
                )))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(all(feature = "unix", unix))]
impl ListenerImplUnix {
    fn poll_accept(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<(Connection, SomeSocketAddr)>> {
        let ListenerImplUnix {
            s,
            #[cfg(feature = "socket_options")]
            recv_buffer_size,
            #[cfg(feature = "socket_options")]
            send_buffer_size,
        } = self;
        match s.poll_accept(cx) {
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Ready(Ok((c, a))) => {
                debug!(r#type = "unix", "incoming connection");
                #[cfg(feature = "socket_options")]
                {
                    apply_socket_buf_opts(&c, recv_buffer_size, send_buffer_size)?;
                }
                Poll::Ready(Ok((
                    Connection(ConnectionImpl::Unix(c)),
                    SomeSocketAddr::Unix(a),
                )))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(feature = "socket_options")]
fn apply_tcp_keepalive_opts(
    c: &TcpStream,
    keepalive: &Option<socket2::TcpKeepalive>,
) -> std::io::Result<()> {
    let sock_ref = socket2::SockRef::from(&c);
    if let Some(ka) = keepalive {
        sock_ref.set_tcp_keepalive(ka)?;
    }
    Ok(())
}

#[cfg(all(feature = "socket_options", unix))]
fn apply_socket_buf_opts<T: std::os::fd::AsFd>(
    c: &T,
    recv_buffer_size: &Option<usize>,
    send_buffer_size: &Option<usize>,
) -> std::io::Result<()> {
    let sock_ref = socket2::SockRef::from(&c);
    if let Some(n) = recv_buffer_size {
        sock_ref.set_recv_buffer_size(*n)?;
    }
    if let Some(n) = send_buffer_size {
        sock_ref.set_send_buffer_size(*n)?;
    }
    Ok(())
}

#[cfg(all(feature = "socket_options", not(unix)))]
fn apply_socket_buf_opts(
    c: &TcpStream,
    recv_buffer_size: &Option<usize>,
    send_buffer_size: &Option<usize>,
) -> std::io::Result<()> {
    let sock_ref = socket2::SockRef::from(&c);
    if let Some(n) = recv_buffer_size {
        sock_ref.set_recv_buffer_size(*n)?;
    }
    if let Some(n) = send_buffer_size {
        sock_ref.set_send_buffer_size(*n)?;
    }
    Ok(())
}
