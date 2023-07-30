#![warn(missing_docs)]
#![doc = include_str!("../README.md")]

use std::{
    fmt::Display,
    net::SocketAddr,
    path::PathBuf,
    pin::Pin,
    str::FromStr,
    task::{ready, Context, Poll},
    time::Duration,
};


#[cfg(unix)]
use std::os::fd::RawFd;

use futures_core::Future;
use pin_project::pin_project;
use tokio::{
    io::{AsyncRead, AsyncWrite, Stdin, Stdout},
    net::{TcpListener, TcpStream},
    sync::oneshot::{channel, Receiver, Sender},
    time::Sleep,
};
use tracing::{debug, error, info, trace, warn};

#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

/// Value of `--unix-listen-chmod` option which allows changing DAC file access mode for UNIX path socket
#[non_exhaustive]
#[cfg_attr(feature="serde", derive(serde_with::DeserializeFromStr, serde_with::SerializeDisplay))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnixChmodVariant {
    /// Set filesystem mode of the UNIX socket to `u+rw`, allowing access only to one uid
    Owner,
    /// Set filesystem mode of the UNIX socket to `ug+rw`, allowing access to owner uid and a group
    Group,
    /// Set filesystem mode of the UNIX socket to `a+rw`, allowing global access to the socket
    Everybody,
}

impl Display for UnixChmodVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnixChmodVariant::Owner => "owner".fmt(f),
            UnixChmodVariant::Group => "group".fmt(f),
            UnixChmodVariant::Everybody => "everybody".fmt(f),
        }
    }
}

impl FromStr for UnixChmodVariant {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("owner") {
            Ok(UnixChmodVariant::Owner)
        } else if s.eq_ignore_ascii_case("group") {
            Ok(UnixChmodVariant::Group)
        } else if s.eq_ignore_ascii_case("everybody") {
            Ok(UnixChmodVariant::Everybody)
        } else {
            Err("Unknown chmod variant. Expected `owner`, `group` or `everybody`.")
        }
    }
}

#[cfg_attr(feature="clap", derive(clap::Args))]
#[cfg_attr(feature="serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Debug, Default)]
#[non_exhaustive]
/// User options that supplement listening address.
/// 
/// With `clap` crate feature, this struct can be `clap(flatten)`-ed directly into your primary command line parameters.
/// With `serde` crate feature, it supportes serialisation and deserialisation.
/// 
/// Create instances with `Default::default()` and modify available fields.
/// 
/// Non-relevant options are ignored by [`Listener::bind`].
/// 
/// All options are always available regardless of current platform.
pub struct UserOptions {
    /// remove UNIX socket prior to binding to it
    #[cfg_attr(feature="clap", clap(long))]
    #[cfg_attr(feature="serde", serde(default))]
    pub unix_listen_unlink: bool,

    /// change filesystem mode of the newly bound UNIX socket to `owner`, `group` or `everybody`
    #[cfg_attr(feature="clap", clap(long))]
    #[cfg_attr(feature="serde", serde(default))]
    pub unix_listen_chmod: Option<UnixChmodVariant>,

    /// change owner user of the newly bound UNIX socket to this numeric uid
    #[cfg_attr(feature="clap", clap(long))]
    #[cfg_attr(feature="serde", serde(default))]
    pub unix_listen_uid: Option<u32>,

    /// change owner group of the newly bound UNIX socket to this numeric uid
    #[cfg_attr(feature="clap", clap(long))]
    #[cfg_attr(feature="serde", serde(default))]
    pub unix_listen_gid: Option<u32>,

    /// ignore environment variables like LISTEN_PID or LISTEN_FDS and unconditionally use
    /// file descritor `3` as a socket in sd-listen or sd-listen-unix modes
    #[cfg_attr(feature="clap", clap(long))]
    #[cfg_attr(feature="serde", serde(default))]
    pub sd_accept_ignore_environment: bool,

    /// set SO_KEEPALIVE settings for each accepted TCP connection.
    /// Note that this version of tokio-listener does not support setting this from config or CLI,
    /// you need to set it programatically.
    #[cfg_attr(feature="clap", clap(skip))]
    #[cfg_attr(feature="serde", serde(skip))]
    pub tcp_keepalive: Option<socket2::TcpKeepalive>,
}

