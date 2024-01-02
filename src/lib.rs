#![cfg_attr(docsrs_alt, feature(doc_cfg))]
#![warn(missing_docs)]
#![allow(clippy::useless_conversion)]
//! Library for abstracting over TCP server sockets, UNIX server sockets, inetd-like mode.
//!
//! [`ListenerAddress`] is like `SocketAddr` and [`Listener`] is like `TcpListener`, but with more flexibility.
//!
//! ```,no_run
//! # tokio_test::block_on(async {
//! # use tokio_listener::*;
//! let addr1 : ListenerAddress = "127.0.0.1:8087".parse().unwrap();
//! let addr2 : ListenerAddress = "/path/to/socket".parse().unwrap();
//! let addr3 : ListenerAddress = "@abstract_linux_address".parse().unwrap();
//!
//! let system_options : SystemOptions = Default::default();
//! let user_options : UserOptions = Default::default();
//!
//! let mut l = Listener::bind(&addr1, &system_options, &user_options).await.unwrap();
//! while let Ok((conn, addr)) = l.accept().await {
//!     // ...
//! }
//! # });
//! ```
//!
//!  There is special integration with `clap`:
//!
//! ```,no_run
//! # #[cfg(feature="clap")] {
//! use clap::Parser;
//!
//! #[derive(Parser)]
//! /// Demo application for tokio-listener
//! struct Args {
//!     #[clap(flatten)]
//!     listener: tokio_listener::ListenerAddressPositional,
//! }
//!
//! # tokio_test::block_on(async {
//! let args = Args::parse();
//!
//! let listener = args.listener.bind().await.unwrap();
//!
//! let app = axum06::Router::new().route("/", axum06::routing::get(|| async { "Hello, world\n" }));
//!
//! axum06::Server::builder(listener).serve(app.into_make_service()).await;
//! # }) }
//! ```
//!
//! See project [README](https://github.com/vi/tokio-listener/blob/main/README.md) for details and more examples.
//!
//! ## Feature flags
#![doc = document_features::document_features!()]
//!
//! Disabling default features bring tokio-listener close to usual `TcpListener`.

#![cfg_attr(not(feature = "sd_listen"), deny(unsafe_code))]
#![cfg_attr(
    not(feature = "default"),
    allow(unused_imports, irrefutable_let_patterns, unused_variables)
)]

use std::{
    fmt::Display,
    net::SocketAddr,
    path::PathBuf,
    pin::Pin,
    str::FromStr,
    task::{ready, Context, Poll},
    time::Duration, ffi::c_int, sync::Arc,
};

#[cfg(unix)]
use std::os::fd::RawFd;

use futures_core::{Future, Stream};
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

#[cfg_attr(docsrs_alt, doc(cfg(feature = "unix_path_tools")))]
#[cfg(feature = "unix_path_tools")]
/// Value of `--unix-listen-chmod` option which allows changing DAC file access mode for UNIX path socket
#[non_exhaustive]
#[cfg_attr(
    feature = "serde",
    derive(serde_with::DeserializeFromStr, serde_with::SerializeDisplay)
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnixChmodVariant {
    /// Set filesystem mode of the UNIX socket to `u+rw`, allowing access only to one uid
    Owner,
    /// Set filesystem mode of the UNIX socket to `ug+rw`, allowing access to owner uid and a group
    Group,
    /// Set filesystem mode of the UNIX socket to `a+rw`, allowing global access to the socket
    Everybody,
}

#[cfg(feature = "unix_path_tools")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "unix_path_tools")))]
impl Display for UnixChmodVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnixChmodVariant::Owner => "owner".fmt(f),
            UnixChmodVariant::Group => "group".fmt(f),
            UnixChmodVariant::Everybody => "everybody".fmt(f),
        }
    }
}

#[cfg(feature = "unix_path_tools")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "unix_path_tools")))]
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

