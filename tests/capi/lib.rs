/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

unsafe extern "C" {
    fn run_c_api_tests() -> i32;
    fn run_c_integration_tests() -> i32;
    fn run_c_page_request_tests() -> i32;
    fn run_c_page_request_pref_off_tests() -> i32;
    fn run_c_permission_tests() -> i32;
    fn run_c_clipboard_tests() -> i32;
    fn run_c_control_origin_tests(port: u16) -> i32;
    fn run_c_frame_events_tests() -> i32;
    fn run_c_read_frame_tests() -> i32;
    fn run_c_resource_request_tests() -> i32;
    fn run_c_resize_tests() -> i32;
}

#[cfg(feature = "multiprocess")]
unsafe extern "C" {
    fn run_c_multiprocess_tests() -> i32;
}

pub fn c_api_tests() {
    let result = unsafe { run_c_api_tests() };
    assert_eq!(result, 0, "C API tests failed");
}

pub fn c_integration_tests() {
    let result = unsafe { run_c_integration_tests() };
    assert_eq!(result, 0, "C integration tests failed");
}

#[path = "../../vendor/open-letter/tests/capi/lib.rs"]
mod open_letter;
#[allow(unused_imports)]
pub use open_letter::*;
