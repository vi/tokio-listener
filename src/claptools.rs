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
    #[allow(clippy::missing_errors_doc)]
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
