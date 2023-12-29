use std::convert::Infallible;

use argh::FromArgs;
use hyper014::{
    service::{make_service_fn, service_fn},
    Body, Request, Response,
};
use tokio_listener::TcpKeepaliveParams;

/// Small http service app to demonstrate tokio-listener
#[derive(FromArgs)]
struct Args {
    // TCP socket address, UNIX socket file path or @-prefixed abstract name, `-` or `sd-listen` or `sd-listen-unix`.
    #[argh(positional)]
    listen_address: tokio_listener::ListenerAddress,

    /// remove UNIX socket prior to binding to it
    #[argh(switch)]
    unix_listen_unlink: bool,

    /// change filesystem mode of the newly bound UNIX socket to `owner` (006), `group` (066) or `everybody` (666)
    #[argh(option)]
    unix_listen_chmod: Option<tokio_listener::UnixChmodVariant>,

    /// change owner user of the newly bound UNIX socket to this numeric uid
    #[argh(option)]
    unix_listen_uid: Option<u32>,

    /// change owner group of the newly bound UNIX socket to this numeric uid
    #[argh(option)]
    unix_listen_gid: Option<u32>,

    /// ignore environment variables like LISTEN_PID or LISTEN_FDS and unconditionally use file descritor `3` as a socket in
    /// sd-listen or sd-listen-unix modes
    #[argh(switch)]
    sd_accept_ignore_environment: bool,

    /// set SO_KEEPALIVE settings for each accepted TCP connection.
    /// 
    /// Value is a colon-separated triplet of time_ms:count:interval_ms, each of which is optional.
    #[argh(option)]
    tcp_keepalive : Option<TcpKeepaliveParams>,
    

    /// try to set SO_REUSEPORT, so that multiple processes can accept connections from the same port in a round-robin fashion
    #[argh(switch)]
    tcp_reuse_port : bool,

    /// set socket's SO_RCVBUF value
    #[argh(option)]
    recv_buffer_size  : Option<usize>,
    /// set socket's SO_SNDBUF value
    #[argh(option)]
    send_buffer_size : Option<usize>,

    /// set socket's IPV6_V6ONLY to true, to avoid receiving IPv4 connections on IPv6 socket
    #[argh(switch)]
    tcp_only_v6: bool,

    /// maximum number of pending unaccepted connections
    #[argh(option)]
    tcp_listen_backlog : Option<u32>,

    /// text to return in all requests
    #[argh(positional)]
    text: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args: Args = argh::from_env();

    let sopts = tokio_listener::SystemOptions::default();

    let mut uopts = tokio_listener::UserOptions::default();
    uopts.unix_listen_unlink = args.unix_listen_unlink;
    uopts.unix_listen_chmod = args.unix_listen_chmod;
    uopts.unix_listen_uid = args.unix_listen_uid;
    uopts.unix_listen_gid = args.unix_listen_gid;
    uopts.sd_accept_ignore_environment = args.sd_accept_ignore_environment;
    uopts.tcp_keepalive = args.tcp_keepalive;
    uopts.tcp_reuse_port = args.tcp_reuse_port;
    uopts.recv_buffer_size = args.recv_buffer_size;
    uopts.send_buffer_size = args.send_buffer_size;
    uopts.tcp_only_v6 = args.tcp_only_v6;
    uopts.tcp_listen_backlog = args.tcp_listen_backlog;

    let listener = tokio_listener::Listener::bind(&args.listen_address, &sopts, &uopts).await?;

    let make_svc = make_service_fn(move |_| {
        let text = args.text.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |_: Request<Body>| {
                let text = text.clone();
                async move { Ok::<_, Infallible>(Response::new(Body::from(text.clone()))) }
            }))
        }
    });

    hyper014::server::Server::builder(listener)
        .serve(make_svc)
        .await?;

    Ok(())
}
