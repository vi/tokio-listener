#[cfg(all(feature = "unix", unix))]
use tonic_010::transport::server::UdsConnectInfo;
use tonic_010::transport::server::{Connected, TcpConnectInfo};

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
        #[cfg(feature = "inetd")]
        if self.try_borrow_stdio().is_some() {
            return ListenerConnectInfo::Stdio;
        }
        #[cfg(feature = "vsock")]
        if let Some(vsock) = self.try_borrow_vsock() {
            return ListenerConnectInfo::Vsock(vsock.connect_info())
        }

        ListenerConnectInfo::Other
    }
}
