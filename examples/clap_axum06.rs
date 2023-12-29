use clap::Parser;

#[derive(Parser)]
/// Demo application for tokio-listener
struct Args {
    #[clap(flatten)]
    listener: tokio_listener::ListenerAddressPositional,

    /// Line of text to return as a body of incoming requests
    text_to_serve: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let listener : tokio_listener::Listener = args.listener.bind().await?;

    let app = axum06::Router::new().route("/", axum06::routing::get(|| async { args.text_to_serve }));

    axum06::Server::builder(listener)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}
