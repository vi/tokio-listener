use std::{
    convert::Infallible,
    future::{poll_fn, IntoFuture},
    io,
    marker::PhantomData,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use axum07::{
    body::Body,
    extract::{connect_info::Connected, Request},
    response::Response,
};
use futures_util::{pin_mut, FutureExt};
use hyper1::body::Incoming;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
};
use std::future::Future;
use tokio::sync::watch;
use tower::{util::Oneshot, ServiceExt};
use tower_service::Service;
use tracing::trace;

use crate::{is_connection_error, SomeSocketAddr, SomeSocketAddrClonable};

/// An incoming stream.
///
/// Used with [`serve`] and [`IntoMakeServiceWithConnectInfo`].
///
/// [`IntoMakeServiceWithConnectInfo`]: axum07::extract::connect_info::IntoMakeServiceWithConnectInfo
#[derive(Debug)]
pub struct IncomingStream<'a> {
    tcp_stream: &'a TokioIo<crate::Connection>,
    remote_addr: SomeSocketAddr,
}

impl IncomingStream<'_> {
    /// Returns the local address that this stream is bound to.
    pub fn local_addr(&self) -> std::io::Result<SomeSocketAddr> {
        let q = self.tcp_stream.inner();
        if let Some(a) = q.try_borrow_tcp() {
            return Ok(SomeSocketAddr::Tcp(a.local_addr()?));
        }
        #[cfg(all(feature = "unix", unix))]
        if let Some(a) = q.try_borrow_unix() {
            return Ok(SomeSocketAddr::Unix(a.local_addr()?));
        }
        #[cfg(feature = "inetd")]
        if let Some(_) = q.try_borrow_stdio() {
            return Ok(SomeSocketAddr::Stdio);
        }
        Err(std::io::Error::other(
            "unhandled tokio-listener address type",
        ))
    }

    /// Returns the remote address that this stream is bound to.
    pub fn remote_addr(&self) -> SomeSocketAddrClonable {
        self.remote_addr.clonable()
    }
}

impl Connected<IncomingStream<'_>> for SomeSocketAddrClonable {
    fn connect_info(target: IncomingStream<'_>) -> Self {
        target.remote_addr()
    }
}

/// Future returned by [`serve`].
pub struct Serve<M, S> {
    tokio_listener: crate::Listener,
    make_service: M,
    _marker: PhantomData<S>,
}

/// Serve the service with the supplied `tokio_listener`-based listener.
///
/// See [`axum07::serve::serve`] for more documentation.
/// 
/// See the following examples in `tokio_listener` project:
/// 
/// * [`clap_axum07.rs`](https://github.com/vi/tokio-listener/blob/main/examples/clap_axum07.rs) for simple example
/// * [`clap_axum07_advanced.rs`](https://github.com/vi/tokio-listener/blob/main/examples/clap_axum07_advanced.rs) for using incoming connection info and graceful shutdown.
pub fn serve<M, S>(tokio_listener: crate::Listener, make_service: M) -> Serve<M, S>
where
    M: for<'a> Service<IncomingStream<'a>, Error = Infallible, Response = S>,
    S: Service<Request, Response = Response, Error = Infallible> + Clone + Send + 'static,
    S::Future: Send,
{
    Serve {
        tokio_listener: tokio_listener,
        make_service,
        _marker: PhantomData,
    }
}

