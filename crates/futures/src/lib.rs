//! Converting between JavaScript `Promise`s to Rust `Future`s.
//!
//! This crate provides a bridge for working with JavaScript `Promise` types as
//! a Rust `Future`, and similarly contains utilities to turn a rust `Future`
//! into a JavaScript `Promise`. This can be useful when working with
//! asynchronous or otherwise blocking work in Rust (wasm), and provides the
//! ability to interoperate with JavaScript events and JavaScript I/O
//! primitives.
//!
//! There are three main interfaces in this crate currently:
//!
//! 1. [**`JsFuture`**](./struct.JsFuture.html)
//!
//!    A type that is constructed with a `Promise` and can then be used as a
//!    `Future<Output = Result<JsValue, JsValue>>`. This Rust future will resolve
//!    or reject with the value coming out of the `Promise`.
//!
//! 2. [**`future_to_promise`**](./fn.future_to_promise.html)
//!
//!    Converts a Rust `Future<Output = Result<JsValue, JsValue>>` into a
//!    JavaScript `Promise`. The future's result will translate to either a
//!    rejected or resolved `Promise` in JavaScript.
//!
//! 3. [**`spawn_local`**](./fn.spawn_local.html)
//!
//!    Spawns a `Future<Output = ()>` on the current thread. This is the
//!    best way to run a `Future` in Rust without sending it to JavaScript.
//!
//! These three items should provide enough of a bridge to interoperate the two
//! systems and make sure that Rust/JavaScript can work together with
//! asynchronous and I/O work.

//#![cfg_attr(target_feature = "atomics", feature(stdsimd))]
#![deny(missing_docs)]

mod singlethread;
pub use singlethread::*;

mod shared;
pub use shared::*;
