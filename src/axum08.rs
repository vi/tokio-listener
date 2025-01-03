impl axum08::serve::Listener for super::Listener {
    type Io = super::Connection;

    type Addr = super::SomeSocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.accept().await {
            Ok((c,a)) => return (c,a),
            Err(e) => {
                // There is an internal retrier within tokio_listener, so failed accept is not expected to be retriable here
                tracing::error!("Fatal error from tokio_listener::Listener::accept: {e}. Hanging forever.");
                std::future::pending().await
            }
        }
    }

    fn local_addr(&self) -> tokio::io::Result<Self::Addr> {
        crate::Listener::local_addr(&self)
    }
}
