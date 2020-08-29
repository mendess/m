#!/bin/sh
set -e
(
    cd lib
    cargo build --lib --target aarch64-linux-android --release
    cargo build --lib --target armv7-linux-androideabi --release
    cargo build --lib --target i686-linux-android --release
)
