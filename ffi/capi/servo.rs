/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::ffi::{CStr, c_char};
#[cfg(feature = "multiprocess")]
use std::panic;

use servo_api::Servo;

/// Destroys the `Servo` instance returned by `servo_builder_build` and
/// frees its memory.
///
/// `servo` is a handle to a `Servo` object.
/// The ownership of `servo` is transferred to the function. The caller
/// must not use or free `servo` again.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `servo` was previously returned by `servo_builder_build` and has
///   not yet been freed nor passed to another API that takes ownership
///   of it.
/// - The call is made from the same thread that originally created the
///   `Servo` instance via `servo_builder_build`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_free(servo: *mut Servo) {
    assert!(!servo.is_null(), "servo pointer must not be null");

    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `servo` documented above.
    unsafe {
        let _: Box<Servo> = Box::from_raw(servo);
    }
}

/// Spin the Servo event loop once. The embedder should call this
/// periodically to process incoming messages and perform rendering
/// updates.
///
/// `servo` is a handle to a `Servo` object.
/// The ownership of `servo` remains with the caller after the call.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `servo` is a non-null pointer to a `Servo` instance previously
///   returned by `servo_builder_build` and has not yet been freed nor
///   passed to another API that takes ownership of it.
/// - The call is made from the same thread that originally created the
///   `Servo` instance via `servo_builder_build`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_spin_event_loop(servo: *mut Servo) {
    assert!(!servo.is_null(), "servo pointer must not be null");

    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `servo` documented above.
    let servo = unsafe { &*servo };

    servo.spin_event_loop();
}

/// Initialize logging for the Servo instance.
///
/// `servo` is a handle to a `Servo` object.
/// The ownership of `servo` remains with the caller after the call.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `servo` is a non-null pointer to a `Servo` instance previously
///   returned by `servo_builder_build` and has not yet been freed nor
///   passed to another API that takes ownership of it.
/// - The call is made from the same thread that originally created the
///   `Servo` instance via `servo_builder_build`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_setup_logging(servo: *mut Servo) {
    assert!(!servo.is_null(), "servo pointer must not be null");

    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `servo` documented above.
    let servo = unsafe { &*servo };

    servo.setup_logging();
}

// The result codes of `servo_content_process_main`. All four are part of the ABI in
// both feature arms and are declared unconditionally, but each arm only returns some
// of them, so each is allowed to be unused rather than being gated on the feature:
// an embedder compiles against the whole set whichever arm the payload was built
// with.
/// The content process ran to completion.
#[allow(dead_code)]
pub const SERVO_CONTENT_PROCESS_OK: i32 = 0;
/// `token` was null, or was not valid UTF-8.
#[allow(dead_code)]
pub const SERVO_CONTENT_PROCESS_INVALID_TOKEN: i32 = -1;
/// The content process failed to start or terminated abnormally.
#[allow(dead_code)]
pub const SERVO_CONTENT_PROCESS_FAILED: i32 = -2;
/// This build has no multiprocess support compiled in, so it cannot serve a
/// content process. The symbol still resolves so that an embedder can detect this
/// by calling it rather than by failing to find it; see
/// [`servo_content_process_main`].
#[allow(dead_code)]
pub const SERVO_CONTENT_PROCESS_UNSUPPORTED: i32 = -3;

/// Serve a content process on this thread, for an embedder that loads Servo as a
/// shared library.
///
/// With `servo_options_set_multiprocess` enabled, Servo starts a content process
/// by re-executing **the host's own binary** with a `--content-process <token>`
/// argument. That works when the binary being re-executed is Servo's own, and not
/// when it belongs to a library embedder: the child does not recognise the
/// argument, exits, and nothing renders. This entry point is how such a host
/// services that re-exec rather than having to *be* `servoshell`.
///
/// A host that recognises the argument calls this instead of its ordinary main
/// loop, passing the token verbatim, and exits with the value returned. `token` is
/// a NUL-terminated UTF-8 string. Servo does not interpret it beyond using it to
/// connect back to the process that spawned this one.
///
/// This function returns only when the content process is finished, which is
/// normally when its parent goes away. It takes over the calling thread for that
/// whole time and must be called before the host has created a `Servo` instance of
/// its own: a content process initialises process-wide state (options,
/// preferences, the JS engine) that cannot be set twice in one process.
///
/// Returns `SERVO_CONTENT_PROCESS_OK` on normal completion, or one of the other
/// `SERVO_CONTENT_PROCESS_*` codes. A panic while serving is contained and
/// reported as `SERVO_CONTENT_PROCESS_FAILED` rather than unwinding into the
/// host's C frame.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `token` is either null or a valid pointer to a NUL-terminated string that
///   remains valid for the duration of the call.
/// - No `Servo` instance has been created in this process.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_content_process_main(token: *const c_char) -> i32 {
    if token.is_null() {
        return SERVO_CONTENT_PROCESS_INVALID_TOKEN;
    }

    // SAFETY: The caller is assumed to uphold the safety requirements for `token`
    // documented above.
    let token = unsafe { CStr::from_ptr(token) };
    let Ok(token) = token.to_str() else {
        return SERVO_CONTENT_PROCESS_INVALID_TOKEN;
    };
    let token = token.to_owned();

    // A panic must not unwind into the host's C frame. `run_content_process`
    // reports every startup failure by panicking, so this is also how a failed
    // start becomes a return code.
    #[cfg(feature = "multiprocess")]
    {
        match panic::catch_unwind(move || servo_api::run_content_process(token)) {
            Ok(()) => SERVO_CONTENT_PROCESS_OK,
            Err(..) => SERVO_CONTENT_PROCESS_FAILED,
        }
    }

    // Upstream gates the content-process entry point behind its `multiprocess`
    // feature, and this crate's `servo` dependency does not enable it, so there is
    // nothing to hand the token to. Report that rather than pretending to serve.
    #[cfg(not(feature = "multiprocess"))]
    {
        let _ = token;
        SERVO_CONTENT_PROCESS_UNSUPPORTED
    }
}
