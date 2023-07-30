use clap::Parser;

#[derive(Parser)]
struct Args {
    /// Socket address to listen for incoming connections.
    /// May be TCP socket address like `0.0.0.0:80` or other forms like
    /// `/path/to/unix/socket`, `@abstract`, `-` or `sd-listen`.
    listener: tokio_listener::ListenerAddress,

    #[clap(flatten)]
    listener_optiots: tokio_listener::UserOptions,

    text_to_serve: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let listener = tokio_listener::Listener::bind(
        &args.listener,
        &tokio_listener::SystemOptions::default(),
        &args.listener_optiots,
    )
    .await?;

    let app = axum::Router::new().route("/", axum::routing::get(|| async { args.text_to_serve }));

    axum::Server::builder(listener)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}
