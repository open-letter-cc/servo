/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Sets `capi_full` for this crate's build, so that the included `ffi/capi`
//! source leaves out the entry points this crate defines in its own shape.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(capi_full)");
    println!("cargo::rustc-cfg=capi_full");
}
