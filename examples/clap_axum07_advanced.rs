use axum07::{
    extract::ConnectInfo,
    routing::{get, post},
};
use clap::Parser;

#[derive(Parser)]
/// Demo application for tokio-listener
struct Args {
    #[clap(flatten)]
    listener: tokio_listener::ListenerAddressPositional,
}

async fn help() -> &'static str {
    "/conninfo - reply with connection info
/quit - stop server\n"
}

async fn conninfo(
    ConnectInfo(addr): ConnectInfo<tokio_listener::SomeSocketAddrClonable>,
) -> String {
    format!("You are connected from {addr}\n")
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let listener: tokio_listener::Listener = args.listener.bind().await?;

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

    let shutdown = move || async move {
        let _ = shutdown_tx.send(()).await;
    };

    let app = axum07::Router::new()
        .route("/", get(help))
        .route("/conninfo", get(conninfo))
        .route("/quit", post(shutdown));

    tokio_listener::axum07::serve(
        listener,
        app.into_make_service_with_connect_info::<tokio_listener::SomeSocketAddrClonable>(),
    )
    .with_graceful_shutdown(async move { let _ = shutdown_rx.recv().await; })
    .await?;

    Ok(())
}
