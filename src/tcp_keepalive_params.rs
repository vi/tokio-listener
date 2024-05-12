use std::{fmt::Display, str::FromStr, time::Duration};


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
    fn from_str(input_string: &str) -> Result<Self, Self::Err> {
        let input_chunks: Vec<&str> = input_string.split(':').collect();
        if input_chunks.len() > 3 {
            return Err("Too many colon-separated chunks");
        }
        let timeout_chunk = input_chunks.get(0).unwrap_or(&"").trim();
        let ping_count_chunk = input_chunks.get(1).unwrap_or(&"").trim();
        let interval_chunk = input_chunks.get(2).unwrap_or(&"").trim();

        let mut ka_params = TcpKeepaliveParams::default();

        if !timeout_chunk.is_empty() {
            ka_params.timeout_ms = Some(
                timeout_chunk
                    .parse()
                    .map_err(|_| "failed to parse timeout as a number")?,
            );
        }
        if !ping_count_chunk.is_empty() {
            ka_params.count = Some(
                ping_count_chunk
                    .parse()
                    .map_err(|_| "failed to parse count as a number")?,
            );
        }
        if !interval_chunk.is_empty() {
            ka_params.interval_ms = Some(
                interval_chunk
                    .parse()
                    .map_err(|_| "failed to parse interval as a number")?,
            );
        }

        Ok(ka_params)
    }
}
#[cfg(feature = "socket_options")]
impl TcpKeepaliveParams {
    /// Attempt to convert values of this struct to socket2 format.
    ///
    /// Some fields may be ignored depending on platform.
    #[must_use]
    pub fn to_socket2(&self) -> socket2::TcpKeepalive {
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
