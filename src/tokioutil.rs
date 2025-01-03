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
        crate::Listener::local_addr(&self)
    }
}
