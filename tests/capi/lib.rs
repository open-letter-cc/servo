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

pub fn c_page_request_tests() {
    let result = unsafe { run_c_page_request_tests() };
    assert_eq!(result, 0, "C page request tests failed");
}

pub fn c_page_request_pref_off_tests() {
    let result = unsafe { run_c_page_request_pref_off_tests() };
    assert_eq!(result, 0, "C page request pref-off tests failed");
}

pub fn c_permission_tests() {
    let result = unsafe { run_c_permission_tests() };
    assert_eq!(result, 0, "C permission tests failed");
}

pub fn c_clipboard_tests() {
    let result = unsafe { run_c_clipboard_tests() };
    assert_eq!(result, 0, "C clipboard tests failed");
}

pub fn c_frame_events_tests() {
    let result = unsafe { run_c_frame_events_tests() };
    assert_eq!(result, 0, "C frame event tests failed");
}

pub fn c_read_frame_tests() {
    let result = unsafe { run_c_read_frame_tests() };
    assert_eq!(result, 0, "C read frame tests failed");
}

pub fn c_resource_request_tests() {
    let result = unsafe { run_c_resource_request_tests() };
    assert_eq!(result, 0, "C resource request tests failed");
}

pub fn c_resize_tests() {
    let result = unsafe { run_c_resize_tests() };
    assert_eq!(result, 0, "C resize tests failed");
}

/// `port` must serve the pages `test_control_origin.c` loads.
pub fn c_control_origin_tests(port: u16) {
    let result = unsafe { run_c_control_origin_tests(port) };
    assert_eq!(result, 0, "C control origin tests failed");
}

#[cfg(feature = "multiprocess")]
pub fn c_multiprocess_tests() -> i32 {
    unsafe { run_c_multiprocess_tests() }
}