/// Abstraction over socket address that instructs in which way and at what address (if any) [`Listener`]
/// should listen for incoming stream connections.
/// 
/// All address variants are available on all platforms, regardness of actual support in the Listener.
/// 
/// If serde is enabled, it is serialized/deserialized the same as string, same as as in the CLI, using `FromStr`/`Display`.
/// 
/// See variatns documentation for FromStr string patterns that are accepted by ListenerAddress parser
/// 
/// Remember to copy or link those documentation snippets into your app's documentation.
#[non_exhaustive]
#[cfg_attr(feature="serde", derive(serde_with::DeserializeFromStr, serde_with::SerializeDisplay))]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ListenerAddress {
    /// Usual server TCP socket. triggered by specifying IPv4 or IPv6 address and port pair.  
    /// Example: `127.0.0.1:8080`.
    Tcp(SocketAddr),
    /// Path-based UNIX socket. Path must begin with `/` or `.`.  
    /// Examples: `/tmp/mysock`, `./mysock`
    Path(PathBuf),
    /// Linux abstract-namespaced UNIX socket. Indicated by using `@` as a first character.
    /// Example: `@server`
    Abstract(String),
    /// "inetd" or "Accept=yes" mode where stdin and stdout (file descriptors 0 and 1) are using together as a socket
    /// and only one connections is served. Triggered by using `-` as the address.
    Inetd,
    /// "Accept=no" mode - using manually specified file descriptor as a pre-created server socket reeady to accept TCP connections.
    /// Triggered by specifying `sd-listen` as address, which sets `3` as file descriptor number
    FromFdTcp(i32),
    /// "Accept=no" mode - using fmanually specified ile descriptor as a pre-created server socket reeady to accept Unix connections.
    /// Triggered by specifying `sd-listen-unix` as address, which sets `3` as file descriptor number
    FromFdUnix(i32),
}

const SD_LISTEN_FDS_START: u32 = 3;

impl FromStr for ListenerAddress {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with("/") || s.starts_with("./") {
            Ok(ListenerAddress::Path(s.into()))
        } else if s.starts_with('@') {
            Ok(ListenerAddress::Abstract(s[1..].to_owned()))
        } else if s == "-" {
            Ok(ListenerAddress::Inetd)
        } else if s.eq_ignore_ascii_case("sd-listen") || s.eq_ignore_ascii_case("sd_listen") {
            Ok(ListenerAddress::FromFdTcp(SD_LISTEN_FDS_START as i32))
        } else if s.eq_ignore_ascii_case("sd-listen-unix")
            || s.eq_ignore_ascii_case("sd_listen_unix")
        {
            Ok(ListenerAddress::FromFdUnix(SD_LISTEN_FDS_START as i32))
        } else if let Ok(a) = s.parse() {
            Ok(ListenerAddress::Tcp(a))
        } else {
            Err("Invalid tokio-listener address type")
        }
    }
}

impl Display for ListenerAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ListenerAddress::Tcp(a) => a.fmt(f),
            ListenerAddress::Path(p) => {
                if let Some(s) = p.to_str() {
                    if p.is_absolute() {
                        s.fmt(f)
                    } else {
                        if s.starts_with("./") {
                            s.fmt(f)
                        } else {
                            write!(f, "./{s}")
                        }
                    }
                } else {
                    if p.is_absolute() {
                        "/???".fmt(f)
                    } else {
                        "./???".fmt(f)
                    }
                }
            }
            ListenerAddress::Abstract(p) => {
                write!(f, "@{p}")
            }
            ListenerAddress::Inetd => "-".fmt(f),
            ListenerAddress::FromFdTcp(fd) => {
                if *fd == SD_LISTEN_FDS_START as i32 {
                    "sd-listen".fmt(f)
                } else {
                    write!(f, "accept-tcp-from-fd:{}", fd)
                }
            }
            ListenerAddress::FromFdUnix(fd) => {
                if *fd == SD_LISTEN_FDS_START as i32 {
                    "sd-listen-unix".fmt(f)
                } else {
                    write!(f, "accept-unix-from-fd:{}", fd)
                }
            }
        }
    }
}
/// Listener options that are supposed to be hard coded in the code
/// (not configurable by user)
#[non_exhaustive]
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemOptions {
    /// Wait for one second and retry if accepting connections fail (for reasons unrelated to the connections themselves),
    /// assuming it is file descriptor number exhaustion, which may be temporary
    pub sleep_on_errors: bool,
    /// Set TCP_NODELAY on accepted TCP sockets. Does not affect other socket types.
    pub nodelay: bool,
}

