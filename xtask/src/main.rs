//! Custom build system for the Yorick meta-tracer.
//!
//! This is required because we need to separately compile parts of the codebase with different
//! configurations.
//!
//!  - Interpreter code, that we expect to be able to trace, needs to be compiled with `-C
//!    tracer=hw`. Crucially, since hardware tracing relies on blocks not being reordered by LLVM,
//!    this flag disables optimisation.
//!
//!  - Anything that we don't need to trace, for example the JIT internals themselves, can (and
//!    should) be optimised.
//!
//! Note that the above definitions also extend to dependencies, and this means that we can have
//! two copies of any given crate but compiled with different flags. This is good, as it means that
//! the dependencies of interpreter code can be traced, whereas dependencies of the JIT internals
//! can be optimised.
//!
//! To this end, we have 2 Rust workspaces in the repo: the "internal" (optimised) workspace, and
//! the "external" (unoptimised) workspace. The external workspace then talks to the internal
//! workspace via `extern` functions defined in the ykshim crate.
//!
//! There are a few of implementation details to note:
//!
//!  - Code traced as part of testing needs to reside in the external workspace.
//!
//!  - Although we build both workspaces with the same compiler, to avoid potential ABI-related
//!    issues (where adding a `-C` flag to the `rustc` invocation could result in ABI skew), the
//!    workspaces communicate via the C ABI.
//!
//! - Similarly, unless explicitly safe (e.g. `std::ffi` types, or `#[repr(C)]` types), we
//!   shouldn't assume that types with the same definition have the same layout in both workspaces.
//!   It is however, always safe for one workspace to give the other an opaque pointer to an
//!   instance of some type as long as the receiving workspace never tries to interpret the value
//!   as anything but an opaque pointer.
//!
//! - Due to the separate compilation of the workspaces, some code will be duplicated. To avoid
//!   collisions of unmangled symbols, the internal workspace is compiled into a shared object.

// FIXME make `cargo xtask fmt` and `cargo audit` work.

use std::{env, path::PathBuf, process::Command};

fn main() {
    let mut args = env::args().skip(1);
    let target = args.next().unwrap();
    let extra_args = args.collect::<Vec<_>>();
    let cargo = env::var("CARGO").unwrap();
    let rflags = env::var("RUSTFLAGS").unwrap_or_else(|_| String::new());

    // Change into the internal workspace.
    let this_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let int_dir = [&this_dir, "..", "internal_ws"].iter().collect::<PathBuf>();
    env::set_current_dir(&int_dir).unwrap();

    let build_internal = |target: &str, with_extra_args: bool| {
        let mut int_rflags = rflags.clone();
        int_rflags.push_str(" --cfg tracermode=\"hw\"");
        let mut cmd = Command::new(&cargo);
        cmd.arg(&target).arg("--release");
        if with_extra_args {
            cmd.args(&extra_args);
        }
        let status = cmd
            .env_remove("RUSTFLAGS")
            .env("RUSTFLAGS", int_rflags)
            .spawn()
            .unwrap()
            .wait()
            .unwrap();

        if !status.success() {
            panic!("internal build failed");
        }
    };

    eprintln!("Building internal (optimised) workspace...");
    if target == "test" {
        // FIXME
        // Running `cargo xtask test` won't rebuild ykshim when it has changed, so force it.
        build_internal("build", false);
    }
    build_internal(&target, true);

    let mut ext_rflags = rflags;
    ext_rflags.push_str(" -C tracer=hw");

    eprintln!("Building external (unoptimised) workspace...");
    let ext_dir = [&this_dir, ".."].iter().collect::<PathBuf>();
    let int_target_dir = [int_dir.to_str().unwrap(), "target", "release"]
        .iter()
        .collect::<PathBuf>();
    env::set_current_dir(ext_dir).unwrap();
    let status = Command::new(cargo)
        .arg(&target)
        .args(&extra_args)
        .env_remove("RUSTFLAGS")
        .env("RUSTFLAGS", ext_rflags)
        .env("LD_LIBRARY_PATH", int_target_dir)
        .spawn()
        .unwrap()
        .wait()
        .unwrap();

    if !status.success() {
        panic!("external build failed");
    }
}