#[cfg(feature = "socket_options")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "socket_options")))]
/// Value of `--tcp-keepalive` option.
///
/// When parsed from string, it expects 0 to 3 colon-separated numbers:
///
/// * Timeout (in milliseconds)
/// * Number of failed pings before failing the connection
/// * Interval of pings (in milliseconds)
///
/// Specifying empty string or "" just requests to enable keepalives without configuring parameters.
///
/// On unsupported platforms, all or some details of keepalives may be ignored.
///
/// Example:
///
/// ```
/// use tokio_listener::TcpKeepaliveParams;
/// let k1 : TcpKeepaliveParams = "60000:3:5000".parse().unwrap();
/// let k2 : TcpKeepaliveParams = "60000".parse().unwrap();
/// let k3 : TcpKeepaliveParams = "".parse().unwrap();
///
/// assert_eq!(k1, TcpKeepaliveParams{timeout_ms:Some(60000), count:Some(3), interval_ms:Some(5000)});
/// ```
#[cfg_attr(
    feature = "serde",
    derive(serde_with::DeserializeFromStr, serde_with::SerializeDisplay)
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TcpKeepaliveParams {
    /// Amount of time after which TCP keepalive probes will be sent on idle connections.
    pub timeout_ms: Option<u32>,
    /// Maximum number of TCP keepalive probes that will be sent before dropping a connection.
    pub count: Option<u32>,
    /// Time interval between TCP keepalive probes.
    pub interval_ms: Option<u32>,
}
#[cfg(feature = "socket_options")]
impl Display for TcpKeepaliveParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(x) = self.timeout_ms {
            x.fmt(f)?;
        }
        ':'.fmt(f)?;
        if let Some(x) = self.count {
            x.fmt(f)?;
        }
        ':'.fmt(f)?;
        if let Some(x) = self.interval_ms {
            x.fmt(f)?;
        }
        Ok(())
    }
}
#[cfg(feature = "socket_options")]
impl FromStr for TcpKeepaliveParams {
    type Err = &'static str;

    #[allow(clippy::get_first)]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let chunks: Vec<&str> = s.split(':').collect();
        if chunks.len() > 3 {
            return Err("Too many colon-separated chunks");
        }
        let t = chunks.get(0).unwrap_or(&"").trim();
        let n = chunks.get(1).unwrap_or(&"").trim();
        let i = chunks.get(2).unwrap_or(&"").trim();

        let mut k = TcpKeepaliveParams::default();

        if !t.is_empty() {
            k.timeout_ms = Some(
                t.parse()
                    .map_err(|_| "failed to parse timeout as a number")?,
            );
        }
        if !n.is_empty() {
            k.count = Some(n.parse().map_err(|_| "failed to parse count as a number")?);
        }
        if !i.is_empty() {
            k.interval_ms = Some(
                i.parse()
                    .map_err(|_| "failed to parse interval as a number")?,
            );
        }

        Ok(k)
    }
}
#[cfg(feature = "socket_options")]
impl TcpKeepaliveParams {
    /// Attempt to convert values of this struct to socket2 format.
    ///
    /// Some fields may be ignored depending on platform.
    #[must_use] pub fn to_socket2(&self) -> socket2::TcpKeepalive {
        let mut k = socket2::TcpKeepalive::new();

        if let Some(x) = self.timeout_ms {
            k = k.with_time(Duration::from_millis(u64::from(x)));
        }

        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "illumos",
            target_os = "ios",
            target_os = "linux",
            target_os = "macos",
            target_os = "netbsd",
            target_os = "tvos",
            target_os = "watchos",
        ))]
        if let Some(x) = self.count {
            k = k.with_retries(x);
        }

        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "illumos",
            target_os = "ios",
            target_os = "linux",
            target_os = "macos",
            target_os = "netbsd",
            target_os = "tvos",
            target_os = "watchos",
            target_os = "windows",
        ))]
        if let Some(x) = self.interval_ms {
            k = k.with_interval(Duration::from_millis(u64::from(x)));
        }

        k
    }
}

