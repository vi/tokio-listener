use std::pin::Pin;

use futures_util::FutureExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn create_mock_listener() -> tokio_listener::Listener {
    let (tx, rx) = tokio::sync::mpsc::channel(1);

    let test_io = tokio_test::io::Builder::new()
        .read(b"qqq")
        .write(b"www")
        .build();
    let boxed: Pin<Box<dyn tokio_listener::AsyncReadWrite + Send>> = Box::pin(test_io);
    let connection: tokio_listener::Connection = boxed.into();
    let addr = tokio_listener::SomeSocketAddr::Mpsc;

    tx.send((connection, addr)).now_or_never().unwrap().unwrap();

    rx.into()
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut listener = create_mock_listener();

    let mut connection = listener.accept().await.unwrap().0;

    let mut content = vec![0; 3];
    connection.read_exact(&mut content).await.unwrap();

    assert_eq!(content, b"qqq");

    connection.write_all(b"www").await.unwrap();
}
