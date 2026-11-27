// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `ffi/capi/preferences.rs`.

#[allow(unused_imports)]
use super::*;

// Resource limits for the threads of one engine. Each is a named form of a preference
// that can also be set by name; these document the units and refuse values out of range.

/// Sets the stack size of each script thread, in KiB (preference
/// `thread_script_stack_size_kb`; default 8192). JavaScript may use half of it, and
/// recursing deeper throws a catchable "too much recursion" error rather than
/// overflowing the thread. Takes effect for script threads started afterwards.
/// `value` must be from 1024 to 1048576.
///
/// Returns 0, or -1 if `value` is out of range, leaving the preference unchanged.
///
/// # Safety
///
/// The caller must ensure that `preferences` is a non-null pointer to a
/// `ServoPreferences` previously returned by `servo_preferences_create` and not
/// yet freed nor passed to another API that takes ownership of it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_preferences_set_script_thread_stack_size_kb(
    preferences: *mut ServoPreferences,
    value: i64,
) -> i32 {
    assert!(
        !preferences.is_null(),
        "preferences pointer must not be null"
    );
    if !((1024..=1024 * 1024).contains(&value)) {
        return -1;
    }
    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `preferences` documented above.
    unsafe { &mut *preferences }.thread_script_stack_size_kb = value;
    0
}

/// Sets the stack size of each worker and worklet thread, in KiB (preference
/// `thread_worker_stack_size_kb`; default 2048), divided between script and the
/// engine as for `servo_preferences_set_script_thread_stack_size_kb`. Takes effect for
/// threads started afterwards. `value` must be from 1024 to 1048576.
///
/// Returns 0, or -1 if `value` is out of range, leaving the preference unchanged.
///
/// # Safety
///
/// The caller must ensure that `preferences` is a non-null pointer to a
/// `ServoPreferences` previously returned by `servo_preferences_create` and not
/// yet freed nor passed to another API that takes ownership of it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_preferences_set_worker_thread_stack_size_kb(
    preferences: *mut ServoPreferences,
    value: i64,
) -> i32 {
    assert!(
        !preferences.is_null(),
        "preferences pointer must not be null"
    );
    if !((1024..=1024 * 1024).contains(&value)) {
        return -1;
    }
    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `preferences` documented above.
    unsafe { &mut *preferences }.thread_worker_stack_size_kb = value;
    0
}

/// Sets the stack size of the threads that style and lay out pages in parallel, in
/// KiB (preference `thread_layout_stack_size_kb`; default 0). 0 keeps the style
/// system's own threads, whose stacks are fixed when the engine is built; a value
/// larger than their stack gives layout its own threads with stacks of this size, for
/// deeply nested pages. Read once, when parallel layout first runs in the process.
/// `value` must be 0 or from 1 to 1048576.
///
/// Returns 0, or -1 if `value` is out of range, leaving the preference unchanged.
///
/// # Safety
///
/// The caller must ensure that `preferences` is a non-null pointer to a
/// `ServoPreferences` previously returned by `servo_preferences_create` and not
/// yet freed nor passed to another API that takes ownership of it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_preferences_set_layout_thread_stack_size_kb(
    preferences: *mut ServoPreferences,
    value: i64,
) -> i32 {
    assert!(
        !preferences.is_null(),
        "preferences pointer must not be null"
    );
    if !((0..=1024 * 1024).contains(&value)) {
        return -1;
    }
    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `preferences` documented above.
    unsafe { &mut *preferences }.thread_layout_stack_size_kb = value;
    0
}

/// Sets the number of threads that style and lay out pages in parallel (preference
/// `layout_threads`; default 3). 0 or 1 styles and lays out on the script thread
/// alone; -1 picks three quarters of the processor's logical cores; at most 6 are
/// used. Read once, when parallel layout first runs in the process. `value` must be
/// from -1 to 64.
///
/// Returns 0, or -1 if `value` is out of range, leaving the preference unchanged.
///
/// # Safety
///
/// The caller must ensure that `preferences` is a non-null pointer to a
/// `ServoPreferences` previously returned by `servo_preferences_create` and not
/// yet freed nor passed to another API that takes ownership of it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_preferences_set_layout_threads(
    preferences: *mut ServoPreferences,
    value: i64,
) -> i32 {
    assert!(
        !preferences.is_null(),
        "preferences pointer must not be null"
    );
    if !((-1..=64).contains(&value)) {
        return -1;
    }
    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `preferences` documented above.
    unsafe { &mut *preferences }.layout_threads = value;
    0
}
