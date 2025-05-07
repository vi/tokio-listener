<a name="v0.5.1"></a>
# v0.5.1

* Add Tonic 0.13 support

<a name="v0.5.0"></a>
# v0.5.0

* Add `AF_VSOCK` support
* Add Axum 0.8 heler
* Additional features for flexibility: `mpsc_listener`, `boxed_variant`, `duplex_variant`, `dummy_variant`, `custom_socket_address`. They are off by default.
* Tonic's connection info enum marked as non_exhausive.
* Fixed unconditional `tokio/io-std` dependency.

<a name="v0.4.4"></a>
# v0.4.4

* Fix `--unix-listen-chmod` behaviour
* Add some `Debug` impls

<a name="v0.4.3"></a>
# v0.4.3

* Add tonic v0.12 support

<a name="v0.4.2"></a>
# v0.4.2

* Fix error message

<a name="v0.4.1"></a>
# v0.4.1

* Fix `axum` being non-optional dependency

<a name="v0.4.0"></a>
# v0.4.0

* `Listener::bind_multiple` and named `sd-listen`.

<a name="v0.3.1"></a>
# v0.3.1 - Fix compilation on Windows

* Fixed cfg gates to make it buildable on Windows (with reduced options, obviously)


<a name="v0.3.0"></a>
# v0.3.0 - Axum 0.7 support

Incompatible changes:

* Removed "hyper014" from default features.

Other changes:

* Added easy integration with Axum 0.7.
* Added implementation of `tokio_util::net::Listener`.
* Added clonable version of `SomeSocketAddr`.
* Improved documentation
* Fixed missing cfg gates resulting in compilation failures with some feature combinations

<a name="v0.2.2"></a>
# v0.2.2 - Base version for Changelog


