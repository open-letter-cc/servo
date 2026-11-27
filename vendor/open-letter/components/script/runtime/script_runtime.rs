// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `components/script/runtime/script_runtime.rs`.

#[allow(unused_imports)]
use super::*;

/// The kinds of thread that run JavaScript, each with its own stack size preference.
#[derive(Clone, Copy, Debug)]
pub(crate) enum JSThreadKind {
    /// A script thread, which runs the documents of an event loop.
    Script,
    /// A worker or worklet thread.
    Worker,
}

/// The smallest stack, in KiB, that a JavaScript thread is given.
pub(super) const MIN_JS_THREAD_STACK_SIZE_KB: i64 = 1024;

/// Stack kept beyond the untrusted-script limit for trusted and system code, as mozjs
/// keeps by default.
pub(super) const SYSTEM_CODE_STACK_BUFFER: usize = 10 * 1024;

pub(super) const TRUSTED_SCRIPT_STACK_BUFFER: usize = 8 * 12800;

impl JSThreadKind {
    /// The stack size, in bytes, to spawn a thread of this kind with, from preferences.
    pub(crate) fn stack_size(self) -> usize {
        let kilobytes = match self {
            JSThreadKind::Script => pref!(thread_script_stack_size_kb),
            JSThreadKind::Worker => pref!(thread_worker_stack_size_kb),
        };
        kilobytes.max(MIN_JS_THREAD_STACK_SIZE_KB) as usize * 1024
    }
}

/// Record, on a newly spawned JavaScript thread, the stack size it was spawned with, so
/// that the runtime created on it limits script to part of that stack. Call this first
/// thing on the thread, before creating its [`Runtime`].
pub(crate) fn set_js_thread_stack_size(stack_size: usize) {
    JS_THREAD_STACK_SIZE.set(Some(stack_size));
}

/// The native stack quota for script on a thread with a stack of `stack_size` bytes: half
/// of it. The other half is kept for the engine frames that run on the same thread below
/// or between script frames: layout when it runs on the script thread, garbage
/// collection, and native functions called from script.
pub(super) fn native_stack_quota(stack_size: usize) -> usize {
    stack_size / 2
}
