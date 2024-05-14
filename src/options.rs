#[cfg_attr(feature = "clap", derive(clap::Args))]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[allow(clippy::struct_excessive_bools)]
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
    pub unix_listen_chmod: Option<crate::UnixChmodVariant>,

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
    pub tcp_keepalive: Option<crate::TcpKeepaliveParams>,

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
