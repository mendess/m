#!/usr/bin/env bash

awk '
features && $1 != "default" && $0 ~ /[a-z]+ =.*/ {
    print "testing feature " $1
    if (system("cargo --quiet test --no-default-features --features " $1) != 0) {
        exit(1)
    }
}
/^\[features\]$/ { features = 1 }
' Cargo.toml
