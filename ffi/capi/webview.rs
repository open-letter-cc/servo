/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::ffi::{CStr, c_void};
use std::os::raw::c_char;
use std::rc::Rc;

use servo_api::{Servo, UserContentManager, UserScript, WebView, WebViewBuilder};

use crate::rendering_context::RenderingContext;
use crate::webview_delegate::ServoWebViewDelegate;

/// An opaque struct representing a builder object for constructing new
/// `WebView`s.
///
/// Handles to this object can be created using [`servo_webview_builder_create`].
///
/// # Thread safety
///
/// The handle must be used only from the thread that created it.
/// It must also be created on the same thread that created the `Servo`
/// instance passed to `servo_webview_builder_create``.
// cbindgen:opaque
pub struct ServoWebViewBuilder {
    servo: Servo,
    rendering_context: Rc<dyn servo_api::RenderingContext>,
    url: Option<url::Url>,
    delegate: Option<ServoWebViewDelegate>,
    /// User scripts to install, in the order they were added. A `WebView` only has a
    /// `UserContentManager` if at least one script is added before it is built, which
    /// is also what makes `servo_webview_add_script` work on it afterwards.
    user_scripts: Vec<String>,
}

/// Creates a handle to a new `WebViewBuilder` object for the given
/// `servo` instance and rendering context.
///
/// `servo` is a handle to a `Servo` object.
/// The ownership of `servo` remains with the caller after the call.
///
/// `context` is a handle to a `RenderingContext` object.
/// The ownership of `context` is transferred to the function.
/// The caller must not use or free `context` again.
///
/// Returns a newly allocated `ServoWebViewBuilder` handle. The
/// ownership of the returned handle is transferred to the caller, who
/// must free it with [`servo_webview_builder_free`] or consume it by
/// passing it to `servo_webview_builder_build`.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `servo` is a non-null pointer to a `Servo` instance previously
///   returned by `servo_builder_build` and has not yet been freed nor
///   passed to another API that takes ownership of it.
/// - `context` is a non-null pointer to a `RenderingContext` previously
///   returned by one of the `servo_rendering_context_create_*`
///   functions and has not yet been freed nor passed to another API
///   that takes ownership of it.
/// - The call is made from the same thread that created `servo` and
///   `context`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_builder_create(
    servo: *mut Servo,
    context: *mut RenderingContext,
) -> *mut ServoWebViewBuilder {
    assert!(!servo.is_null(), "servo pointer must not be null");
    assert!(!context.is_null(), "context pointer must not be null");

    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `servo` documented above.
    let servo = unsafe { &*servo };

    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `context` documented above. We take ownership here.
    let boxed_c_context = unsafe { Box::from_raw(context) };

    Box::into_raw(Box::new(ServoWebViewBuilder {
        servo: servo.clone(),
        rendering_context: boxed_c_context.inner,
        url: None,
        delegate: None,
        user_scripts: Vec::new(),
    }))
}

/// Sets the initial URL for the `WebView`.
///
/// `builder` is a handle to a `ServoWebViewBuilder` object.
/// The ownership of `builder` remains with the caller after the call.
///
/// `url` is a NUL terminated UTF-8 string.
/// The function panics if it is not a valid UTF-8 string.
/// The ownership of `url` remains with the caller after the call.
///
/// Returns 0 on success, or -1 if the URL could not be parsed.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `builder` is a non-null pointer to a `ServoWebViewBuilder`
///   previously returned by `servo_webview_builder_create` and has not
///   yet been freed nor passed to another API that takes ownership of
///   it.
/// - `url` is a non-null pointer to a C string that remains unmodified
///   for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_builder_set_url(
    builder: *mut ServoWebViewBuilder,
    url: *const c_char,
) -> i32 {
    assert!(!builder.is_null(), "builder pointer must not be null");
    assert!(!url.is_null(), "url pointer must not be null");

    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `builder` documented above.
    let builder = unsafe { &mut *builder };

    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `url` documented above.
    let url_str = unsafe { CStr::from_ptr(url) }.to_str().unwrap();

    match url::Url::parse(url_str) {
        Ok(parsed) => {
            builder.url = Some(parsed);
            0
        },
        Err(_) => -1,
    }
}

/// Sets the delegate that will receive notification for `WebView` events.
///
/// `builder` is a handle to a `ServoWebViewBuilder` object.
/// The ownership of `builder` remains with the caller after the call.
///
/// `delegate` is a `ServoWebViewDelegate` struct with callbacks
/// for the notifications you are interested in.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `builder` is a non-null pointer to a `ServoWebViewBuilder`
///   previously returned by `servo_webview_builder_create` and has not
///   yet been freed nor passed to another API that takes ownership of
///   it.
/// - `delegate` must uphold the safety requirements documented on the
///    `ServoWebViewDelegate` type.
///
/// Servo copies the delegate's function pointers — they must remain
/// valid for the lifetime of the `WebView` (or until a new delegate is set).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_builder_set_delegate(
    builder: *mut ServoWebViewBuilder,
    delegate: ServoWebViewDelegate,
) {
    assert!(!builder.is_null(), "builder pointer must not be null");
    let builder = unsafe { &mut *builder };
    builder.delegate = Some(delegate);
}

