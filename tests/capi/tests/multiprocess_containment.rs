/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// This test supplies its own `main` (see `[[test]]` in Cargo.toml) because the
// content-process re-exec in `components/constellation/sandboxing.rs` relaunches
// `env::current_exe()` with `--content-process <token>` and expects *that exact
// process* to recognise the flag and call `servo_content_process_main`, as any
// library embedder's own binary must. libtest's own generated `main` would reject that
// flag as an unrecognised option before this test ever ran, so this binary checks for
// it itself, first, before anything else runs.

use std::ffi::CString;
use std::os::raw::c_char;

unsafe extern "C" {
    fn servo_content_process_main(token: *const c_char) -> i32;
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "--content-process" {
        let token = CString::new(args[2].clone()).expect("token must not contain NUL");
        let code = unsafe { servo_content_process_main(token.as_ptr()) };
        std::process::exit(code);
    }

    // Freeing the engine after a content process has died is one of the things the
    // C test checks, and a hang there would otherwise stall the run with no verdict.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(300));
        eprintln!("multiprocess containment test did not finish within 300s");
        std::process::exit(3);
    });

    let failed = servo_capi_tests::c_multiprocess_tests();
    std::process::exit(if failed == 0 { 0 } else { 1 });
}