/// Configured TCP. AF_UNIX or other stream socket acceptor.
/// 
/// Based on extended hyper 0.14's `AddrIncoming` code.
pub struct Listener {
    i: ListenerImpl,
    sleep_on_errors: bool,
    timeout: Option<Pin<Box<Sleep>>>,
}

// based on https://docs.rs/sd-notify/0.4.1/src/sd_notify/lib.rs.html#164, but simplified
#[allow(unused)]
fn check_env_for_fd(fdnum: i32) -> Option<()> {
    let listen_pid = std::env::var("LISTEN_PID").ok()?;
    let listen_pid: u32 = listen_pid.parse().ok()?;

    let listen_fds = std::env::var("LISTEN_FDS").ok()?;
    let listen_fds: u32 = listen_fds.parse().ok()?;

    debug!("Parsed LISTEN_PID and LISTEN_FDS");

    if listen_pid != std::process::id() {
        error!(expected = %std::process::id(), actual=listen_pid, "Failed LISTEN_PID check");
        return None;
    }

    if fdnum < SD_LISTEN_FDS_START as i32 || fdnum >= (SD_LISTEN_FDS_START + listen_fds) as i32 {
        error!(fdnum, listen_fds, "Failed LISTEN_FDS check");
        return None;
    }

    Some(())
}

impl Listener {
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
        sopts: &SystemOptions,
        uopts: &UserOptions,
    ) -> std::io::Result<Self> {
        let i: ListenerImpl = match addr {
            ListenerAddress::Tcp(a) => ListenerImpl::Tcp {
                s: TcpListener::bind(a).await?,
                nodelay: sopts.nodelay,
                keepalive: uopts.tcp_keepalive.clone(),
            },
            #[cfg(unix)]
            ListenerAddress::Path(p) => {
                if uopts.unix_listen_unlink {
                    if std::fs::remove_file(&p).is_ok() {
                        debug!(file=?p, "removed UNIX socket before listening")
                    }
                }
                let i = ListenerImpl::Unix(UnixListener::bind(&p)?);
                if let Some(chmod) = uopts.unix_listen_chmod {
                    let mode = match chmod {
                        UnixChmodVariant::Owner => 0o006,
                        UnixChmodVariant::Group => 0o066,
                        UnixChmodVariant::Everybody => 0o666,
                    };
                    use std::os::unix::fs::PermissionsExt;
                    let perms = std::fs::Permissions::from_mode(mode);
                    std::fs::set_permissions(&p, perms)?;
                }
                if (uopts.unix_listen_uid, uopts.unix_listen_gid) != (None, None) {
                    let uid = uopts.unix_listen_uid.map(Into::into);
                    let gid = uopts.unix_listen_gid.map(Into::into);
                    nix::unistd::chown(p, uid, gid)?;
                }
                i
            }
            #[cfg(any(target_os = "linux", target_os = "android"))]
            ListenerAddress::Abstract(a) => {
                use std::os::linux::net::SocketAddrExt;
                let a = std::os::unix::net::SocketAddr::from_abstract_name(a)?;
                let s = std::os::unix::net::UnixListener::bind_addr(&a)?;
                s.set_nonblocking(true)?;
                ListenerImpl::Unix(UnixListener::from_std(s)?)
            }
            ListenerAddress::Inetd => {
                let (tx, rx) = channel();
                ListenerImpl::Stdio(StdioListener {
                    rx,
                    token: Some(tx),
                })
            }
            #[cfg(unix)]
            ListenerAddress::FromFdTcp(fdnum) => {
                if !uopts.sd_accept_ignore_environment {
                    if check_env_for_fd(*fdnum).is_none() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            "Failed LISTEN_PID or LISTEN_FDS environment check for sd-listen mode",
                        ));
                    }
                }
                let fd: RawFd = (*fdnum).into();
                use std::os::fd::FromRawFd;

                // Safety: we assume end user is reasonable and won't specify too tricky file descriptor numbers.
                // Besides checking systemd's environment variables, we can't do much to prevent misuse anyway.
                let s = unsafe { std::net::TcpListener::from_raw_fd(fd) };
                s.set_nonblocking(true)?;
                ListenerImpl::Tcp {
                    s: TcpListener::from_std(s)?,
                    nodelay: sopts.nodelay,
                    keepalive: uopts.tcp_keepalive.clone(),
                }
            }
            #[cfg(unix)]
            ListenerAddress::FromFdUnix(fdnum) => {
                if !uopts.sd_accept_ignore_environment {
                    if check_env_for_fd(*fdnum).is_none() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            "Failed LISTEN_PID or LISTEN_FDS environment check for sd-listen mode",
                        ));
                    }
                }
                let fd: RawFd = (*fdnum).into();
                use std::os::fd::FromRawFd;
                // Safety: we assume end user is reasonable and won't specify too tricky file descriptor numbers.
                // Besides checking systemd's environment variables, we can't do much to prevent misuse anyway.
                let s = unsafe { std::os::unix::net::UnixListener::from_raw_fd(fd) };
                s.set_nonblocking(true)?;
                ListenerImpl::Unix(UnixListener::from_std(s)?)
            }
            #[cfg(not(unix))]
            ListenerAddress::Path(..) | ListenerAddress::FromFdUnix(..) | ListenerAddress::FromFdTcp(..) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "This tokio-listener mode is not supported on this platform",
                ));
            }
            #[cfg(not(any(target_os = "linux", target_os = "android")))]
            ListenerAddress::Abstract(..) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "This tokio-listener mode is not supported on this platform",
                )); 
            }
        };
        Ok(Listener {
            i,
            sleep_on_errors: sopts.sleep_on_errors,
            timeout: None,
        })
    }
}