/// Consumes `builder` and creates a new `WebView` instance.
///
/// `builder` is a handle to a `ServoWebViewBuilder` object.
/// The ownership of `builder` is transferred to the function. The
/// caller must not use or free `builder` again.
///
/// Returns a newly allocated `WebView` handle. The ownership of the
/// returned handle is transferred to the caller, who must free it with
/// [`servo_webview_free`].
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `builder` is a non-null pointer to a `ServoWebViewBuilder`
///   previously returned by `servo_webview_builder_create` and has not
///   yet been freed nor passed to another API that takes ownership of
///   it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_builder_build(
    builder: *mut ServoWebViewBuilder,
) -> *mut WebView {
    assert!(!builder.is_null(), "builder pointer must not be null");

    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `builder` documented above. We take ownership here.
    let builder = unsafe { Box::from_raw(builder) };

    let mut webview_builder = WebViewBuilder::new(&builder.servo, builder.rendering_context);

    if let Some(url) = builder.url {
        webview_builder = webview_builder.url(url);
    }

    if let Some(delegate) = builder.delegate {
        webview_builder = webview_builder.delegate(std::rc::Rc::new(delegate));
    }

    // Every `WebView` gets a `UserContentManager`, whether or not a script was added
    // here. Creating it lazily would make `servo_webview_add_script` work only on a
    // `WebView` that happened to be built with a script already, and there is no way to
    // attach a manager after the fact — a conditional nobody remembers by the time a
    // script is added later.
    let user_content_manager = Rc::new(UserContentManager::new(&builder.servo));
    for script in builder.user_scripts {
        user_content_manager.add_script(Rc::new(UserScript::new(script, None)));
    }
    webview_builder = webview_builder.user_content_manager(user_content_manager);

    Box::into_raw(Box::new(webview_builder.build()))
}

/// Destroys `builder` and frees its memory.
///
/// `builder` is a handle to a `ServoWebViewBuilder` object.
/// The ownership of `builder` is transferred to the function. The
/// caller must not use or free `builder` again.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `builder` was previously returned by `servo_webview_builder_create`
///   and has not yet been freed nor passed to another API that takes
///   ownership of it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_builder_free(builder: *mut ServoWebViewBuilder) {
    assert!(!builder.is_null(), "builder pointer must not be null");

    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `builder` documented above.
    unsafe {
        let _ = Box::from_raw(builder);
    }
}

/// Paints the contents of the `WebView` to the rendering context's
/// surface.
///
/// Should be called when the embedder receives a
/// [`notify_new_frame_ready`] notification via [`ServoWebViewDelegate`]
/// or when a repaint is needed for other reasons.
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
pub unsafe extern "C" fn servo_webview_paint(webview: *mut WebView) {
    assert!(!webview.is_null(), "webview pointer must not be null");

    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `webview` documented above.
    let webview = unsafe { &*webview };

    webview.paint();
}

/// Loads the given URL into the `WebView`.
///
/// `webview` is a handle to a `WebView` object.
/// The ownership of `webview` remains with the caller after the call.
///
/// `url` is a NUL terminated UTF-8 string.
/// The function panics if it is not a valid UTF-8 string.
/// The ownership of `url` remains with the caller after the call.
///
/// Returns 0 on success, or -1 if the URL could not be parsed. **The return
/// value reports only whether the URL parsed**, not that the load will
/// happen: the load is handed to Servo without acknowledgement, and nothing
/// here can report what becomes of it.
///
/// # Loading before the first load has committed
///
/// A `WebView` begins loading as soon as it is built — the URL given to
/// `servo_webview_builder_set_url`, or `about:blank` when none was given. A
/// load issued through this function before that first load has committed
/// may be superseded by it and silently dropped, having already returned 0.
///
/// To open a `WebView` on a particular URL, set it on the builder rather
/// than building the `WebView` and loading immediately afterwards. Where a
/// load really must follow creation, wait until
/// `notify_load_status_changed` reports `SERVO_LOAD_STATUS_COMPLETE` first.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `webview` is a non-null pointer to a `WebView` previously returned
///   by `servo_webview_builder_build` and not yet passed to
///   `servo_webview_free`. No other code may read or write `*webview`
///   for the duration of this call.
/// - `url` is a non-null pointer to a C string that remains unmodified
///   for the duration of the call.
/// - The call is made from the same thread that originally created the
///   `WebView` via `servo_webview_builder_build`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_load(webview: *mut WebView, url: *const c_char) -> i32 {
    assert!(!webview.is_null(), "webview pointer must not be null");
    assert!(!url.is_null(), "url pointer must not be null");

    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `webview` documented above.
    let webview = unsafe { &*webview };

    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `url` documented above.
    let url_str = unsafe { CStr::from_ptr(url) }.to_str().unwrap();

    match url::Url::parse(url_str) {
        Ok(parsed) => {
            webview.load(parsed);
            0
        },
        Err(_) => -1,
    }
}