impl<M, S> IntoFuture for Serve<M, S>
where
    M: for<'a> Service<IncomingStream<'a>, Error = Infallible, Response = S> + Send + 'static,
    for<'a> <M as Service<IncomingStream<'a>>>::Future: Send,
    S: Service<Request, Response = Response, Error = Infallible> + Clone + Send + 'static,
    S::Future: Send,
{
    type Output = io::Result<()>;
    type IntoFuture = private::ServeFuture;

    fn into_future(self) -> Self::IntoFuture {
        private::ServeFuture(Box::pin(async move {
            let Self {
                mut tokio_listener,
                mut make_service,
                _marker: _,
            } = self;

            loop {
                let (tcp_stream, remote_addr) =
                    match tokio_listener_accept(&mut tokio_listener).await {
                        Some(conn) => conn,
                        None => continue,
                    };
                let tcp_stream = TokioIo::new(tcp_stream);

                poll_fn(|cx| make_service.poll_ready(cx))
                    .await
                    .unwrap_or_else(|err| match err {});

                let tower_service = make_service
                    .call(IncomingStream {
                        tcp_stream: &tcp_stream,
                        remote_addr,
                    })
                    .await
                    .unwrap_or_else(|err| match err {});

                let hyper_service = TowerToHyperService {
                    service: tower_service,
                };

                tokio::spawn(async move {
                    match Builder::new(TokioExecutor::new())
                        // upgrades needed for websockets
                        .serve_connection_with_upgrades(tcp_stream, hyper_service)
                        .await
                    {
                        Ok(()) => {}
                        Err(_err) => {
                            // This error only appears when the client doesn't send a request and
                            // terminate the connection.
                            //
                            // If client sends one request then terminate connection whenever, it doesn't
                            // appear.
                        }
                    }
                });
            }
        }))
    }
}

mod private {
    use std::{
        future::Future,
        io,
        pin::Pin,
        task::{Context, Poll},
    };

    pub struct ServeFuture(pub(super) futures_util::future::BoxFuture<'static, io::Result<()>>);

    impl Future for ServeFuture {
        type Output = io::Result<()>;

        #[inline]
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            self.0.as_mut().poll(cx)
        }
    }

    impl std::fmt::Debug for ServeFuture {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("ServeFuture").finish_non_exhaustive()
        }
    }
}

#[derive(Debug, Copy, Clone)]
struct TowerToHyperService<S> {
    service: S,
}

impl<S> hyper1::service::Service<Request<Incoming>> for TowerToHyperService<S>
where
    S: tower_service::Service<Request> + Clone,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = TowerToHyperServiceFuture<S, Request>;

    fn call(&self, req: Request<Incoming>) -> Self::Future {
        let req = req.map(Body::new);
        TowerToHyperServiceFuture {
            future: self.service.clone().oneshot(req),
        }
    }
}

#[pin_project::pin_project]
struct TowerToHyperServiceFuture<S, R>
where
    S: tower_service::Service<R>,
{
    #[pin]
    future: Oneshot<S, R>,
}

impl<S, R> Future for TowerToHyperServiceFuture<S, R>
where
    S: tower_service::Service<R>,
{
    type Output = Result<S::Response, S::Error>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().future.poll(cx)
    }
}

async fn tokio_listener_accept(
    listener: &mut crate::Listener,
) -> Option<(crate::Connection, SomeSocketAddr)> {
    match listener.accept().await {
        Ok(conn) => Some(conn),
        Err(e) => {
            if is_connection_error(&e) {
                return None;
            }

            // [From `hyper::Server` in 0.14](https://github.com/hyperium/hyper/blob/v0.14.27/src/server/tcp.rs#L186)
            //
            // > A possible scenario is that the process has hit the max open files
            // > allowed, and so trying to accept a new connection will fail with
            // > `EMFILE`. In some cases, it's preferable to just wait for some time, if
            // > the application will likely close some files (or connections), and try
            // > to accept the connection again. If this option is `true`, the error
            // > will be logged at the `error` level, since it is still a big deal,
            // > and then the listener will sleep for 1 second.
            //
            // hyper allowed customizing this but axum does not.
            tracing::error!("accept error: {e}");
            tokio::time::sleep(Duration::from_secs(1)).await;
            None
        }
    }
}

/// Serve future with graceful shutdown enabled.
pub struct WithGracefulShutdown<M, S, F> {
    tokio_listener: crate::Listener,
    make_service: M,
    signal: F,
    _marker: PhantomData<S>,
}

impl<M, S, F> std::fmt::Debug for WithGracefulShutdown<M, S, F>
where
    M: std::fmt::Debug,
    S: std::fmt::Debug,
    F: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            tokio_listener,
            make_service,
            signal,
            _marker: _,
        } = self;

        f.debug_struct("WithGracefulShutdown")
            .field("tokio_listener", tokio_listener)
            .field("make_service", make_service)
            .field("signal", signal)
            .finish()
    }
}