struct StdioListener {
    rx: Receiver<()>,
    token: Option<Sender<()>>,
}

enum ListenerImpl {
    Tcp {
        s: TcpListener,
        nodelay: bool,
        keepalive: Option<socket2::TcpKeepalive>,
    },
    #[cfg(unix)]
    Unix(UnixListener),
    Stdio(StdioListener),
}

impl Listener {
    #[allow(missing_docs)]
    pub fn try_borrow_tcp_listener(&self) -> Option<&TcpListener> {
        if let ListenerImpl::Tcp { ref s, .. } = self.i {
            Some(s)
        } else {
            None
        }
    }
    #[allow(missing_docs)]
    #[cfg(unix)]
    pub fn try_borrow_unix_listener(&self) -> Option<&UnixListener> {
        if let ListenerImpl::Unix(ref x) = self.i {
            Some(x)
        } else {
            None
        }
    }

    #[allow(missing_docs)]
    pub fn try_into_tcp_listener(self) -> Result<TcpListener, Self> {
        if let ListenerImpl::Tcp { s, .. } = self.i {
            Ok(s)
        } else {
            Err(self)
        }
    }
    #[allow(missing_docs)]
    #[cfg(unix)]
    pub fn try_into_unix_listener(self) -> Result<UnixListener, Self> {
        if let ListenerImpl::Unix(x) = self.i {
            Ok(x)
        } else {
            Err(self)
        }
    }

    /// This listener is in inetd (stdin/stdout) more and the sole connection is already accepted
    pub fn no_more_connections(&self) -> bool {
        if let ListenerImpl::Stdio(ref x) = self.i {
            x.token.is_none()
        } else {
            false
        }
    }

