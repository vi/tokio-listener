use serde::Deserialize;
use tokio::io::AsyncWriteExt;

#[derive(Deserialize, Debug)]
struct Opts {
    addr: tokio_listener::ListenerAddress,
    #[serde(flatten)]
    uopts: tokio_listener::UserOptions,
}

const TEXT: &str = r#"
  addr = "127.0.0.1:1234"

  # example of some inner option, does not affect anything in this case
  sd_accept_ignore_environment = true
"#;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let opts: Opts = toml::from_str(TEXT)?;

    //eprintln!("{opts:#?}");

    let addr = opts.addr;

    let mut listener = tokio_listener::Listener::bind(
        &addr,
        &tokio_listener::SystemOptions::default(),
        &opts.uopts,
    )
    .await?;

    eprintln!("Listening {addr}");
    while let Ok((conn, addr)) = listener.accept().await {
        eprintln!("Incoming connection from {addr}");
        tokio::spawn(async move {
            let (mut r, mut w) = tokio::io::split(conn);
            let _ = tokio::io::copy(&mut r, &mut w).await;
            let _ = w.shutdown().await;
        });
    }


    Ok(())
}
