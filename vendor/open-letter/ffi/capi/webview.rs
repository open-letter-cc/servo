// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `ffi/capi/webview.rs`.

#[allow(unused_imports)]
use super::*;

/// Sets the callback that vets every resource request the `WebView` makes. See
/// [`ServoResourceRequestCallback`] for when it is called and what its answer does.
///
/// `callback` may be `NULL`, which leaves the `WebView` without one: every request
/// proceeds as it does today. `user_data` is passed to every call of `callback` and is
/// independent of the delegate's `user_data`. It may be `NULL`.
///
/// A callback set here is in place before the `WebView`'s first request, its initial
/// URL's navigation included. It works whether or not a delegate is set on the builder.
/// Change or clear it afterwards with [`servo_webview_set_resource_request_callback`].
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `builder` is a non-null pointer to a `ServoWebViewBuilder` previously returned by
///   `servo_webview_builder_create` and not yet freed or built.
/// - `callback`, if non-null, is a valid C ABI function matching the signature of
///   [`ServoResourceRequestCallback`] and does not unwind across the FFI boundary.
/// - `callback` and `user_data` remain valid for as long as the `WebView` built from
///   `builder` exists, or until another callback is set on it.
/// - The call is made from the thread that created `builder`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_builder_set_resource_request_callback(
    builder: *mut ServoWebViewBuilder,
    callback: ServoResourceRequestCallback,
    user_data: *mut c_void,
) {
    assert!(!builder.is_null(), "builder pointer must not be null");
    // SAFETY: The caller is assumed to uphold the safety requirements for `builder`
    // documented above.
    let builder = unsafe { &mut *builder };
    builder.resource_request = callback.map(|callback| ResourceRequestHook {
        callback,
        user_data,
    });
}

/// Sets, replaces or clears (`callback` `NULL`) the resource request callback of a live
/// `WebView`. See [`ServoResourceRequestCallback`] for when it is called and what its
/// answer does. The change applies to requests offered after this returns.
///
/// Returns `true` if the change was made. Returns `false`, changing nothing, if
/// `webview` is null, or if the `WebView` has no delegate of this API's own: built by
/// `servo_webview_accept_new` with a `NULL` delegate, or by
/// `servo_webview_builder_build` with neither a delegate nor a resource request
/// callback. A `WebView` built any other way accepts one. To vet the requests of a
/// `WebView` that `servo_webview_accept_new` creates, pass a delegate to it and call
/// this before the next `servo_spin_event_loop`, which is before the new `WebView`
/// makes any request.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `webview` is null or a valid pointer returned by `servo_webview_builder_build` or
///   `servo_webview_accept_new` and not yet freed with `servo_webview_free`.
/// - `callback`, if non-null, is a valid C ABI function matching the signature of
///   [`ServoResourceRequestCallback`] and does not unwind across the FFI boundary.
/// - `callback` and `user_data` remain valid for as long as the `WebView` exists, or
///   until another callback is set on it.
/// - The call is made from the thread that created the `Servo` instance.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_set_resource_request_callback(
    webview: *const WebView,
    callback: ServoResourceRequestCallback,
    user_data: *mut c_void,
) -> bool {
    if webview.is_null() {
        return false;
    }
    // SAFETY: The caller is assumed to uphold the safety requirements for `webview`
    // documented above.
    let webview = unsafe { &*webview };
    set_resource_request_hook(
        webview.id(),
        callback.map(|callback| ResourceRequestHook {
            callback,
            user_data,
        }),
    )
}

/// Shows the `WebView` within its rendering context, undoing
/// `servo_webview_hide`. Showing a `WebView` that is already shown does
/// nothing.
///
/// Its documents, nested documents included, become visible again: each
/// one's `document.visibilityState` becomes `"visible"` and it receives a
/// `visibilitychange` event. Its timers run at their normal rate again.
///
/// `WebView`s are shown when they are built.
///
/// `webview` is a handle to a `WebView` object.
/// The ownership of `webview` remains with the caller after the call.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `webview` is a non-null pointer to a `WebView` previously returned
///   by `servo_webview_builder_build` and not yet passed to
///   `servo_webview_free`. No other code may read or write `*webview`
///   for the duration of this call.
/// - The call is made from the same thread that originally created the
///   `WebView` via `servo_webview_builder_build`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_show(webview: *mut WebView) {
    assert!(!webview.is_null(), "webview pointer must not be null");

    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `webview` documented above.
    let webview = unsafe { &*webview };

    webview.show();
}

