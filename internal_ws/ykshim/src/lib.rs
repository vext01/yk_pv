//! API to the optimised internals of the JIT runtime.
//!
//! Note that the crate is compiled with the abort strategy and that it is therefore not necessary
//! to guard against panics over the C ABI boundary.

/// The production API.
mod prod_api;

/// The testing API.
/// These functions are only exposed to allow testing from the external workspace.
mod test_api;