impl<M, S> std::fmt::Debug for Serve<M, S>
where
    M: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            tokio_listener,
            make_service,
            _marker: _,
        } = self;

        f.debug_struct("Serve")
            .field("tokio_listener", tokio_listener)
            .field("make_service", make_service)
            .finish()
    }
}

impl<M, S, F> IntoFuture for WithGracefulShutdown<M, S, F>
where
    M: for<'a> Service<IncomingStream<'a>, Error = Infallible, Response = S> + Send + 'static,
    for<'a> <M as Service<IncomingStream<'a>>>::Future: Send,
    S: Service<Request, Response = Response, Error = Infallible> + Clone + Send + 'static,
    S::Future: Send,
    F: Future<Output = ()> + Send + 'static,
{
    type Output = io::Result<()>;
    type IntoFuture = private::ServeFuture;

    fn into_future(self) -> Self::IntoFuture {
        let Self {
            mut tokio_listener,
            mut make_service,
            signal,
            _marker: _,
        } = self;

        let (signal_tx, signal_rx) = watch::channel(());
        let signal_tx = Arc::new(signal_tx);
        tokio::spawn(async move {
            signal.await;
            tracing::trace!("received graceful shutdown signal. Telling tasks to shutdown");
            drop(signal_rx);
        });

        let (close_tx, close_rx) = watch::channel(());

        private::ServeFuture(Box::pin(async move {
            loop {
                let (tcp_stream, remote_addr) = tokio::select! {
                    conn = tokio_listener_accept(&mut tokio_listener) => {
                        match conn {
                            Some(conn) => conn,
                            None => continue,
                        }
                    }
                    _ = signal_tx.closed() => {
                        trace!("signal received, not accepting new connections");
                        break;
                    }
                };
                let tcp_stream = TokioIo::new(tcp_stream);

                trace!("connection {remote_addr} accepted");

                poll_fn(|cx| make_service.poll_ready(cx))
                    .await
                    .unwrap_or_else(|err| match err {});

                let tower_service = make_service
                    .call(IncomingStream {
                        tcp_stream: &tcp_stream,
                        remote_addr,
                    })
                    .await
                    .unwrap_or_else(|err| match err {});

                let hyper_service = TowerToHyperService {
                    service: tower_service,
                };

                let signal_tx = Arc::clone(&signal_tx);

                let close_rx = close_rx.clone();

                tokio::spawn(async move {
                    let builder = Builder::new(TokioExecutor::new());
                    let conn = builder.serve_connection_with_upgrades(tcp_stream, hyper_service);
                    pin_mut!(conn);

                    let signal_closed = signal_tx.closed().fuse();
                    pin_mut!(signal_closed);

                    loop {
                        tokio::select! {
                            result = conn.as_mut() => {
                                if let Err(_err) = result {
                                    trace!("failed to serve connection: {_err:#}");
                                }
                                break;
                            }
                            _ = &mut signal_closed => {
                                trace!("signal received in task, starting graceful shutdown");
                                conn.as_mut().graceful_shutdown();
                            }
                        }
                    }

                    trace!("a connection closed");

                    drop(close_rx);
                });
            }

            drop(close_rx);
            drop(tokio_listener);

            trace!(
                "waiting for {} task(s) to finish",
                close_tx.receiver_count()
            );
            close_tx.closed().await;

            Ok(())
        }))
    }
}

impl<M, S> Serve<M, S> {
    /// Prepares a server to handle graceful shutdown when the provided future completes.
    ///
    /// See [the original documentation][1] for the example.
    /// 
    /// [1]: axum07::serve::Serve::with_graceful_shutdown
    pub fn with_graceful_shutdown<F>(self, signal: F) -> WithGracefulShutdown<M, S, F>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        WithGracefulShutdown {
            tokio_listener: self.tokio_listener,
            make_service: self.make_service,
            signal,
            _marker: PhantomData,
        }
    }
}
