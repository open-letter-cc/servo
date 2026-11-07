// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Sets `capi_full` for this crate's build, so that the included `ffi/capi`
//! source leaves out the entry points this crate defines in its own shape.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(capi_full)");
    println!("cargo::rustc-cfg=capi_full");
}
