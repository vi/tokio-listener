#[cfg(feature = "tonic010")]
use tonic_010 as tonic;
#[cfg(feature = "tonic011")]
use tonic_011 as tonic;
#[cfg(feature = "tonic012")]
use tonic_012 as tonic;
#[cfg(feature = "tonic013")]
use tonic_013 as tonic;

use tonic::transport::server::{Connected, TcpConnectInfo};

#[cfg(all(feature = "unix", unix))]
use tonic::transport::server::UdsConnectInfo;

#[cfg(all(any(target_os = "linux", target_os = "android", target_os = "macos"), feature = "vsock"))]
use tokio_vsock::VsockConnectInfo;

use crate::Connection;

#[non_exhaustive]
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
    Vsock(VsockConnectInfo),
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
            return ListenerConnectInfo::Vsock(vsock_stream.connect_info());
        }
        #[cfg(feature = "inetd")]
        if self.try_borrow_stdio().is_some() {
            return ListenerConnectInfo::Stdio;
        }

        ListenerConnectInfo::Other
    }
}