#[cfg_attr(feature = "clap", derive(clap::Args))]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
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
/// All options are always available regardless of current platform, but may be hidden from --help.
///
/// Disabling related crate features removes them for good though.
pub struct UserOptions {
    #[cfg(feature = "unix_path_tools")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "unix_path_tools")))]
    /// remove UNIX socket prior to binding to it
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "serde", serde(default))]
    #[cfg_attr(all(feature = "clap", not(unix)), clap(hide = true))]
    pub unix_listen_unlink: bool,

    #[cfg(feature = "unix_path_tools")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "unix_path_tools")))]
    /// change filesystem mode of the newly bound UNIX socket to `owner`, `group` or `everybody`
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(all(feature = "clap", not(unix)), clap(hide = true))]
    #[cfg_attr(feature = "serde", serde(default))]
    pub unix_listen_chmod: Option<UnixChmodVariant>,

    #[cfg(feature = "unix_path_tools")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "unix_path_tools")))]
    /// change owner user of the newly bound UNIX socket to this numeric uid
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "serde", serde(default))]
    #[cfg_attr(all(feature = "clap", not(unix)), clap(hide = true))]
    pub unix_listen_uid: Option<u32>,

    #[cfg(feature = "unix_path_tools")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "unix_path_tools")))]
    /// change owner group of the newly bound UNIX socket to this numeric uid
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "serde", serde(default))]
    #[cfg_attr(all(feature = "clap", not(unix)), clap(hide = true))]
    pub unix_listen_gid: Option<u32>,

    #[cfg(feature = "sd_listen")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "sd_listen")))]
    /// ignore environment variables like LISTEN_PID or LISTEN_FDS and unconditionally use
    /// file descritor `3` as a socket in sd-listen or sd-listen-unix modes
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "serde", serde(default))]
    #[cfg_attr(all(feature = "clap", not(unix)), clap(hide = true))]
    pub sd_accept_ignore_environment: bool,

    #[cfg(feature = "socket_options")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "socket_options")))]
    /// set SO_KEEPALIVE settings for each accepted TCP connection.
    /// 
    /// Value is a colon-separated triplet of time_ms:count:interval_ms, each of which is optional.
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "serde", serde(default))]
    pub tcp_keepalive: Option<TcpKeepaliveParams>,

    #[cfg(feature = "socket_options")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "socket_options")))]
    /// Try to set SO_REUSEPORT, so that multiple processes can accept connections from the same port
    /// in a round-robin fashion
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "serde", serde(default))]
    pub tcp_reuse_port: bool,

    #[cfg(feature = "socket_options")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "socket_options")))]
    /// Set socket's SO_RCVBUF value.
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "serde", serde(default))]
    pub recv_buffer_size: Option<usize>,

    #[cfg(feature = "socket_options")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "socket_options")))]
    /// Set socket's SO_SNDBUF value.
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "serde", serde(default))]
    pub send_buffer_size: Option<usize>,

    #[cfg(feature = "socket_options")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "socket_options")))]
    /// Set socket's IPV6_V6ONLY to true, to avoid receiving IPv4 connections on IPv6 socket.
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "serde", serde(default))]
    pub tcp_only_v6: bool,

    #[cfg(feature = "socket_options")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "socket_options")))]
    /// Maximum number of pending unaccepted connections
    #[cfg_attr(feature = "clap", clap(long))]
    #[cfg_attr(feature = "serde", serde(default))]
    pub tcp_listen_backlog: Option<u32>,
}

/// Abstraction over socket address that instructs in which way and at what address (if any) [`Listener`]
/// should listen for incoming stream connections.
///
/// All address variants are available on all platforms, regardness of actual support in the Listener or enabled crate features.
///
/// If serde is enabled, it is serialized/deserialized the same as string, same as as in the CLI, using `FromStr`/`Display`.
///
/// See variants documentation for `FromStr` string patterns that are accepted by `ListenerAddress` parser
///
/// If you are not using clap helper types then remember to copy or link those documentation snippets into your app's documentation.
///
/// ```
/// # use tokio_listener::*;
/// let addr : ListenerAddress = "127.0.0.1:8087".parse().unwrap();
/// let addr : ListenerAddress = "[::]:80".parse().unwrap();
/// let addr : ListenerAddress = "/path/to/socket".parse().unwrap();
/// let addr : ListenerAddress = "@abstract_linux_address".parse().unwrap();
/// let addr : ListenerAddress = "inetd".parse().unwrap();
/// let addr : ListenerAddress = "sd-listen".parse().unwrap();
/// let addr : ListenerAddress = "SD_LISTEN".parse().unwrap();
/// let addr : ListenerAddress = "sd-listen-unix".parse().unwrap();
/// ```
#[non_exhaustive]
#[cfg_attr(
    feature = "serde",
    derive(serde_with::DeserializeFromStr, serde_with::SerializeDisplay)
)]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ListenerAddress {
    /// Usual server TCP socket. triggered by specifying IPv4 or IPv6 address and port pair.  
    /// Example: `127.0.0.1:8080`.
    ///
    /// Hostnames are not supported.
    Tcp(SocketAddr),
    /// Path-based UNIX socket. Path must begin with `/` or `.`.  
    /// Examples: `/tmp/mysock`, `./mysock`
    Path(PathBuf),
    /// Linux abstract-namespaced UNIX socket. Indicated by using `@` as a first character.
    /// Example: `@server`
    Abstract(String),
    /// "inetd" or "Accept=yes" mode where stdin and stdout (file descriptors 0 and 1) are using together as a socket
    /// and only one connection is served. Triggered by using `inetd` or `stdio` or `-` as the address.
    Inetd,
    /// "Accept=no" mode - using manually specified file descriptor as a pre-created server socket reeady to accept TCP connections.
    /// Triggered by specifying `sd-listen` as address, which sets `3` as file descriptor number
    FromFdTcp(i32),
    /// "Accept=no" mode - using fmanually specified ile descriptor as a pre-created server socket reeady to accept Unix connections.
    /// Triggered by specifying `sd-listen-unix` as address, which sets `3` as file descriptor number
    FromFdUnix(i32),
}