/// Asynchronously takes a screenshot of the full `WebView` viewport.
///
/// The provided `callback` will be invoked with either the image data
/// when the screenshot is ready, or error information if Servo was not able
/// to handle the screenshot request successfully.
///
/// `webview` is a handle to a `WebView` object.
/// The ownership of `webview` remains with the caller after the call.
///
/// `callback` is a function pointer that will be invoked at
/// most once when the screenshot completes. The callback is invoked
/// asynchronously on the embedder thread that runs `servo_spin_event_loop`.
///
/// The callback receives:
/// - `data`  : pointer to raw pixel data, or `NULL` on error. Data is in
///             RGBA format with 4 bytes per pixel.
/// - `width` : image width in pixels or 0 on error.
/// - `height`: image height in pixels or 0 on error.
/// - `error` : error code represented by one of
///             the `SERVO_SCREENSHOT_CAPTURE_ERROR_*` constants.
///             Only valid when `data` is `NULL`.
/// - `user_data`: the user data pointer that was passed to`servo_webview_take_screenshot`.
///
/// The `data` pointer is valid only for the duration of the callback.
/// The callback must copy the data if it needs to be persist after the callback
/// returns. The ownership of `data` remains with Servo.
///
/// `user_data` is an opaque pointer that is passed to `callback` when it is invoked.
///  The ownership and validity of `user_data` is the caller's responsibility.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `webview` is a non-null pointer to a `WebView` previously returned
///   by `servo_webview_builder_build` and not yet passed to
///   `servo_webview_free`. No other code may read or write `*webview`
///   for the duration of this call.
/// - If `callback` is not null, it is a valid C ABI function matching
///   the exact signature shown, remains valid until it is invoked or
///   the `WebView` is freed, and does not unwind across the FFI
///   boundary.
/// - `user_data` is either null or remains valid until `callback` is
///   invoked or the `WebView` is freed.
/// - The call is made from the same thread that originally created the
///   `WebView` via `servo_webview_builder_build`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_take_screenshot(
    webview: *mut WebView,
    callback: Option<
        unsafe extern "C" fn(
            data: *const u8,
            width: u32,
            height: u32,
            error: i32,
            user_data: *mut c_void,
        ),
    >,
    user_data: *mut c_void,
) {
    assert!(!webview.is_null(), "webview pointer must not be null");

    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `webview` documented above.
    let webview = unsafe { &*webview };

    let Some(callback) = callback else {
        return;
    };

    webview.take_screenshot(None, move |result| {
        match result {
            Ok(image) => {
                let (width, height) = image.dimensions();
                let data = image.into_raw();
                // SAFETY: The caller is assumed to uphold the safety
                // requirements for `callback` documented above.
                // `data.as_ptr()` is valid for the call as the backing `Vec`
                // is dropped only after `callback` returns.
                unsafe {
                    callback(data.as_ptr(), width, height, 0, user_data);
                }
            },
            Err(error) => {
                let error_code = error as _;
                // SAFETY: The caller is assumed to uphold the safety
                // requirements for `callback` documented above.
                // `data` pointer is null as part of the contract.
                unsafe {
                    callback(std::ptr::null(), 0, 0, error_code, user_data);
                }
            },
        }
    });
}

/// Destroys the `WebView` instance and frees its memory.
///
/// `webview` is a handle to a `WebView` object.
/// The ownership of `webview` is transferred to the function. The
/// caller must not use or free `webview` again.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `webview` was previously returned by `servo_webview_builder_build`
///   and has not yet been freed nor passed to another API that takes
///   ownership of it.
/// - The call is made from the same thread that originally created the
///   `WebView` via `servo_webview_builder_build`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_free(webview: *mut WebView) {
    assert!(!webview.is_null(), "webview pointer must not be null");

    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `webview` documented above.
    unsafe {
        let _ = Box::from_raw(webview);
    }
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

/// Copy a borrowed UTF-8 script out of embedder memory, or `None` if the pointer and
/// length do not describe valid UTF-8.
///
/// # Safety
///
/// `script_ptr` must be either null with a `script_len` of zero, or valid for reads
/// of `script_len` bytes.
unsafe fn script_from_raw(script_ptr: *const u8, script_len: usize) -> Option<String> {
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
