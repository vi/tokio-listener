#![cfg_attr(docsrs_alt, feature(doc_cfg))]
#![warn(missing_docs)]
#![allow(clippy::useless_conversion, clippy::module_name_repetitions)]
//! Library for abstracting over TCP server sockets, UNIX server sockets, inetd-like mode.
//!
//! [`ListenerAddress`] is like `SocketAddr` and [`Listener`] is like `TcpListener`, but with more flexibility.
//!
//! ```,no_run
//! # tokio_test::block_on(async {
//! # use tokio_listener::*;
//! let addr1 : ListenerAddress = "127.0.0.1:8087".parse().unwrap();
//! let addr2 : ListenerAddress = "/path/to/socket".parse().unwrap();
//! let addr3 : ListenerAddress = "@abstract_linux_address".parse().unwrap();
//!
//! let system_options : SystemOptions = Default::default();
//! let user_options : UserOptions = Default::default();
//!
//! let mut l = Listener::bind(&addr1, &system_options, &user_options).await.unwrap();
//! while let Ok((conn, addr)) = l.accept().await {
//!     // ...
//! }
//! # });
//! ```
//!
//!  There is special integration with `clap`:
//!
//! ```,no_run
//! # #[cfg(feature="clap")] {
//! use clap::Parser;
//!
//! #[derive(Parser)]
//! /// Demo application for tokio-listener
//! struct Args {
//!     #[clap(flatten)]
//!     listener: tokio_listener::ListenerAddressPositional,
//! }
//!
//! # tokio_test::block_on(async {
//! let args = Args::parse();
//!
//! let listener = args.listener.bind().await.unwrap();
//!
//! let app = axum06::Router::new().route("/", axum06::routing::get(|| async { "Hello, world\n" }));
//!
//! axum06::Server::builder(listener).serve(app.into_make_service()).await;
//! # }) }
//! ```
//!
//! See project [README](https://github.com/vi/tokio-listener/blob/main/README.md) for details and more examples.
//!
//! ## Feature flags
#![doc = document_features::document_features!()]
//!
//! Disabling default features bring tokio-listener close to usual `TcpListener`.

#![cfg_attr(not(feature = "sd_listen"), deny(unsafe_code))]
#![cfg_attr(
    not(feature = "default"),
    allow(unused_imports, irrefutable_let_patterns, unused_variables)
)]

mod connection;
mod error;
mod listener;
mod listener_address;
mod options;
mod some_socket_addr;
mod tcp_keepalive_params;
mod unix_chmod;

#[cfg(feature = "unix_path_tools")]
#[doc(inline)]
pub use unix_chmod::UnixChmodVariant;

#[cfg(feature = "socket_options")]
#[doc(inline)]
pub use tcp_keepalive_params::TcpKeepaliveParams;

#[doc(inline)]
pub use options::{SystemOptions, UserOptions};

#[doc(inline)]
pub use listener_address::ListenerAddress;

#[doc(inline)]
pub use listener::Listener;

#[allow(unused_imports)]
pub(crate) use listener::is_connection_error;

#[doc(inline)]
pub use connection::{Connection, AsyncReadWrite};

#[doc(inline)]
pub use some_socket_addr::{SomeSocketAddr, SomeSocketAddrClonable};

#[cfg(feature = "clap")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "clap")))]
mod claptools;

#[cfg(feature = "clap")]
pub use claptools::{ListenerAddressLFlag, ListenerAddressPositional};

#[cfg(feature = "hyper014")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "hyper014")))]
mod hyper014;

/// Analogue of `axum::serve` module, but for tokio-listener.
#[cfg(feature = "axum07")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "axum07")))]
pub mod axum07;

#[cfg(feature = "axum08")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "axum08")))]
mod axum08;


#[cfg(feature = "tonic010")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "tonic010")))]
mod tonic010;

#[cfg(feature = "tonic011")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "tonic011")))]
mod tonic011;

#[cfg(feature = "tonic012")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "tonic012")))]
mod tonic012;

#[cfg(feature = "tokio-util")]
#[cfg_attr(docsrs_alt, doc(cfg(feature = "tokio-util")))]
mod tokioutil;

#[doc(inline)]
pub use error::{AcceptError, BindError};