    /// See main [`Listener::bind`] documentation for specifics of how it accepts conenctions
    pub fn poll_accept(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<(Connection, SomeSocketAddr)>> {
        loop {
            if let Some(ref mut to) = self.timeout {
                ready!(Pin::new(to).poll(cx));
            }
            self.timeout = None;

            let e: std::io::Error = match &mut self.i {
                ListenerImpl::Tcp {
                    s,
                    nodelay,
                    keepalive,
                } => match s.poll_accept(cx) {
                    Poll::Ready(Err(e)) => e,
                    Poll::Ready(Ok((c, a))) => {
                        debug!(fromaddr=%a, r#type="tcp", "incoming connection");
                        if *nodelay {
                            c.set_nodelay(true)?;
                        }
                        if let Some(ka) = keepalive {
                            let sock_ref = socket2::SockRef::from(&c);
                            sock_ref.set_tcp_keepalive(ka)?;
                        }
                        return Poll::Ready(Ok((
                            Connection(ConnectionImpl::Tcp(c)),
                            SomeSocketAddr::Tcp(a),
                        )));
                    }
                    Poll::Pending => return Poll::Pending,
                },
                #[cfg(unix)]
                ListenerImpl::Unix(x) => match x.poll_accept(cx) {
                    Poll::Ready(Err(e)) => e,
                    Poll::Ready(Ok((s, a))) => {
                        debug!(r#type = "unix", "incoming connection");
                        return Poll::Ready(Ok((
                            Connection(ConnectionImpl::Unix(s)),
                            SomeSocketAddr::Unix(a),
                        )));
                    }
                    Poll::Pending => return Poll::Pending,
                },
                ListenerImpl::Stdio(x) => {
                    match x.token.take() {
                        Some(tx) => {
                            debug!(r#type = "stdio", "incoming connection");
                            return Poll::Ready(Ok((
                                Connection(ConnectionImpl::Stdio(
                                    tokio::io::stdin(),
                                    tokio::io::stdout(),
                                    Some(tx),
                                )),
                                SomeSocketAddr::Stdio,
                            )));
                        }
                        None => match Pin::new(&mut x.rx).poll(cx) {
                            Poll::Ready(..) => {
                                trace!("finished waiting for liberation of stdout to stop listening loop");
                                return Poll::Ready(Err(std::io::Error::new(
                                    std::io::ErrorKind::Other,
                                    "stdin/stdout pseudosocket is already used",
                                )));
                            }
                            Poll::Pending => return Poll::Pending,
                        },
                    }
                }
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

    /// See main [`Listener::bind`] documentation for specifics of how it accepts conenctions
    pub async fn accept(&mut self) -> std::io::Result<(Connection, SomeSocketAddr)> {
        std::future::poll_fn(|cx|self.poll_accept(cx)).await
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
/// Based on https://docs.rs/hyper/latest/src/hyper/server/tcp.rs.html#109-116
fn is_connection_error(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
    )
}

/// Accepted connection, which can be a TCP socket, AF_UNIX stream socket or a stdin/stdout pair.
/// 
/// Although inner enum is private, you can use methods or `From` impls to convert this to/from usual Tokio types.
#[pin_project]
pub struct Connection(#[pin] ConnectionImpl);

#[derive(Debug)]
#[pin_project(project = ConnectionImplProj)]
enum ConnectionImpl {
    Tcp(#[pin] TcpStream),
    #[cfg(unix)]
    Unix(#[pin] UnixStream),
    Stdio(
        #[pin] tokio::io::Stdin,
        #[pin] tokio::io::Stdout,
        Option<Sender<()>>,
    ),
}

impl Connection {
    #[allow(missing_docs)]
    pub fn try_into_tcp(self) -> Result<TcpStream, Self> {
        if let ConnectionImpl::Tcp(s) = self.0 {
            Ok(s)
        } else {
            Err(self)
        }
    }
    #[allow(missing_docs)]
    #[cfg(unix)]
    pub fn try_into_unix(self) -> Result<UnixStream, Self> {
        if let ConnectionImpl::Unix(s) = self.0 {
            Ok(s)
        } else {
            Err(self)
        }
    }
    #[allow(missing_docs)]
    pub fn try_into_stdio(self) -> Result<(Stdin, Stdout, Option<Sender<()>>), Self> {
        if let ConnectionImpl::Stdio(i, o, f) = self.0 {
            Ok((i, o, f))
        } else {
            Err(self)
        }
    }

    #[allow(missing_docs)]
    pub fn try_borrow_tcp(&self) -> Option<&TcpStream> {
        if let ConnectionImpl::Tcp(ref s) = self.0 {
            Some(s)
        } else {
            None
        }
    }
    #[cfg(unix)]
    #[allow(missing_docs)]
    pub fn try_borrow_unix(&self) -> Option<&UnixStream> {
        if let ConnectionImpl::Unix(ref s) = self.0 {
            Some(s)
        } else {
            None
        }
    }
    #[allow(missing_docs)]
    pub fn try_borrow_stdio(&self) -> Option<(&Stdin, &Stdout)> {
        if let ConnectionImpl::Stdio(ref i, ref o, ..) = self.0 {
            Some((i, o))
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
#[cfg(unix)]
impl From<UnixStream> for Connection {
    fn from(s: UnixStream) -> Self {
        Connection(ConnectionImpl::Unix(s))
    }
}
impl From<(Stdin, Stdout, Option<Sender<()>>)> for Connection {
    fn from(s: (Stdin, Stdout, Option<Sender<()>>)) -> Self {
        Connection(ConnectionImpl::Stdio(s.0, s.1, s.2))
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
            #[cfg(unix)]
            ConnectionImplProj::Unix(s) => s.poll_read(cx, buf),
            ConnectionImplProj::Stdio(s, _, _) => s.poll_read(cx, buf),
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
            #[cfg(unix)]
            ConnectionImplProj::Unix(s) => s.poll_write(cx, buf),
            ConnectionImplProj::Stdio(_, s, _) => s.poll_write(cx, buf),
        }
    }

    #[inline]
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        let q: Pin<&mut ConnectionImpl> = self.project().0;
        match q.project() {
            ConnectionImplProj::Tcp(s) => s.poll_flush(cx),
            #[cfg(unix)]
            ConnectionImplProj::Unix(s) => s.poll_flush(cx),
            ConnectionImplProj::Stdio(_, s, _) => s.poll_flush(cx),
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
            #[cfg(unix)]
            ConnectionImplProj::Unix(s) => s.poll_shutdown(cx),
            ConnectionImplProj::Stdio(_, s, tx) => match s.poll_shutdown(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(ret) => {
                    if let Some(tx) = tx.take() {
                        if tx.send(()).is_err() {
                            warn!("stdout wrapper for inetd mode failed to notify the listener to abort listening loop");
                        } else {
                            debug!("stdout finished in inetd mode. Aborting the listening loop.")
                        }
                    }
                    Poll::Ready(ret)
                }
            },
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
            #[cfg(unix)]
            ConnectionImplProj::Unix(s) => s.poll_write_vectored(cx, bufs),
            ConnectionImplProj::Stdio(_, s, _) => s.poll_write_vectored(cx, bufs),
        }
    }

    #[inline]
    fn is_write_vectored(&self) -> bool {
        match &self.0 {
            ConnectionImpl::Tcp(s) => s.is_write_vectored(),
            #[cfg(unix)]
            ConnectionImpl::Unix(s) => s.is_write_vectored(),
            ConnectionImpl::Stdio(_, s, _) => s.is_write_vectored(),
        }
    }
}

/// Some form of accepted connection's address.
/// Variant depends on variant used in [`ListenerAddress`].
#[derive(Debug)]
#[non_exhaustive]
#[allow(missing_docs)]
pub enum SomeSocketAddr {
    Tcp(SocketAddr),
    #[cfg(unix)]
    Unix(tokio::net::unix::SocketAddr),
    Stdio,
}

impl Display for SomeSocketAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SomeSocketAddr::Tcp(x) => x.fmt(f),
            #[cfg(unix)]
            SomeSocketAddr::Unix(_x) => "unix".fmt(f),
            SomeSocketAddr::Stdio => "stdio".fmt(f),
        }
    }
}

#[cfg(feature="hyper014")]
mod hyper014 {
    use std::{
        pin::Pin,
        task::{self, Poll},
    };

    use hyper::server::accept::Accept;

    use crate::Listener;

    impl Accept for Listener {
        type Conn = crate::Connection;
        type Error = std::io::Error;

        fn poll_accept(
            self: std::pin::Pin<&mut Self>,
            cx: &mut task::Context<'_>,
        ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
           
            let nmc = self.no_more_connections();
            match crate::Listener::poll_accept(Pin::<_>::into_inner(self), cx) {
                Poll::Ready(Ok((s, _a))) => Poll::Ready(Some(Ok(s))),
                Poll::Ready(Err(e)) => {
                    if nmc {
                        return Poll::Ready(None);
                    }
                    Poll::Ready(Some(Err(e)))
                },
                Poll::Pending => Poll::Pending,
            }
        }
    }
}
