// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `tests/capi/lib.rs`.

#[allow(unused_imports)]
use super::*;


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
