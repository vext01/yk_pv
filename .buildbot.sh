#!/bin/sh

set -e

case ${STD_TRACER_MODE} in
    "sw" | "hw" ) true;
    *) echo "STD_TRACER_MODE must be set to either 'hw' or 'sw'"
       exit 1;;
esac

RUSTFLAGS="${RUSTFLAGS} -D warnings"

# Use the most recent successful ykrustc build.
tar jxf /opt/ykrustc-bin-snapshots/ykrustc-${STD_TRACER_MODE}-stage2-latest.tar.bz2
export PATH=`pwd`/ykrustc-stage2-latest/bin:${PATH}

cargo xtask test

export CARGO_HOME="`pwd`/.cargo"
export RUSTUP_HOME="`pwd`/.rustup"
export RUSTUP_INIT_SKIP_PATH_CHECK="yes"
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs > rustup.sh
sh rustup.sh --default-host x86_64-unknown-linux-gnu \
    --default-toolchain nightly \
    --no-modify-path \
    --profile minimal \
    -y
export PATH=${CARGO_HOME}/bin/:$PATH
rustup toolchain install nightly --allow-downgrade --component rustfmt
# FIXME add fmt support to xtask build system.
cargo +nightly fmt --all -- --check
cd internal_ws && cargo +nightly fmt --all -- --check
