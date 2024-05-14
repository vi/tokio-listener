use crate::listener::{ListenerImpl, ListenerImplTcp};
use crate::SomeSocketAddr;

impl tokio_util::net::Listener for crate::Listener {
    type Io = crate::Connection;

    type Addr = SomeSocketAddr;

    fn poll_accept(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<(Self::Io, Self::Addr)>> {
        self.poll_accept(cx)
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        match &self.i {
            ListenerImpl::Tcp(ListenerImplTcp { s, .. }) => {
                Ok(SomeSocketAddr::Tcp(s.local_addr()?))
            }
            #[cfg(all(feature = "unix", unix))]
            crate::listener::ListenerImpl::Unix(crate::listener::ListenerImplUnix {
                s, ..
            }) => Ok(SomeSocketAddr::Unix(s.local_addr()?)),
            #[cfg(feature = "inetd")]
            crate::listener::ListenerImpl::Stdio(_) => Ok(SomeSocketAddr::Stdio),
            #[cfg(feature = "multi-listener")]
            crate::listener::ListenerImpl::Multi(_) => Ok(SomeSocketAddr::Multiple),
        }
    }
}