#[cfg(feature = "clap")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "clap")))]
mod claptools {
    #![allow(rustdoc::broken_intra_doc_links)]

    use clap::Parser;
    /// Clap helper to require listener address as a required positional argument `listen_address`,
    /// for `clap(flatten)`-ing into your primary options struct.
    ///
    /// Also invides a number of additional optional options to adjust the way it listens.
    ///
    /// Provides documentation about how to specify listening addresses into `--help`.
    ///
    /// Example:
    ///
    /// ```,no_run
    /// # use clap::Parser;
    /// #[derive(Parser)]
    /// /// Doc comment here is highly adviced
    /// struct Args {
    ///    #[clap(flatten)]
    ///    listener: tokio_listener::ListenerAddressPositional,
    /// }
    /// ```
    #[derive(Parser)]
    pub struct ListenerAddressPositional {
        /// Socket address to listen for incoming connections.  
        ///
        /// Various types of addresses are supported:
        ///
        /// * TCP socket address and port, like 127.0.0.1:8080 or [::]:80
        ///
        /// * UNIX socket path like /tmp/mysock or Linux abstract address like @abstract
        ///
        /// * Special keyword "inetd" for serving one connection from stdin/stdout
        ///
        /// * Special keyword "sd-listen" or "sd-listen-unix" to accept connections from file descriptor 3 (e.g. systemd socket activation)
        ///
        #[cfg_attr(
            not(any(target_os = "linux", target_os = "android")),
            doc = "Note that this platform does not support all the modes described above."
        )]
        #[cfg_attr(
            not(feature = "user_facing_default"),
            doc = "Note that some features may be disabled by compile-time settings."
        )]
        pub listen_address: crate::ListenerAddress,

        #[allow(missing_docs)]
        #[clap(flatten)]
        pub listener_options: crate::UserOptions,
    }

    /// Clap helper to provide optional listener address  a named argument `--listen-address` or `-l`.
    ///
    /// For `clap(flatten)`-ing into your primary options struct.
    ///
    /// Also invides a number of additional optional options to adjust the way it listens.
    ///
    /// Provides documentation about how to specify listening addresses into `--help`.
    ///
    /// Example:
    ///
    /// ```,no_run
    /// # use clap::Parser;
    /// #[derive(Parser)]
    /// /// Doc comment here is highly adviced
    /// struct Args {
    ///    #[clap(flatten)]
    ///    listener: tokio_listener::ListenerAddressLFlag,
    /// }
    /// ```
    #[derive(Parser)]
    pub struct ListenerAddressLFlag {
        /// Socket address to listen for incoming connections.  
        ///
        /// Various types of addresses are supported:
        ///
        /// * TCP socket address and port, like 127.0.0.1:8080 or [::]:80
        ///
        /// * UNIX socket path like /tmp/mysock or Linux abstract address like @abstract
        ///
        /// * Special keyword "inetd" for serving one connection from stdin/stdout
        ///
        /// * Special keyword "sd-listen" or "sd-listen-unix" to accept connections from file descriptor 3 (e.g. systemd socket activation)
        ///
        #[cfg_attr(
            not(any(target_os = "linux", target_os = "android")),
            doc = "Note that this platform does not support all the modes described above."
        )]
        #[cfg_attr(
            not(feature = "user_facing_default"),
            doc = "Note that some features may be disabled by compile-time settings."
        )]
        #[clap(short = 'l', long = "listen-address")]
        pub listen_address: Option<crate::ListenerAddress>,

        #[allow(missing_docs)]
        #[clap(flatten)]
        pub listener_options: crate::UserOptions,
    }

    impl ListenerAddressPositional {
        /// Simple function to activate the listener without any extra parameters (just as nodelay, retries or keepalives) based on supplied arguments.
        pub async fn bind(&self) -> std::io::Result<crate::Listener> {
            crate::Listener::bind(
                &self.listen_address,
                &crate::SystemOptions::default(),
                &self.listener_options,
            )
            .await
        }
    }
    impl ListenerAddressLFlag {
        /// Simple function to activate the listener (if it is set)
        /// without any extra parameters (just as nodelay, retries or keepalives) based on supplied arguments.
        pub async fn bind(&self) -> Option<std::io::Result<crate::Listener>> {
            if let Some(addr) = self.listen_address.as_ref() {
                Some(
                    crate::Listener::bind(
                        addr,
                        &crate::SystemOptions::default(),
                        &self.listener_options,
                    )
                    .await,
                )
            } else {
                None
            }
        }
    }
}
#[cfg(feature = "clap")]
pub use claptools::{ListenerAddressLFlag, ListenerAddressPositional};

