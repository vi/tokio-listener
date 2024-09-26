use tonic_012::transport::server::{Connected, TcpConnectInfo};

#[cfg(all(feature = "unix", unix))]
use tonic_012::transport::server::UdsConnectInfo;

use crate::Connection;

#[derive(Clone)]
pub enum ListenerConnectInfo {
    Tcp(TcpConnectInfo),
    #[cfg(all(feature = "unix", unix))]
    #[cfg_attr(docsrs_alt, doc(cfg(all(feature = "unix", unix))))]
    Unix(UdsConnectInfo),
    #[cfg(feature = "inetd")]
    #[cfg_attr(docsrs_alt, doc(cfg(feature = "inetd")))]
    Stdio,
    #[cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))]
    Vsock(vsock_support::VsockConnectInfo),
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
            return ListenerConnectInfo::Unix(unix_stream.connect_info());
        }
        #[cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))]
        if let Some(vsock_stream) = self.try_borrow_vsock() {
            return ListenerConnectInfo::Vsock(vsock_support::VsockConnectInfo::new(vsock_stream));
        }
        #[cfg(feature = "inetd")]
        if self.try_borrow_stdio().is_some() {
            return ListenerConnectInfo::Stdio;
        }

        ListenerConnectInfo::Other
    }
}

#[cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))]
mod vsock_support {
    use tokio_vsock::{VsockAddr, VsockStream};

    /// Connection info for a Vsock Stream.
    ///
    /// See [`Connected`] for more details.
    ///
    #[derive(Debug, Clone, Eq, PartialEq)]
    pub struct VsockConnectInfo {
        peer_addr: Option<VsockAddr>,
    }

    impl VsockConnectInfo {
        /// Return the remote address the IO resource is connected too.
        pub fn peer_addr(&self) -> Option<VsockAddr> {
            self.peer_addr
        }

        pub(crate) fn new(vs: &VsockStream) -> Self {
            Self {
                peer_addr: vs.peer_addr().ok(),
            }
        }
    }
}
