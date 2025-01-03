# Rationale

Web service projects created using Hyper or Axum Rust frameworks typically allow users to specify TCP port and host to bind to in order to listen for incoming connetions.
While it is a solid default choice, sometimes more flexibility is desired, especially on Linux.

`tokio-listener` allows to add this flexibility by offering special abstract types `ListenerAddress` and `Listener` instead of typical `SocketAddr` and `TcpListener`, allowing adding this flexibility to projects in easy way.

# Features

## Listening modes

* Listening UNIX socket instead of TCP socket. It is triggered by beginning the address with `.` or `/`.
* Listening abstract-namespaced UNIX socket on Linux. It is triggered by beginning the address with `@`.
* Accepting connections from a pre-opened socket inherited from parent process (e.g. systemd). This is triggered by special address `sd-listen`. You can also request a specific named or all sockets.
* Inetd mode - stdin/stdout can also be used for serving one connection.

## Other 

* Portability - tricky features are not compiled on platforms which do not support them
* `clap` integration - custom address can be added as a Clap field (due to FromStr impl). Other options can be included by `clap(flatten)`, which also brings short documentation to the CLI help message. Alternatively, the whole set of primary address and additional options can be brought in using `ListenerAddressPositional` or `ListenerAddressLFlag` helper types.
* `serde` intergration - custom address type behaves like a string with respect to Serde. Other options can also be serialized or deserialized.
* For UNIX path sockets, it supports unlinking, chowning and chmodding the file per user request.
* `axum 0.7` integration.
* Multi-listener - easy was to bind to multiple ports simulteneously. Combined with systemd support, also allows to trigger multi-listen using special `sd-listen:*` address. Not enabled by default.
* Fine-grained compile-time feature switches. Without features it should basically reduce to a thin wrapper around plain `TcpListener`.

# Examples

* clap_axum - simplest, most straighforward example. Uses Clap as CLI framework and Axum as web framework. There are multiple versions of the example, for various Axum versions.
* argh_hyper - demonstrages how to use non-clap CLI parser. Also uses `hyper` directly instead of Axum.
* serde_echo - demonstrates that listening configuration can also be specified using e.g. toml file. Is not a web service, but an echo server.

See [crate docs](https://docs.rs/tokio-listener) for API reference and some other examples.

# Limitations

* There is no support of SEQPACKET or DGRAM sockets.
* It may be slower that just using TcpListener directly, as each send or recv needs to go though a wrapper. This slowdown should disappear if you disable UNIX and inetd modes at compilation time.
* Specifying non-UTF8-compatible paths for UNIX sockets is not supported.

# Example session


Given this series of invocations:

```
target/debug/examples/clap_axum07 127.0.0.1:8080   $'Hello from usual mode\n'
target/debug/examples/clap_axum07 ./path_socket    $'Hello from UNIX socket path mode\n'
target/debug/examples/clap_axum07 @abstract_socket $'Hello from UNIX socket abstract mode\n'
systemd-socket-activate          -l 8081 target/debug/examples/clap_axum07   sd-listen   $'Hello from pre-listened socket\n'
systemd-socket-activate --inetd -al 8082 target/debug/examples/clap_axum07   inetd       $'Hello from inetd mode\n'
systemd-socket-activate -l 8083 -l 8084 --fdname=foo:bar -- target/debug/examples/clap_axum07   sd-listen:bar   $'Hello from a named pre-listened socket\n'
systemd-socket-activate -l 8085 -l 8086 -- target/debug/examples/clap_axum07   sd-listen:*   $'Hello from any of the two pre-listened sockets\n'
```

and this [Caddyfile](https://caddyserver.com/):

```
{
    admin off
}
:4000

handle_path /tcp/* {
    reverse_proxy 127.0.0.1:8080
}
handle_path /unix/* {
    reverse_proxy unix/./path_socket
}
handle_path /abstract/* {
    reverse_proxy unix/@abstract_socket
}
handle_path /sdlisten/* {
    reverse_proxy 127.0.0.1:8081
}
handle_path /inetd/* {
    reverse_proxy 127.0.0.1:8082
}
```

you can see that effectively the same service can be accessed in multiple ways:

```
$ curl http://127.0.0.1:4000/tcp/
Hello from usual mode
$ curl http://127.0.0.1:4000/unix/
Hello from UNIX socket path mode
$ curl http://127.0.0.1:4000/abstract/
Hello from UNIX socket abstract mode
$ curl http://127.0.0.1:4000/sdlisten/
Hello from a pre-listened socket
$ curl http://127.0.0.1:4000/inetd/
Hello from inetd 

$ curl --unix ./path_socket http://q/
Hello from UNIX socket path mode
$ curl --abstract-unix abstract_socket http://q/
Hello from UNIX socket abstract mode
```

# Help message of one of the examples

```
Demo application for tokio-listener

Usage: clap_axum [OPTIONS] <LISTEN_ADDRESS> <TEXT_TO_SERVE>

Arguments:
  <LISTEN_ADDRESS>
          Socket address to listen for incoming connections.
          
          Various types of addresses are supported:
          
          * TCP socket address and port, like 127.0.0.1:8080 or [::]:80
          
          * UNIX socket path like /tmp/mysock or Linux abstract address like @abstract
          
          * Special keyword "inetd" for serving one connection from stdin/stdout
          
          * Special keyword "sd-listen" to accept connections from file descriptor 3 (e.g. systemd socket activation).
            You can also specify a named descriptor after a colon or * to use all passed sockets (if this feature is enabled).

  <TEXT_TO_SERVE>
          Line of text to return as a body of incoming requests

Options:
      --unix-listen-unlink
          remove UNIX socket prior to binding to it

      --unix-listen-chmod <UNIX_LISTEN_CHMOD>
          change filesystem mode of the newly bound UNIX socket to `owner`, `group` or `everybody`

      --unix-listen-uid <UNIX_LISTEN_UID>
          change owner user of the newly bound UNIX socket to this numeric uid

      --unix-listen-gid <UNIX_LISTEN_GID>
          change owner group of the newly bound UNIX socket to this numeric uid

      --sd-accept-ignore-environment
          ignore environment variables like LISTEN_PID or LISTEN_FDS and unconditionally use file descritor `3` as a socket in sd-listen or sd-listen-unix modes

      --tcp-keepalive <TCP_KEEPALIVE>
          set SO_KEEPALIVE settings for each accepted TCP connection.

          Value is a colon-separated triplet of time_ms:count:interval_ms, each of which is optional.

      --tcp-reuse-port
          Try to set SO_REUSEPORT, so that multiple processes can accept connections from the same port in a round-robin fashion

      --recv-buffer-size <RECV_BUFFER_SIZE>
          Set socket's SO_RCVBUF value

      --send-buffer-size <SEND_BUFFER_SIZE>
          Set socket's SO_SNDBUF value

      --tcp-only-v6
          Set socket's IPV6_V6ONLY to true, to avoid receiving IPv4 connections on IPv6 socket

      --tcp-listen-backlog <TCP_LISTEN_BACKLOG>
          Maximum number of pending unaccepted connections

  -h, --help
          Print help (see a summary with '-h')

```

All this can be brought in with just one `#[clap(flatten)] addr: tokio_listener::ListenerAddressPositional`.