const SD_LISTEN_FDS_START: u32 = 3;

impl FromStr for ListenerAddress {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with('/') || s.starts_with("./") {
            Ok(ListenerAddress::Path(s.into()))
        } else if let Some(x) = s.strip_prefix('@') {
            Ok(ListenerAddress::Abstract(x.to_owned()))
        } else if s.eq_ignore_ascii_case("inetd") || s.eq_ignore_ascii_case("stdio") || s == "-" {
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
                    if p.is_absolute() || s.starts_with("./") {
                        s.fmt(f)
                    } else {
                        write!(f, "./{s}")
                    }
                } else if p.is_absolute() {
                    "/???".fmt(f)
                } else {
                    "./???".fmt(f)
                }
            }
            ListenerAddress::Abstract(p) => {
                write!(f, "@{p}")
            }
            ListenerAddress::Inetd => "inetd".fmt(f),
            ListenerAddress::FromFdTcp(fd) => {
                if *fd == SD_LISTEN_FDS_START as i32 {
                    "sd-listen".fmt(f)
                } else {
                    write!(f, "accept-tcp-from-fd:{fd}")
                }
            }
            ListenerAddress::FromFdUnix(fd) => {
                if *fd == SD_LISTEN_FDS_START as i32 {
                    "sd-listen-unix".fmt(f)
                } else {
                    write!(f, "accept-unix-from-fd:{fd}")
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
    ///
    /// This field is available even without `socket_options` crate feature.
    pub nodelay: bool,
}

/// Configured TCP. `AF_UNIX` or other stream socket acceptor.
///
/// Based on extended hyper 0.14's `AddrIncoming` code.
pub struct Listener {
    i: ListenerImpl,
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

#[cfg(feature = "sd_listen")]
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
            ListenerAddress::Tcp(a) => {
                #[cfg(not(feature="socket_options"))]
                let s = TcpListener::bind(a).await?;
                #[cfg(feature="socket_options")]
                let s = if uopts.tcp_only_v6 || uopts.tcp_reuse_port || uopts.tcp_listen_backlog.is_some() {
                    let s = socket2::Socket::new(socket2::Domain::for_address(*a), socket2::Type::STREAM, None)?;
                    if uopts.tcp_only_v6 {
                        s.set_only_v6(true)?;
                    }
                    #[cfg(all(unix,not(any(target_os = "solaris", target_os = "illumos"))))]
                    if uopts.tcp_reuse_port {
                        s.set_reuse_port(true)?;
                    }
                    s.bind(&socket2::SockAddr::from(*a))?;
                    s.listen(uopts.tcp_listen_backlog.unwrap_or(1024) as c_int)?;
                    s.set_nonblocking(true)?;
                    TcpListener::from_std(std::net::TcpListener::from(s))?
                } else {
                    TcpListener::bind(a).await?
                };
                ListenerImpl::Tcp {
                s,
                nodelay: sopts.nodelay,
                #[cfg(feature = "socket_options")]
                keepalive: uopts
                    .tcp_keepalive
                    .as_ref()
                    .map(TcpKeepaliveParams::to_socket2),
                #[cfg(feature = "socket_options")]
                recv_buffer_size: uopts.recv_buffer_size,
                #[cfg(feature = "socket_options")]
                send_buffer_size: uopts.send_buffer_size,
            }},
            #[cfg(all(unix, feature = "unix"))]
            ListenerAddress::Path(p) => {
                #[cfg(feature = "unix_path_tools")]
                #[allow(clippy::collapsible_if)]
                if uopts.unix_listen_unlink {
                    if std::fs::remove_file(p).is_ok() {
                        debug!(file=?p, "removed UNIX socket before listening");
                    }
                }
                let i = ListenerImpl::Unix {
                    s: UnixListener::bind(p)?,
                    #[cfg(feature = "socket_options")]
                    recv_buffer_size: uopts.recv_buffer_size,
                    #[cfg(feature = "socket_options")]
                    send_buffer_size: uopts.send_buffer_size,
                };
                #[cfg(feature = "unix_path_tools")]
                {
                    if let Some(chmod) = uopts.unix_listen_chmod {
                        let mode = match chmod {
                            UnixChmodVariant::Owner => 0o006,
                            UnixChmodVariant::Group => 0o066,
                            UnixChmodVariant::Everybody => 0o666,
                        };
                        use std::os::unix::fs::PermissionsExt;
                        let perms = std::fs::Permissions::from_mode(mode);
                        std::fs::set_permissions(p, perms)?;
                    }
                    if (uopts.unix_listen_uid, uopts.unix_listen_gid) != (None, None) {
                        let uid = uopts.unix_listen_uid.map(Into::into);
                        let gid = uopts.unix_listen_gid.map(Into::into);
                        nix::unistd::chown(p, uid, gid)?;
                    }
                }
                i
            }
            #[cfg(all(feature = "unix", any(target_os = "linux", target_os = "android")))]
            ListenerAddress::Abstract(a) => {
                #[cfg(target_os = "linux")]
                use std::os::linux::net::SocketAddrExt;
                #[cfg(target_os = "android")]
                use std::os::android::net::SocketAddrExt;
                let a = std::os::unix::net::SocketAddr::from_abstract_name(a)?;
                let s = std::os::unix::net::UnixListener::bind_addr(&a)?;
                s.set_nonblocking(true)?;
                ListenerImpl::Unix {
                    s: UnixListener::from_std(s)?,
                    #[cfg(feature = "socket_options")]
                    recv_buffer_size: uopts.recv_buffer_size,
                    #[cfg(feature = "socket_options")]
                    send_buffer_size: uopts.send_buffer_size,
                }
            }
            #[cfg(feature = "inetd")]
            ListenerAddress::Inetd => {
                let (tx, rx) = channel();
                ListenerImpl::Stdio(StdioListener {
                    rx,
                    token: Some(tx),
                })
            }
            #[cfg(all(feature = "sd_listen", unix))]
            ListenerAddress::FromFdTcp(fdnum) => {
                if !uopts.sd_accept_ignore_environment && check_env_for_fd(*fdnum).is_none() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Failed LISTEN_PID or LISTEN_FDS environment check for sd-listen mode",
                    ));
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
                    #[cfg(feature = "socket_options")]
                    keepalive: uopts
                        .tcp_keepalive
                        .as_ref()
                        .map(TcpKeepaliveParams::to_socket2),
                    #[cfg(feature = "socket_options")]
                    recv_buffer_size: uopts.recv_buffer_size,
                    #[cfg(feature = "socket_options")]
                    send_buffer_size: uopts.send_buffer_size,
                }
            }
            #[cfg(all(feature = "sd_listen", feature = "unix", unix))]
            ListenerAddress::FromFdUnix(fdnum) => {
                if !uopts.sd_accept_ignore_environment && check_env_for_fd(*fdnum).is_none() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Failed LISTEN_PID or LISTEN_FDS environment check for sd-listen mode",
                    ));
                }
                let fd: RawFd = (*fdnum).into();
                use std::os::fd::FromRawFd;
                // Safety: we assume end user is reasonable and won't specify too tricky file descriptor numbers.
                // Besides checking systemd's environment variables, we can't do much to prevent misuse anyway.
                let s = unsafe { std::os::unix::net::UnixListener::from_raw_fd(fd) };
                s.set_nonblocking(true)?;
                ListenerImpl::Unix {
                    s: UnixListener::from_std(s)?,
                    #[cfg(feature = "socket_options")]
                    send_buffer_size: uopts.send_buffer_size,

                    #[cfg(feature = "socket_options")]
                    recv_buffer_size: uopts.recv_buffer_size,
                }
            }
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
            sleep_on_errors: sopts.sleep_on_errors,
            timeout: None,
        })
    }
}

