#!/bin/sh

set -e


if [ "${SOFTDEV_CI}" = "1" ]; then
    echo "XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX"
else
    echo "YYYYYYYYYYYYYYYYYYYYYYYYYYYYYYY"
fi

# Install rustup.
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

# There are some feature-gated testing/debugging switches which slow the JIT
# down a bit. Check that if we build the system without tests, those features
# are not enabled.
for mode in "" "--release"; do \
    cargo -Z unstable-options build ${mode} --build-plan -p ykcapi | \
        awk '/yk_testing/ { ec=1 } /yk_jitstate_debug/ { ec=1 } END {exit ec}'; \
done

cargo fmt --all -- --check

# Check licenses.
which cargo-deny | cargo install cargo-deny
cargo-deny check license

# Build the docs
cargo install mdbook
cd docs
mdbook build
test -d book
cd ..

# Build LLVM for the C tests.
mkdir -p target && cd target
git clone https://github.com/ykjit/ykllvm
cd ykllvm
mkdir build
cd build

# Due to an LLVM bug, PIE breaks our mapper, and it's not enough to pass
# `-fno-pie` to clang for some reason:
# https://github.com/llvm/llvm-project/issues/57085
cmake -DCMAKE_INSTALL_PREFIX=`pwd`/../inst \
    -DLLVM_INSTALL_UTILS=On \
    -DCMAKE_BUILD_TYPE=release \
    -DLLVM_ENABLE_ASSERTIONS=On \
    -DLLVM_ENABLE_PROJECTS="lld;clang" \
    -DCLANG_DEFAULT_PIE_ON_LINUX=OFF \
    -GNinja \
    ../llvm
cmake --build .
cmake --install .
export PATH=`pwd`/../inst/bin:${PATH}
cd ../../..

# Check that clang-format is installed.
clang-format --version
# Check C/C++ formatting using xtask.
cargo xtask cfmt
git diff --exit-code

# Check that building `ykcapi` in isolation works. This is what we'd be doing
# if we were building release binaries, as it would mean we get a system
# without the (slower) `yk_testing` and `yk_jitstate_debug` features enabled.
for mode in "" "--release"; do
    cargo build ${mode} -p ykcapi;
done

if [ "${SOFTDEV_CI}" = "1" ]; then
    TEST_ITERS=100
else
    TEST_ITERS=10
fi

for i in $(seq ${TEST_ITERS}); do
    echo "---> Test iteration ${i}"
    cargo test
    cargo test --release
done

cargo bench

# Run examples.
cargo run --example hwtracer_example
cargo run --release --example hwtracer_example
