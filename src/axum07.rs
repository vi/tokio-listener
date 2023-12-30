use std::{marker::PhantomData, convert::Infallible, net::SocketAddr, io, future::{IntoFuture, poll_fn}, task::{Context, Poll}, pin::Pin, time::Duration};

use axum07::{extract::{Request, connect_info::Connected}, response::Response, body::Body};
use tower_service::Service;
use std::future::Future;
use hyper1::body::Incoming;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
};
use tokio::net::{TcpListener, TcpStream};
use tower::{util::Oneshot, ServiceExt};
use futures_util::{pin_mut, FutureExt};

use crate::is_connection_error;


/// An incoming stream.
///
/// Used with [`serve`] and [`IntoMakeServiceWithConnectInfo`].
///
/// [`IntoMakeServiceWithConnectInfo`]: crate::extract::connect_info::IntoMakeServiceWithConnectInfo
#[derive(Debug)]
pub struct IncomingStream<'a> {
    tcp_stream: &'a TokioIo<TcpStream>,
    remote_addr: SocketAddr,
}

impl IncomingStream<'_> {
    /// Returns the local address that this stream is bound to.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.tcp_stream.inner().local_addr()
    }

    /// Returns the remote address that this stream is bound to.
    pub fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }
}

impl Connected<IncomingStream<'_>> for SocketAddr {
    fn connect_info(target: IncomingStream<'_>) -> Self {
        target.remote_addr()
    }
}


/// Future returned by [`serve`].
pub struct Serve<M, S> {
    tcp_listener: TcpListener,
    make_service: M,
    _marker: PhantomData<S>,
}

pub fn serve<M, S>(tcp_listener: TcpListener, make_service: M) -> Serve<M, S>
where
    M: for<'a> Service<IncomingStream<'a>, Error = Infallible, Response = S>,
    S: Service<Request, Response = Response, Error = Infallible> + Clone + Send + 'static,
    S::Future: Send,

{
    Serve {
        tcp_listener,
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
                tcp_listener,
                mut make_service,
                _marker: _,
            } = self;

            loop {
                let (tcp_stream, remote_addr) = match tcp_accept(&tcp_listener).await {
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

async fn tcp_accept(listener: &TcpListener) -> Option<(TcpStream, SocketAddr)> {
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