#[cfg(feature = "inetd")]
struct StdioListener {
    rx: Receiver<()>,
    token: Option<Sender<()>>,
}

enum ListenerImpl {
    Tcp {
        s: TcpListener,
        nodelay: bool,
        #[cfg(feature = "socket_options")]
        keepalive: Option<socket2::TcpKeepalive>,
        #[cfg(feature = "socket_options")]
        recv_buffer_size: Option<usize>,
        #[cfg(feature = "socket_options")]
        send_buffer_size: Option<usize>,
    },
    #[cfg(all(feature = "unix", unix))]
    Unix {
        s: UnixListener,
        #[cfg(feature = "socket_options")]
        recv_buffer_size: Option<usize>,
        #[cfg(feature = "socket_options")]
        send_buffer_size: Option<usize>,
    },
    #[cfg(feature = "inetd")]
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
    #[cfg(all(feature = "unix", unix))]
    #[cfg_attr(docsrs_alt, doc(cfg(all(feature = "unix", unix))))]
    pub fn try_borrow_unix_listener(&self) -> Option<&UnixListener> {
        if let ListenerImpl::Unix { s: ref x, .. } = self.i {
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
    #[cfg(all(feature = "unix", unix))]
    #[cfg_attr(docsrs_alt, doc(cfg(all(feature = "unix", unix))))]
    pub fn try_into_unix_listener(self) -> Result<UnixListener, Self> {
        if let ListenerImpl::Unix { s: x, .. } = self.i {
            Ok(x)
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

            let e: std::io::Error = match &mut self.i {
                ListenerImpl::Tcp {
                    s,
                    nodelay,
                    #[cfg(feature = "socket_options")]
                    keepalive,
                    #[cfg(feature = "socket_options")]
                    recv_buffer_size,
                    #[cfg(feature = "socket_options")]
                    send_buffer_size,
                } => match s.poll_accept(cx) {
                    Poll::Ready(Err(e)) => e,
                    Poll::Ready(Ok((c, a))) => {
                        debug!(fromaddr=%a, r#type="tcp", "incoming connection");
                        if *nodelay {
                            c.set_nodelay(true)?;
                        }
                        #[cfg(feature = "socket_options")]
                        {
                            if let Some(ka) = keepalive {
                                let sock_ref = socket2::SockRef::from(&c);
                                sock_ref.set_tcp_keepalive(ka)?;
                            }
                            if let Some(n) = recv_buffer_size {
                                let sock_ref = socket2::SockRef::from(&c);
                                sock_ref.set_recv_buffer_size(*n)?;
                            }
                            if let Some(n) = send_buffer_size {
                                let sock_ref = socket2::SockRef::from(&c);
                                sock_ref.set_send_buffer_size(*n)?;
                            }
                        }
                        return Poll::Ready(Ok((
                            Connection(ConnectionImpl::Tcp(c)),
                            SomeSocketAddr::Tcp(a),
                        )));
                    }
                    Poll::Pending => return Poll::Pending,
                },
                #[cfg(all(feature = "unix", unix))]
                ListenerImpl::Unix {
                    s: x,
                    #[cfg(feature = "socket_options")]
                    recv_buffer_size,
                    #[cfg(feature = "socket_options")]
                    send_buffer_size,
                } => match x.poll_accept(cx) {
                    Poll::Ready(Err(e)) => e,
                    Poll::Ready(Ok((c, a))) => {
                        debug!(r#type = "unix", "incoming connection");
                        #[cfg(feature = "socket_options")]
                        {
                            if let Some(n) = recv_buffer_size {
                                let sock_ref = socket2::SockRef::from(&c);
                                sock_ref.set_recv_buffer_size(*n)?;
                            }
                            if let Some(n) = send_buffer_size {
                                let sock_ref = socket2::SockRef::from(&c);
                                sock_ref.set_send_buffer_size(*n)?;
                            }
                        }
                        return Poll::Ready(Ok((
                            Connection(ConnectionImpl::Unix(c)),
                            SomeSocketAddr::Unix(a),
                        )));
                    }
                    Poll::Pending => return Poll::Pending,
                },
                #[cfg(feature = "inetd")]
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

impl std::fmt::Debug  for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            ConnectionImpl::Tcp(_) => f.write_str("Connection(tcp)"),
            #[cfg(all(feature = "unix", unix))]
            ConnectionImpl::Unix(_) => f.write_str("Connection(unix)"),
            #[cfg(feature = "inetd")]
            ConnectionImpl::Stdio(_, _, _) => f.write_str("Connection(stdio)"),
        }
    }
}

#[derive(Debug)]
#[pin_project(project = ConnectionImplProj)]
enum ConnectionImpl {
    Tcp(#[pin] TcpStream),
    #[cfg(all(feature = "unix", unix))]
    Unix(#[pin] UnixStream),
    #[cfg(feature = "inetd")]
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

    #[allow(missing_docs)]
    pub fn try_borrow_tcp(&self) -> Option<&TcpStream> {
        if let ConnectionImpl::Tcp(ref s) = self.0 {
            Some(s)
        } else {
            None
        }
    }
    #[cfg(all(feature = "unix", unix))]
    #[cfg_attr(docsrs_alt, doc(cfg(all(feature = "unix", unix))))]
    #[allow(missing_docs)]
    pub fn try_borrow_unix(&self) -> Option<&UnixStream> {
        if let ConnectionImpl::Unix(ref s) = self.0 {
            Some(s)
        } else {
            None
        }
    }
    #[cfg(feature = "inetd")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "inetd")))]
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
    #[cfg(all(feature = "unix", unix))]
    #[cfg_attr(docsrs_alt, doc(cfg(all(feature = "unix", unix))))]
    Unix(tokio::net::unix::SocketAddr),
    #[cfg(feature = "inetd")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "inetd")))]
    Stdio,
}

impl Display for SomeSocketAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SomeSocketAddr::Tcp(x) => x.fmt(f),
            #[cfg(all(feature = "unix", unix))]
            SomeSocketAddr::Unix(_x) => "unix".fmt(f),
            #[cfg(feature = "inetd")]
            SomeSocketAddr::Stdio => "stdio".fmt(f),
        }
    }
}

impl SomeSocketAddr {
    /// Convert this address representation into a clonable form.
    /// For UNIX socket addresses, it converts them to a string using Debug representation.
    #[must_use] pub fn clonable(self) -> SomeSocketAddrClonable {
        match self {
            SomeSocketAddr::Tcp(x) => SomeSocketAddrClonable::Tcp(x),
            #[cfg(all(feature = "unix", unix))]
            SomeSocketAddr::Unix(x) => SomeSocketAddrClonable::Unix(Arc::new(x)),
            #[cfg(feature = "inetd")]
            SomeSocketAddr::Stdio => SomeSocketAddrClonable::Stdio,
        }
    }
}

/// Other representation of [`SomeSocketAddr`] with Arc-wrapped Unix addresses to enable cloning
#[derive(Debug, Clone)]
#[non_exhaustive]
#[allow(missing_docs)]
pub enum SomeSocketAddrClonable {
    Tcp(SocketAddr),
    #[cfg(all(feature = "unix", unix))]
    #[cfg_attr(docsrs_alt, doc(cfg(all(feature = "unix", unix))))]
    Unix(Arc<tokio::net::unix::SocketAddr>),
    #[cfg(feature = "inetd")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "inetd")))]
    Stdio,
}

impl Display for SomeSocketAddrClonable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SomeSocketAddrClonable::Tcp(x) => x.fmt(f),
            #[cfg(all(feature = "unix", unix))]
            SomeSocketAddrClonable::Unix(_x) => write!(f, "unix:{_x:?}"),
            #[cfg(feature = "inetd")]
            SomeSocketAddrClonable::Stdio => "stdio".fmt(f),
        }
    }
}

