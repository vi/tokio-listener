#!/bin/bash

set -e

true ${S:="target/q/debug/examples/clap_axum07"}

cleanup() {
        local pids=$(jobs -pr)
        [ -n "$pids" ] && kill $pids
}
trap "cleanup" INT QUIT TERM EXIT

$S 127.0.0.1:8080   $'Hello from usual mode\n'&
$S --unix-listen-unlink ./path_socket    $'Hello from UNIX socket path mode\n'&
$S @abstract_socket $'Hello from UNIX socket abstract mode\n'&
systemd-socket-activate          -l 8081 $S   sd-listen   $'Hello from pre-listened socket\n'&
systemd-socket-activate --inetd -al 8082 $S   inetd       $'Hello from inetd mode\n'&

sleep 0.2

curl http://127.0.0.1:8080/
curl http://127.0.0.1:8081/
curl http://127.0.0.1:8082/
curl --unix ./path_socket http://127.0.0.1:8080/
curl --abstract-unix abstract_socket http://127.0.0.1:8080/
