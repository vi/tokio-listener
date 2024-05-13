use std::{fmt::Display, net::SocketAddr, path::PathBuf, str::FromStr};


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
/// let addr : ListenerAddress = "sd-listen:named_socket".parse().unwrap();
/// let addr : ListenerAddress = "sd-listen:*".parse().unwrap();
/// ```
#[non_exhaustive]
#[cfg_attr(
    feature = "serde",
    derive(serde_with::DeserializeFromStr, serde_with::SerializeDisplay)
)]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ListenerAddress {
    /// Usual server TCP socket. Triggered by specifying IPv4 or IPv6 address and port pair.
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
    /// "inetd" or "Accept=yes" mode where stdin and stdout (file descriptors 0 and 1) are used together as a socket
    /// and only one connection is served. Triggered by using `inetd` or `stdio` or `-` as the address.
    Inetd,
    /// SystemD's "Accept=no" mode - using manually specified file descriptor as a pre-created server socket ready to accept TCP or UNIX connections.
    /// Triggered by specifying `sd-listen` as address, which sets `3` as file descriptor number.
    FromFd(i32),
    /// SystemD's "Accept=no" mode - relying on `LISTEN_FDNAMES` environment variable instead of using the hard coded number
    /// Triggered by using appending a colon and a name after `sd-listen`. Example: `sd-listen:mynamedsock`
    /// 
    /// Special name `*` means to bind all passed addresses simultaneously, if `multi-listener` crate feature is enabled.
    FromFdNamed(String),
}


pub(crate) const SD_LISTEN_FDS_START: i32 = 3;

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
            Ok(ListenerAddress::FromFd(SD_LISTEN_FDS_START))
        } else if s.eq_ignore_ascii_case("sd-listen-unix")
            || s.eq_ignore_ascii_case("sd_listen_unix")
        {
            Ok(ListenerAddress::FromFd(SD_LISTEN_FDS_START))
        // No easy `strip_prefix_ignore_ascii_case` in Rust stdlib,
        // so this specific variant is not reachable for upper case end users as well.
        } else if let Some(x) = s.strip_prefix("sd-listen:").or(s.strip_prefix("sd_listen:")) {
            if x.contains(':') {
                return Err("Invalid tokio-listener sd-listen: name");
            }
            Ok(ListenerAddress::FromFdNamed(x.to_owned()))
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
            ListenerAddress::FromFd(fd) => {
                if *fd == SD_LISTEN_FDS_START {
                    "sd-listen".fmt(f)
                } else {
                    write!(f, "accept-from-fd:{fd}")
                }
            }
            ListenerAddress::FromFdNamed(name) => {
                write!(f, "sd-listen:{name}")
            }
        }
    }
}


#[cfg(feature = "sd_listen")]
// based on https://docs.rs/sd-notify/0.4.1/src/sd_notify/lib.rs.html#164, but simplified
#[allow(unused)]
pub(crate) fn check_env_for_fd(fdnum: i32) -> Option<()> {
    use tracing::{debug, error};

    let listen_pid = std::env::var("LISTEN_PID").ok()?;
    let listen_pid: u32 = listen_pid.parse().ok()?;

    let listen_fds = std::env::var("LISTEN_FDS").ok()?;
    let listen_fds: i32 = listen_fds.parse().ok()?;

    debug!("Parsed LISTEN_PID and LISTEN_FDS");

    if listen_pid != std::process::id() {
        error!(expected = %std::process::id(), actual=listen_pid, "Failed LISTEN_PID check");
        return None;
    }

    if fdnum < SD_LISTEN_FDS_START || fdnum >= SD_LISTEN_FDS_START.checked_add(listen_fds)? {
        error!(fdnum, listen_fds, "Failed LISTEN_FDS check");
        return None;
    }

    Some(())
}