/// Hides the `WebView` within its rendering context, keeping all of its
/// state. Hiding a `WebView` that is already hidden does nothing.
///
/// While hidden, the `WebView` is not painted, and it behaves as a page in a
/// tab that is not in view: its documents, nested documents included, have a
/// `document.visibilityState` of `"hidden"` (each receives a
/// `visibilitychange` event when it is hidden), and their timers run at a
/// heavily limited rate. Documents created while the `WebView` is hidden
/// start hidden. `servo_webview_show` reverses all of this.
///
/// `webview` is a handle to a `WebView` object.
/// The ownership of `webview` remains with the caller after the call.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `webview` is a non-null pointer to a `WebView` previously returned
///   by `servo_webview_builder_build` and not yet passed to
///   `servo_webview_free`. No other code may read or write `*webview`
///   for the duration of this call.
/// - The call is made from the same thread that originally created the
///   `WebView` via `servo_webview_builder_build`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_hide(webview: *mut WebView) {
    assert!(!webview.is_null(), "webview pointer must not be null");

    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `webview` documented above.
    let webview = unsafe { &*webview };

    webview.hide();
}

/// Returns the operating-system process id of the content process running this
/// `WebView`'s active top-level document, or 0 if there is none.
///
/// 0 means: this payload or this `Servo` runs content in the embedder's own process
/// (no multiprocess feature, or `servo_options_set_multiprocess` off), or no top-level
/// document has become active yet. The value changes when a navigation moves the
/// `WebView` into a different content process; `notify_frame_changed` with
/// `SERVO_FRAME_EVENT_NAVIGATED` for the top-level frame (`parent_browsing_context_id`
/// of 0) is the point after which to read it again.
///
/// A content process that exits is noticed by the engine on its own, through a channel
/// that closes with the process. Every `WebView` whose active document it served is
/// told through `notify_crashed`, and from that notification on this function returns
/// 0 for the `WebView` until a replacement document (the engine's crash page, or
/// whatever the embedder loads next) becomes active and names its process. The
/// engine never again sends to the dead process, so freeing the `WebView` and the
/// `Servo` after such an exit returns as normal.
///
/// One content process can serve several `WebView`s, so the same id can be returned
/// for more than one. Out-of-process subframes are not reported: a document in a
/// subframe can run in a process this function never names.
///
/// The id is the one the operating system issued, valid only for as long as that
/// process is alive; Windows reuses ids after exit. Open a handle to it promptly
/// rather than retaining the number.
///
/// Must be called on the thread that created the `Servo` instance.
///
/// # Safety
///
/// `webview` must be a valid pointer returned by `servo_webview_builder_build` and
/// not yet freed with `servo_webview_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_content_process_id(webview: *const WebView) -> u32 {
    if webview.is_null() {
        return 0;
    }
    // SAFETY: The caller is assumed to uphold the safety requirements for `webview`
    // documented above.
    let webview = unsafe { &*webview };
    crate::webview_delegate::top_level_content_process(webview.id())
}

/// Adds a user script to be installed in every document this `WebView` loads.
///
/// The script runs in the document's own world, with the same capabilities as a
/// `<script>` the page itself contained. Servo does not interpret it, and a script
/// added here is the embedder's own code regardless of what the document is.
///
/// `script_ptr`/`script_len` are UTF-8 source, not NUL-terminated. The bytes are
/// copied, so the caller may free them as soon as this returns. Scripts are
/// installed in the order they are added.
///
/// This must be called before `servo_webview_builder_build`. Adding a script to a
/// `WebView` that already exists is [`servo_webview_add_script`], which works on every
/// `WebView` this function's builder produced whether or not a script was added here.
///
/// Returns `true` if the script was accepted, `false` if `script_ptr` is null with a
/// non-zero length or the bytes are not valid UTF-8. A rejected script is not
/// installed and the builder is left unchanged.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `builder` is a non-null pointer to a `ServoWebViewBuilder` previously returned
///   by `servo_webview_builder_create` and not yet freed or built.
/// - `script_ptr` is either null with a `script_len` of zero, or valid for reads of
///   `script_len` bytes for the duration of the call.
/// - The call is made from the thread that created `builder`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_builder_add_script(
    builder: *mut ServoWebViewBuilder,
    script_ptr: *const u8,
    script_len: usize,
) -> bool {
    assert!(!builder.is_null(), "builder pointer must not be null");

    // SAFETY: The caller is assumed to uphold the safety requirements documented
    // above, which include those of `script_from_raw`.
    let script = unsafe { script_from_raw(script_ptr, script_len) };
    let Some(script) = script else {
        return false;
    };

    // SAFETY: The caller is assumed to uphold the safety requirements for `builder`
    // documented above. Ownership stays with the caller.
    let builder = unsafe { &mut *builder };
    builder.user_scripts.push(script);

    true
}

