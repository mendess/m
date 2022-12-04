#!/usr/bin/env bash

awk '
features && $0 ~ /[a-z]+ =.*/ {
    print "############# testing feature " $1
    if ($1 == "default") {
        cmd = "cargo --quiet test --no-default-features"
    } else {
        cmd = "cargo --quiet test --no-default-features --features " $1
    }
    if (system(cmd) != 0) {
        exit(1)
    }
}
/^\[features\]$/ { features = 1 }
' Cargo.toml