impl From<SomeSocketAddr> for SomeSocketAddrClonable {
    fn from(value: SomeSocketAddr) -> Self {
        value.clonable()
    }
}


#[cfg(feature = "hyper014")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "hyper014")))]
mod hyper014 {
    use std::{
        pin::Pin,
        task::{self, Poll},
    };

    use hyper014::server::accept::Accept;

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
                }
                Poll::Pending => Poll::Pending,
            }
        }
    }
}

/// Analogue of `axum::serve` module, but for tokio-listener.
#[cfg(feature="axum07")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "axum07")))]
pub mod axum07;

#[cfg(feature = "tonic010")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "tonic010")))]
mod tonic010 {
    use tonic::transport::server::{Connected, TcpConnectInfo, UdsConnectInfo};

    use crate::Connection;

    #[derive(Clone)]
    pub enum ListenerConnectInfo {
        Tcp(TcpConnectInfo),
        Unix(UdsConnectInfo),
        Stdio,
        Other,
    }

    impl Connected for Connection {
        type ConnectInfo = ListenerConnectInfo;

        fn connect_info(&self) -> Self::ConnectInfo {
            if let Some(tcp_stream) = self.try_borrow_tcp() {
                return ListenerConnectInfo::Tcp(tcp_stream.connect_info());
            }
            #[cfg(all(feature = "unix", unix))]
            if let Some(unix_stream) = self.try_borrow_unix() {
                return ListenerConnectInfo::Unix(unix_stream.connect_info())
            }
            #[cfg(feature = "inetd")]
            if self.try_borrow_stdio().is_some() {
                return ListenerConnectInfo::Stdio;
            }

            ListenerConnectInfo::Other
        }
    }
}

#[cfg(feature="tokio-util")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "tokio-util")))]
mod tokioutil {
    use crate::SomeSocketAddr;

    impl tokio_util::net::Listener for crate::Listener {
        type Io = crate::Connection;

        type Addr = SomeSocketAddr;

        fn poll_accept(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<(Self::Io, Self::Addr)>> {
            self.poll_accept(cx)
        }

        fn local_addr(&self) -> std::io::Result<Self::Addr> {
            match &self.i {
                crate::ListenerImpl::Tcp { s, .. } => Ok(SomeSocketAddr::Tcp(s.local_addr()?)),
                #[cfg(all(feature = "unix", unix))]
                crate::ListenerImpl::Unix { s, .. } =>  Ok(SomeSocketAddr::Unix(s.local_addr()?)),
                #[cfg(feature = "inetd")]
                crate::ListenerImpl::Stdio(_) => Ok(SomeSocketAddr::Stdio),
            }
        }
    }
}
