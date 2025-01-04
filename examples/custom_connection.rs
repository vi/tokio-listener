use std::pin::Pin;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let test_io = tokio_test::io::Builder::new().read(b"qqq").write(b"www").build();
    let boxed : Pin<Box<dyn tokio_listener::AsyncReadWrite + Send>> = Box::pin(test_io);
    let mut connection : tokio_listener::Connection = boxed.into();

    let mut content = vec![0; 3];
    connection.read_exact(&mut content).await.unwrap();

    assert_eq!(content, b"qqq");

    connection.write_all(b"www").await.unwrap();


}
