/// A simple example of how to use tokio-listener with a tonic gRPC server.
/// Keep in mind tokio-listener supports different tonic versions, and
/// version-suffixes them.
/// In your code, the crate would probably just be called "tonic".
use tonic_012::transport::Server;
use tonic_health::server::health_reporter;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let sys_opts = tokio_listener::SystemOptions::default();
    let user_opts = tokio_listener::UserOptions::default();
    let addr = tokio_listener::ListenerAddress::Path("/tmp/test.sock".into());

    let listener = tokio_listener::Listener::bind(&addr, &sys_opts, &user_opts).await?;

    let (_health_reporter, health_server) = health_reporter();

    println!("Listening on {addr:?}");
    Server::builder()
        .add_service(health_server)
        .serve_with_incoming(listener)
        .await?;

    Ok(())
}
