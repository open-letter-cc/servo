/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The full-surface Servo C ABI cdylib.
//!
//! This crate is `ffi/capi` plus the twelve entry points an embedder needs in
//! order to put a `WebView` inside a window it already owns. It is not a fork
//! of `ffi/capi`: the line below includes that crate's own `lib.rs`, so every
//! symbol `servo_capi.dll` exports is exported here too, from the same source,
//! and stays in step with upstream automatically.
//!
//! The reason this is an `include!` and not a dependency is that `ffi/capi` is
//! `crate-type = ["cdylib"]`. A cdylib cannot be a Rust dependency, and the
//! internals the new entry points need - `RenderingContext`'s `inner` field in
//! particular - are `pub(crate)`. Pulling the source into *this* crate root is
//! what makes them reachable without editing `ffi/capi` or `components/`.
//!
//! `include!` rather than `#[path] mod`, because the included file's `mod`
//! items must land at *this* crate's root: `ffi/capi`'s modules refer to each
//! other as `crate::rendering_context`, `crate::webview_delegate` and so on.
//!
//! See `src/native.rs` for the twelve additions and for the ABI they pin.
include!("../../../ffi/capi/lib.rs");

mod native;
