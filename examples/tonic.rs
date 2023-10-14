/// A simple example of how to use tokio-listener with a tonic gRPC server.
use tonic::transport::Server;
use tonic_health::server::health_reporter;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let sopts = tokio_listener::SystemOptions::default();
    let uopts = tokio_listener::UserOptions::default();
    let addr = tokio_listener::ListenerAddress::Path("/tmp/test.sock".into());

    let listener = tokio_listener::Listener::bind(&addr, &sopts, &uopts).await?;

    let (_health_reporter, health_server) = health_reporter();

    println!("Listening on {:?}", addr);
    Server::builder()
        .add_service(health_server)
        .serve_with_incoming(listener)
        .await?;

    Ok(())
}
