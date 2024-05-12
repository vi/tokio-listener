use std::{
    pin::Pin,
    task::{self, Poll},
};

use hyper014::server::accept::Accept;

use crate::Listener;

impl Accept for Listener {
    type Conn = crate::Connection;
    type Error = std::io::Error;

    fn poll_accept(
        self: std::pin::Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        let nmc = self.no_more_connections();
        match crate::Listener::poll_accept(Pin::<_>::into_inner(self), cx) {
            Poll::Ready(Ok((s, _a))) => Poll::Ready(Some(Ok(s))),
            Poll::Ready(Err(e)) => {
                if nmc {
                    return Poll::Ready(None);
                }
                Poll::Ready(Some(Err(e)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