/// Adds a user script to a `WebView` that already exists. It is installed in every
/// document the `WebView` loads from now on, and not in the one already loaded.
///
/// See [`servo_webview_builder_add_script`] for what a user script is and how the
/// bytes are treated.
///
/// Returns `true` if the script was accepted, and `false` if the bytes are rejected on
/// the same terms as [`servo_webview_builder_add_script`].
///
/// Every `WebView` built by `servo_webview_builder_build` carries a user-content
/// manager, so this does not depend on a script having been added at build time. A
/// `false` return therefore always means the bytes were rejected, never that the
/// `WebView` was the wrong kind.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `webview` is a non-null pointer to a `WebView` previously returned by
///   `servo_webview_builder_build` and not yet freed.
/// - `script_ptr` is either null with a `script_len` of zero, or valid for reads of
///   `script_len` bytes for the duration of the call.
/// - The call is made from the thread that created `webview`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_add_script(
    webview: *mut WebView,
    script_ptr: *const u8,
    script_len: usize,
) -> bool {
    assert!(!webview.is_null(), "webview pointer must not be null");

    // SAFETY: The caller is assumed to uphold the safety requirements documented
    // above, which include those of `script_from_raw`.
    let script = unsafe { script_from_raw(script_ptr, script_len) };
    let Some(script) = script else {
        return false;
    };

    // SAFETY: The caller is assumed to uphold the safety requirements for `webview`
    // documented above. Ownership stays with the caller.
    let webview = unsafe { &*webview };

    let Some(user_content_manager) = webview.user_content_manager() else {
        return false;
    };
    user_content_manager.add_script(Rc::new(UserScript::new(script, None)));

    true
}

/// Evaluate `script` in the document `webview` is showing now, asynchronously.
///
/// Unlike [`servo_webview_add_script`], which installs a script for every document the
/// `WebView` loads *from now on*, this runs once, in the current document's own realm,
/// and never in a later one. It is `WebView::evaluate_javascript` with the result
/// dropped: the call returns as soon as the script is queued, and an evaluation error is
/// logged, never reported. A script queued before the current document has loaded runs
/// in whichever document is current when Servo evaluates it.
///
/// This is transport only. Servo applies no policy to the script; deciding which
/// documents may be sent what is the embedder's job.
///
/// Returns `true` if the script was queued, `false` if its bytes are not UTF-8.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `webview` is a non-null pointer to a `WebView` previously returned by
///   `servo_webview_builder_build` and not yet freed.
/// - `script_ptr` is either null with a `script_len` of zero, or valid for reads of
///   `script_len` bytes for the duration of the call.
/// - The call is made from the thread that created `webview`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_evaluate_javascript(
    webview: *mut WebView,
    script_ptr: *const u8,
    script_len: usize,
) -> bool {
    assert!(!webview.is_null(), "webview pointer must not be null");

    // SAFETY: The caller is assumed to uphold the safety requirements documented
    // above, which include those of `script_from_raw`.
    let script = unsafe { script_from_raw(script_ptr, script_len) };
    let Some(script) = script else {
        return false;
    };

    // SAFETY: The caller is assumed to uphold the safety requirements for `webview`
    // documented above. Ownership stays with the caller.
    let webview = unsafe { &*webview };

    webview.evaluate_javascript(script, |result| {
        if let Err(error) = result {
            log::warn!("servo_webview_evaluate_javascript: {error:?}");
        }
    });

    true
}

/// Copy a borrowed UTF-8 script out of embedder memory, or `None` if the pointer and
/// length do not describe valid UTF-8.
///
/// # Safety
///
/// `script_ptr` must be either null with a `script_len` of zero, or valid for reads
/// of `script_len` bytes.
pub(super) unsafe fn script_from_raw(script_ptr: *const u8, script_len: usize) -> Option<String> {
    if script_len == 0 {
        return Some(String::new());
    }
    if script_ptr.is_null() {
        return None;
    }

    // SAFETY: The caller guarantees that `script_ptr` is valid for reads of
    // `script_len` bytes. The slice is copied and not held past this call.
    let bytes = unsafe { std::slice::from_raw_parts(script_ptr, script_len) };
    std::str::from_utf8(bytes).ok().map(str::to_owned)
}
