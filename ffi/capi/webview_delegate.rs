/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::cell::RefCell;
use std::ffi::c_void;
use std::panic::{self, AssertUnwindSafe};

pub use servo_api::LoadStatus;
use servo_api::{
    AllowOrDenyRequest, ContextMenuAction, CreateNewWebViewRequest, Cursor, DeviceIntPoint,
    DeviceIntSize, EmbedderControl, EmbedderControlTag, NavigationRequest,
    SelectElementOptionOrOptgroup, UserContentManager, WebView, WebViewDelegate,
};

use crate::rendering_context::RenderingContext;

/// The delegate that receives notifications about `WebView` events.
///
/// The function pointers can be set to `NULL` for callbacks that are not
/// needed. All callbacks receive the `user_data` pointer that was set in
/// this struct. The callbacks are invoked on the embedder thread that
/// calls `servo_spin_event_loop`.
///
/// Refer to the documentation of the corresponding
/// [`servo::WebViewDelegate`] trait in the Rust API for more information.
///
/// [`servo::WebViewDelegate`]: https://doc.servo.org/servo/trait.WebViewDelegate.html
///
/// # Ownership of callback arguments
///
/// The `webview` argument passed to each callback is a temporary handle.
/// Its lifetime is limited to the duration of the callback. The
/// embedder must not retain or use handle after the callback returns.
/// The callback must not pass it to `servo_webview_free` or any other
/// function that takes ownership of a `WebView`.
///
/// The `user_data` pointer is owned by the embedder.
/// The validity and lifetime of `user_data` is the embedder's responsibility.
///
/// # Safety
///
/// The embedder must ensure that for all function-pointer fields of
/// this struct:
///
/// - A non-null function pointer is a valid C ABI callback function
///   matching the exact signature shown.
/// - The function pointers and `user_data` remain valid for as long as
///   this delegate is associated with any `WebView`.
/// - The callback does not unwind across the FFI boundary.
/// - The `webview` argument is not retained or used after the callback
///   returns and is not passed to `servo_webview_free` or any other
///   function that takes ownership of the `WebView`.
///
/// # Copying
///
/// Every field is a pointer or a nullable function pointer, so this is `Copy`. That is
/// what lets [`servo_webview_accept_new`] take a delegate by pointer and snapshot it
/// for the `WebView` it creates.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ServoWebViewDelegate {
    /// An opaque pointer passed to all delegate callbacks. May be `NULL`.
    pub user_data: *mut c_void,

    /// Called when the load status of the associated `WebView` changes.
    ///
    /// `load_status` is one of the `SERVO_LOAD_STATUS_*` constants.
    pub notify_load_status_changed: Option<
        unsafe extern "C" fn(webview: *mut WebView, load_status: i32, user_data: *mut c_void),
    >,

    /// Called when Servo has rendered a new frame and the embedder should
    /// call `servo_webview_paint` to update the rendering context.
    pub notify_new_frame_ready:
        Option<unsafe extern "C" fn(webview: *mut WebView, user_data: *mut c_void)>,

    /// Generic embedder control extension, no domain logic.
    ///
    /// An optional [`ServoEmbedderController`] owned by the embedder, or `NULL` to
    /// keep Servo's default behaviour for every embedder control. See
    /// [`ServoEmbedderController`] for the safety requirements on its fields.
    pub embedder_control: *const ServoEmbedderController,

    /// Called when the `WebView`'s content process has crashed.
    ///
    /// Without this callback, an in-process embedder has no signal at all when
    /// content stops responding: there is no separate process to observe dying.
    ///
    /// `reason_ptr`/`reason_len` describe the crash and are always present.
    /// `backtrace_ptr`/`backtrace_len` are a backtrace, or `NULL`/`0` if none
    /// is available. Both are borrowed for the duration of this call only,
    /// are not NUL-terminated, and are not valid UTF-8 unless the embedder
    /// checks them (Servo always produces valid UTF-8 here, but the ABI does
    /// not promise it). The embedder must copy the bytes if it needs to
    /// retain them past the call.
    pub notify_crashed: Option<
        unsafe extern "C" fn(
            webview: *mut WebView,
            reason_ptr: *const u8,
            reason_len: usize,
            backtrace_ptr: *const u8,
            backtrace_len: usize,
            user_data: *mut c_void,
        ),
    >,

    /// Called when the `WebView` has closed. No further callbacks will be
    /// delivered for it after this one returns.
    pub notify_closed: Option<unsafe extern "C" fn(webview: *mut WebView, user_data: *mut c_void)>,

    /// Called when content running in this `WebView` logs a console message.
    ///
    /// `level` is one of the `SERVO_CONSOLE_LOG_LEVEL_*` constants.
    /// `message_ptr`/`message_len` are the message text, borrowed for the
    /// duration of this call only and not NUL-terminated. The embedder must
    /// copy the bytes if it needs to retain them past the call.
    pub show_console_message: Option<
        unsafe extern "C" fn(
            webview: *mut WebView,
            level: i32,
            message_ptr: *const u8,
            message_len: usize,
            user_data: *mut c_void,
        ),
    >,

    /// Called when the `WebView` navigates to a new URL, including
    /// page-initiated navigation (a link click, `location =`, and similar)
    /// as well as host-initiated navigation via `servo_webview_load`.
    ///
    /// `url_ptr`/`url_len` are the URL's serialization, borrowed for the
    /// duration of this call only and not NUL-terminated. The embedder must
    /// copy the bytes if it needs to retain them past the call.
    pub notify_url_changed: Option<
        unsafe extern "C" fn(
            webview: *mut WebView,
            url_ptr: *const u8,
            url_len: usize,
            user_data: *mut c_void,
        ),
    >,

    /// Called before a navigation proceeds, so that the embedder can allow or
    /// refuse it. Return `true` to allow the navigation, `false` to refuse it.
    ///
    /// This fires for every navigation of this `WebView` or one of its inner
    /// frames, whichever side started it: a link click, a redirect, script
    /// setting `location`, or `servo_webview_load`. It is the counterpart to
    /// `notify_url_changed`, which reports a navigation that has already
    /// happened and so cannot refuse one.
    ///
    /// `url_ptr`/`url_len` are the target URL's serialization, borrowed for the
    /// duration of this call only and not NUL-terminated. The embedder must
    /// copy the bytes if it needs to retain them past the call.
    ///
    /// Servo attaches no meaning to the answer and forms no view of its own
    /// about which navigations are reasonable; this reports the navigation and
    /// carries the answer back, nothing more.
    ///
    /// # Defaults, which are not the same in both directions
    ///
    /// A `NULL` callback keeps Servo's default path, which **allows** the
    /// navigation. That matches Servo's own behaviour for an unhandled request.
    ///
    /// A callback that is present but **unwinds is treated as a refusal**, not
    /// as a default. A hook whose purpose is to vet navigations must not turn
    /// into a silent bypass because the embedder panicked, so the failure
    /// direction here is deliberately the opposite of the `NULL` case.
    pub request_navigation: Option<
        unsafe extern "C" fn(
            webview: *mut WebView,
            url_ptr: *const u8,
            url_len: usize,
            user_data: *mut c_void,
        ) -> bool,
    >,

    /// Called when this `WebView` has entered or left fullscreen state.
    ///
    /// This is a notification and cannot be refused: the page enters or leaves
    /// fullscreen internally according to the Fullscreen API regardless of what
    /// the embedder does with it. It exists so that an embedder managing its own
    /// window chrome can transition the containing window to match.
    pub notify_fullscreen_state_changed: Option<
        unsafe extern "C" fn(webview: *mut WebView, is_fullscreen: bool, user_data: *mut c_void),
    >,

    /// Called when the cursor this `WebView` wants to display has changed.
    ///
    /// `cursor` is one of the `SERVO_CURSOR_*` constants.
    ///
    /// # Unknown values
    ///
    /// The set of values can grow, so an embedder may receive a value it does not
    /// know. **`SERVO_CURSOR_DEFAULT` is the designated fallback**: an embedder
    /// must map any unrecognised value to it, and must not transmute or cast the
    /// integer into a cursor type of its own. Servo never sends a value outside
    /// the `SERVO_CURSOR_*` set, but the fallback is what makes a newer payload
    /// safe against an older embedder rather than undefined.
    pub notify_cursor_changed:
        Option<unsafe extern "C" fn(webview: *mut WebView, cursor: u32, user_data: *mut c_void)>,

    /// Called when the page title of this `WebView` has changed.
    ///
    /// `title_ptr`/`title_len` are the title text, borrowed for the duration of
    /// this call only and not NUL-terminated. The embedder must copy the bytes
    /// if it needs to retain them past the call.
    ///
    /// A page with no title is reported as `NULL`/`0`, which is distinct from a
    /// title that is the empty string: that arrives as a non-`NULL` pointer with
    /// a length of zero.
    pub notify_page_title_changed: Option<
        unsafe extern "C" fn(
            webview: *mut WebView,
            title_ptr: *const u8,
            title_len: usize,
            user_data: *mut c_void,
        ),
    >,

    /// Called when page content asks for the window containing this `WebView` to
    /// move, for example through `window.moveTo`.
    ///
    /// This is a request reported as a notification: there is nothing to answer,
    /// and whether to honour it is entirely the embedder's decision. Servo does
    /// not move anything itself and forms no view about whether the position is
    /// reasonable.
    pub request_move_to:
        Option<unsafe extern "C" fn(webview: *mut WebView, x: i32, y: i32, user_data: *mut c_void)>,

    /// Called when page content asks for the window containing this `WebView` to
    /// be resized to the given outer size, for example through `window.resizeTo`.
    ///
    /// Servo guarantees both values are greater than zero but applies no upper
    /// bound; limiting the maximum size is the embedder's job. As with
    /// `request_move_to` there is nothing to answer and Servo resizes nothing
    /// itself.
    pub request_resize_to: Option<
        unsafe extern "C" fn(
            webview: *mut WebView,
            width: i32,
            height: i32,
            user_data: *mut c_void,
        ),
    >,

    /// Called before a `Document` in this `WebView`'s main frame or one of its
    /// nested frames is unloaded, so that the embedder can allow or refuse it.
    /// Return `true` to allow the unload, `false` to refuse it.
    ///
    /// This is the counterpart to `request_navigation`: that one covers arriving
    /// at a document, this one covers leaving it. The same asymmetric defaults
    /// apply for the same reasons — a `NULL` callback **allows** the unload,
    /// matching Servo's own default, while a callback that **unwinds is treated
    /// as a refusal** so that a panicking embedder cannot become a silent
    /// bypass. See `request_navigation` for the full rationale.
    pub request_unload:
        Option<unsafe extern "C" fn(webview: *mut WebView, user_data: *mut c_void) -> bool>,

    /// Called when script asks for a new top-level browsing context, through
    /// `window.open` or a target that names a new context.
    ///
    /// `webview` is the opener. `request` describes what was asked for and is valid
    /// only for the duration of this call.
    ///
    /// Call [`servo_webview_accept_new`] with `request` to accept, or do nothing to
    /// decline. Declining is what an embedder does when the open belongs somewhere
    /// other than a new browsing context: script then sees the blocked open it already
    /// has to handle, and the embedder is free to do what it likes with the requested
    /// URL instead. Nothing links the two, so a URL opened that way is a fresh load,
    /// not a continuation of this one.
    ///
    /// The return value is advisory and is not what decides the outcome: the outcome is
    /// decided by whether the request was claimed. Returning `true` without accepting
    /// still declines.
    ///
    /// # This call blocks the engine
    ///
    /// Servo's constellation is stopped, waiting for the answer, from the moment this is
    /// called until it returns. Whatever the embedder does here — creating a window,
    /// creating a rendering context — happens on the engine's clock, so it should
    /// answer promptly rather than, say, waiting on user input.
    pub request_create_new: Option<
        unsafe extern "C" fn(
            webview: *mut WebView,
            request: *const ServoNewWebViewRequest,
            user_data: *mut c_void,
        ) -> bool,
    >,
}

// Adding a field here changes this number. Update it deliberately, state the
// new size in the specification as part of the ABI contract, and append new
// fields after the existing ones rather than reordering, so that a caller
// built against an older header fails by reading a struct that is too short
// rather than by misreading an existing field.
const _: () = assert!(
    size_of::<ServoWebViewDelegate>() == 128,
    "ServoWebViewDelegate must stay 128 bytes wide"
);

impl WebViewDelegate for ServoWebViewDelegate {
    fn notify_load_status_changed(&self, mut webview: WebView, load_status: LoadStatus) {
        let Some(callback) = self.notify_load_status_changed else {
            return;
        };

        let load_status = load_status as _;

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle.
        unsafe { callback(&mut webview as *mut WebView, load_status, self.user_data) };
    }

    fn notify_new_frame_ready(&self, mut webview: WebView) {
        let Some(callback) = self.notify_new_frame_ready else {
            return;
        };

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle.
        unsafe { callback(&mut webview as *mut WebView, self.user_data) };
    }

    fn notify_crashed(&self, mut webview: WebView, reason: String, backtrace: Option<String>) {
        let Some(callback) = self.notify_crashed else {
            return;
        };

        let (backtrace_ptr, backtrace_len) = match &backtrace {
            Some(backtrace) => (backtrace.as_ptr(), backtrace.len()),
            None => (std::ptr::null(), 0),
        };

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle.
        // `reason` and `backtrace` outlive the call, so the borrowed pointers into
        // them remain valid for its duration.
        unsafe {
            callback(
                &mut webview as *mut WebView,
                reason.as_ptr(),
                reason.len(),
                backtrace_ptr,
                backtrace_len,
                self.user_data,
            )
        };
    }

    fn notify_closed(&self, mut webview: WebView) {
        let Some(callback) = self.notify_closed else {
            return;
        };

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle.
        unsafe { callback(&mut webview as *mut WebView, self.user_data) };
    }

    fn show_console_message(
        &self,
        mut webview: WebView,
        level: servo_api::ConsoleLogLevel,
        message: String,
    ) {
        let Some(callback) = self.show_console_message else {
            return;
        };

        let level = console_log_level_to_c(&level);

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle, and
        // `message` outlives the call, so the borrowed pointer into it remains
        // valid for its duration.
        unsafe {
            callback(
                &mut webview as *mut WebView,
                level,
                message.as_ptr(),
                message.len(),
                self.user_data,
            )
        };
    }

    fn notify_url_changed(&self, mut webview: WebView, url: url::Url) {
        let Some(callback) = self.notify_url_changed else {
            return;
        };

        let url = url.as_str();

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle, and
        // `url` outlives the call, so the borrowed pointer into it remains valid
        // for its duration.
        unsafe {
            callback(
                &mut webview as *mut WebView,
                url.as_ptr(),
                url.len(),
                self.user_data,
            )
        };
    }

    fn request_navigation(&self, mut webview: WebView, navigation_request: NavigationRequest) {
        let Some(callback) = self.request_navigation else {
            // Keep Servo's default path. Dropping the request without answering sends
            // an allow, which is what an unhandled navigation request does upstream.
            return;
        };

        let url = navigation_request.url.as_str();

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle, and
        // `url` outlives the call, so the borrowed pointer into it remains valid
        // for its duration.
        //
        // The callback is contracted not to unwind, but it is contained here anyway
        // so that a panicking embedder cannot unwind into Servo's event loop.
        let allowed = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            callback(
                &mut webview as *mut WebView,
                url.as_ptr(),
                url.len(),
                self.user_data,
            )
        }));

        match allowed {
            Ok(true) => navigation_request.allow(),
            Ok(false) => navigation_request.deny(),
            // The embedder was asked and did not answer. Refuse rather than fall back
            // to the permissive default: this hook exists so that navigations can be
            // vetted, and a panicking callback must not become a silent bypass.
            Err(..) => navigation_request.deny(),
        }
    }

    fn notify_fullscreen_state_changed(&self, mut webview: WebView, is_fullscreen: bool) {
        let Some(callback) = self.notify_fullscreen_state_changed else {
            return;
        };

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle.
        unsafe { callback(&mut webview as *mut WebView, is_fullscreen, self.user_data) };
    }

    fn notify_cursor_changed(&self, mut webview: WebView, cursor: Cursor) {
        let Some(callback) = self.notify_cursor_changed else {
            return;
        };

        // `Cursor` is `#[repr(u8)]`, so its discriminants are part of upstream's
        // layout and can be cast directly, the same way `LoadStatus` is.
        let cursor = cursor as u32;

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle.
        unsafe { callback(&mut webview as *mut WebView, cursor, self.user_data) };
    }

    fn notify_page_title_changed(&self, mut webview: WebView, title: Option<String>) {
        let Some(callback) = self.notify_page_title_changed else {
            return;
        };

        // `title` owns the string for the duration of the callback. A missing title
        // is passed as `NULL`/`0`, which the embedder can tell apart from a title
        // that is present and empty.
        let (title_ptr, title_len) = match title.as_deref() {
            Some(title) => (title.as_ptr(), title.len()),
            None => (std::ptr::null(), 0),
        };

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle, and
        // `title` outlives the call, so the borrowed pointer into it remains valid
        // for its duration.
        unsafe {
            callback(
                &mut webview as *mut WebView,
                title_ptr,
                title_len,
                self.user_data,
            )
        };
    }

    fn request_move_to(&self, mut webview: WebView, point: DeviceIntPoint) {
        let Some(callback) = self.request_move_to else {
            return;
        };

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle.
        unsafe {
            callback(
                &mut webview as *mut WebView,
                point.x,
                point.y,
                self.user_data,
            )
        };
    }

    fn request_resize_to(&self, mut webview: WebView, requested_outer_size: DeviceIntSize) {
        let Some(callback) = self.request_resize_to else {
            return;
        };

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle.
        unsafe {
            callback(
                &mut webview as *mut WebView,
                requested_outer_size.width,
                requested_outer_size.height,
                self.user_data,
            )
        };
    }

    fn request_unload(&self, mut webview: WebView, unload_request: AllowOrDenyRequest) {
        let Some(callback) = self.request_unload else {
            // Keep Servo's default path, which allows the unload.
            return;
        };

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle.
        //
        // The callback is contracted not to unwind, but it is contained here anyway
        // so that a panicking embedder cannot unwind into Servo's event loop.
        let allowed = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            callback(&mut webview as *mut WebView, self.user_data)
        }));

        match allowed {
            Ok(true) => unload_request.allow(),
            Ok(false) => unload_request.deny(),
            // Refused for the same reason as an unanswered navigation: see
            // `request_navigation`.
            Err(..) => unload_request.deny(),
        }
    }

    fn request_create_new(
        &self,
        mut webview: WebView,
        create_new_webview_request: CreateNewWebViewRequest,
    ) {
        let Some(callback) = self.request_create_new else {
            // Keep Servo's default path: dropping the request answers it with `None`,
            // which script sees as a blocked open.
            return;
        };

        // `url` owns the serialization for the duration of the callback, in the same way
        // as every other string this delegate passes out.
        let url = create_new_webview_request
            .requested_url()
            .as_str()
            .to_owned();
        let request = ServoNewWebViewRequest {
            url_ptr: url.as_ptr(),
            url_len: url.len(),
        };

        // Make the request claimable by `servo_webview_accept_new` for the duration of
        // the callback, keyed by the address of the borrowed view the embedder is handed.
        IN_FLIGHT_NEW_WEBVIEW.with(|in_flight| {
            in_flight.borrow_mut().push(InFlightNewWebView {
                view: &request as *const ServoNewWebViewRequest,
                request: Some(create_new_webview_request),
            })
        });

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle, and
        // `request` borrows `url`, which outlives the call.
        //
        // The callback is contracted not to unwind, but it is contained here anyway so
        // that a panicking embedder cannot tear down Servo or strand the request.
        let _advisory = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            callback(
                &mut webview as *mut WebView,
                &request as *const ServoNewWebViewRequest,
                self.user_data,
            )
        }));

        drop(url);

        // Whether the open proceeds is decided by whether the request was claimed, not by
        // the return value. A claimed request has already answered the constellation with
        // the new webview; an unclaimed one is dropped here, which answers `None` and
        // leaves script with the blocked open it already handles.
        let reclaimed = IN_FLIGHT_NEW_WEBVIEW.with(|in_flight| {
            in_flight
                .borrow_mut()
                .pop()
                .and_then(|in_flight| in_flight.request)
        });
        drop(reclaimed);
    }

    fn embedder_control_flags(&self) -> u64 {
        self.controller().map_or(0, |controller| controller.flags)
    }

    fn handle_embedder_control(
        &self,
        mut webview: WebView,
        embedder_control: EmbedderControl,
    ) -> Option<EmbedderControl> {
        let Some(on_control) = self
            .controller()
            .and_then(|controller| controller.on_control)
        else {
            return Some(embedder_control);
        };
        let Some(tag) = embedder_control.tag() else {
            return Some(embedder_control);
        };

        // Servo does not interpret the origin; it is passed through so that the embedder
        // can gate on it. `origin` owns the string for the duration of the callback.
        let origin = webview
            .url()
            .map(|url| url.origin().ascii_serialization())
            .unwrap_or_default();

        let control = ServoEmbedderControl {
            tag: tag as u32,
            origin_ptr: origin.as_ptr(),
            origin_len: origin.len(),
            // Servo never serializes the contents of a control it originated. Payload
            // bytes belong to controls the embedder originates through this same wire
            // format; see `ServoEmbedderControl`.
            payload_ptr: std::ptr::null(),
            payload_len: 0,
        };

        // Make the control claimable by `servo_webview_send_response` for the duration of
        // the callback, keyed by the address of the borrowed view the embedder receives.
        IN_FLIGHT.with(|in_flight| {
            in_flight.borrow_mut().push(InFlightControl {
                view: &control as *const ServoEmbedderControl,
                control: Some(embedder_control),
            })
        });

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoEmbedderController` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle, and
        // `control` borrows `origin`, which outlives the call.
        //
        // The callback is contracted not to unwind, but it is contained here anyway so
        // that a panicking embedder cannot tear down Servo or strand the control: a
        // control that is still unclaimed then follows the default path below.
        let handled = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            on_control(
                &mut webview as *mut WebView,
                &control as *const ServoEmbedderControl,
                self.user_data,
            )
        }))
        .unwrap_or(false);

        drop(origin);

        let reclaimed = IN_FLIGHT.with(|in_flight| {
            in_flight
                .borrow_mut()
                .pop()
                .and_then(|in_flight| in_flight.control)
        });

        let Some(embedder_control) = reclaimed else {
            // The embedder answered through `servo_webview_send_response`, which consumed
            // the control and sent that response. There is no default response left to
            // send and nothing further for Servo to do.
            return None;
        };

        (!handled).then_some(embedder_control)
    }
}

impl ServoWebViewDelegate {
    /// Generic embedder control extension, no domain logic.
    fn controller(&self) -> Option<&ServoEmbedderController> {
        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct, which require `embedder_control` to be either
        // `NULL` or a valid pointer that outlives this delegate.
        unsafe { self.embedder_control.as_ref() }
    }
}

/// Generic embedder control extension, no domain logic.
///
/// The kind of control being offered to the embedder. The numeric values are part of
/// the ABI and must match `EMBEDDER_CONTROL_*` bit positions: a tag occupies bit
/// `tag - 1`.
///
/// `SERVO_EMBEDDER_CONTROL_TAG_WASM_IMPORT` and
/// `SERVO_EMBEDDER_CONTROL_TAG_WEB_MCP_TOOL_CALL` are reserved for controls that the
/// embedder itself originates. Servo never emits them; they exist so that an embedder
/// can reuse this tag space and wire format for its own controls.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum ServoEmbedderControlTag {
    ContextMenu = 1,
    Select = 2,
    FilePicker = 3,
    WasmImport = 4,
    WebMcpToolCall = 5,
}

const _: () = assert!(
    ServoEmbedderControlTag::ContextMenu as u32 == EmbedderControlTag::ContextMenu as u32
        && ServoEmbedderControlTag::Select as u32 == EmbedderControlTag::Select as u32
        && ServoEmbedderControlTag::FilePicker as u32 == EmbedderControlTag::FilePicker as u32
        && ServoEmbedderControlTag::WasmImport as u32 == EmbedderControlTag::WasmImport as u32
        && ServoEmbedderControlTag::WebMcpToolCall as u32
            == EmbedderControlTag::WebMcpToolCall as u32,
    "the C and Rust embedder control tags must agree"
);

/// Generic embedder control extension, no domain logic.
///
/// A control offered to the embedder. This is a borrowed view that is only valid for
/// the duration of the [`ServoEmbedderController::on_control`] call that receives it;
/// the embedder must copy anything it needs to retain.
///
/// `payload_ptr`/`payload_len` are raw bytes. Servo never parses them and never
/// produces them: controls that Servo originates carry a `NULL`, zero-length payload,
/// and the bytes are reserved for controls the embedder originates through this same
/// wire format.
#[repr(C)]
pub struct ServoEmbedderControl {
    /// One of the `ServoEmbedderControlTag` values.
    pub tag: u32,

    /// The origin of the document that triggered this control, as an ASCII
    /// serialization. Not NUL-terminated. May be empty. Servo does not check the
    /// origin; it is passed through so that the embedder can gate on it.
    pub origin_ptr: *const u8,
    /// The length of `origin_ptr` in bytes.
    pub origin_len: usize,

    /// Opaque bytes. May be `NULL`. Servo never parses these.
    pub payload_ptr: *const u8,
    /// The length of `payload_ptr` in bytes.
    pub payload_len: usize,
}

const _: () = assert!(
    size_of::<ServoEmbedderControl>() == 40,
    "ServoEmbedderControl must stay 40 bytes wide"
);

/// Generic embedder control extension, no domain logic.
///
/// An escape hatch that lets the embedder take over selected embedder controls. It is
/// referenced by [`ServoWebViewDelegate::embedder_control`] and is owned by the
/// embedder.
///
/// # Safety
///
/// The embedder must ensure that:
///
/// - This struct outlives every `ServoWebViewDelegate` that points at it.
/// - `on_control`, when non-null, is a valid C ABI callback matching the signature
///   shown, and does not unwind across the FFI boundary.
/// - The `webview` argument is not retained or used after the callback returns, under
///   the same rules as the other `ServoWebViewDelegate` callbacks.
/// - The `control` argument, and the memory it points at, are not used after the
///   callback returns.
#[repr(C)]
pub struct ServoEmbedderController {
    /// A bitmask of `SERVO_EMBEDDER_CONTROL_*` bits. A control whose tag bit is set
    /// here is offered to `on_control`; every other control follows Servo's default
    /// path. `SERVO_EMBEDDER_CONTROL_NONE` (the default) changes nothing.
    pub flags: u64,

    /// Called for a control whose tag bit is set in `flags`. Return `true` to report
    /// the control as handled, in which case Servo does nothing further with it.
    /// Return `false` to decline it and let Servo follow its default path.
    ///
    /// Note that a control reported as handled is dropped by Servo, and dropping a
    /// control sends its default response (a dismissal, or the current selection). To
    /// send some other response instead, call [`servo_webview_send_response`] with the
    /// `control` pointer before returning; that consumes the control, so Servo sends no
    /// default response for it afterwards.
    pub on_control: Option<
        unsafe extern "C" fn(
            webview: *mut WebView,
            control: *const ServoEmbedderControl,
            user_data: *mut c_void,
        ) -> bool,
    >,
}

const _: () = assert!(
    size_of::<ServoEmbedderController>() == 16,
    "ServoEmbedderController must stay 16 bytes wide"
);

/// A request from script for a new top-level browsing context, as passed to
/// [`ServoWebViewDelegate::request_create_new`].
///
/// This is a borrowed view that is valid only for the duration of that call; the
/// embedder must copy anything it needs to retain and must not use the pointer after the
/// callback returns.
#[repr(C)]
pub struct ServoNewWebViewRequest {
    /// The URL the open requested, as a serialization. Not NUL-terminated, and borrowed
    /// for the duration of the callback only.
    pub url_ptr: *const u8,
    /// The length of `url_ptr` in bytes.
    pub url_len: usize,
}

const _: () = assert!(
    size_of::<ServoNewWebViewRequest>() == 16,
    "ServoNewWebViewRequest must stay 16 bytes wide"
);

/// A new-browsing-context request currently being offered to
/// [`ServoWebViewDelegate::request_create_new`] and not yet claimed, together with the
/// address of the borrowed [`ServoNewWebViewRequest`] view the embedder was handed. The
/// view address is what [`servo_webview_accept_new`] matches on, the same way
/// [`servo_webview_send_response`] matches a control.
struct InFlightNewWebView {
    view: *const ServoNewWebViewRequest,
    /// `None` once the embedder has claimed it.
    request: Option<CreateNewWebViewRequest>,
}

thread_local! {
    /// The new-browsing-context requests currently being offered on this thread,
    /// innermost last, so that an embedder whose handling of one open triggers another
    /// cannot strand or misroute the outer request.
    static IN_FLIGHT_NEW_WEBVIEW: RefCell<Vec<InFlightNewWebView>> =
        const { RefCell::new(Vec::new()) };
}

/// Accept the new-browsing-context request currently being offered to
/// [`ServoWebViewDelegate::request_create_new`], creating the `WebView` for it.
///
/// `request` is the pointer that callback received. `context` is the rendering context
/// the new `WebView` draws into, which the embedder has created on a window it owns;
/// **ownership of `context` transfers to this function** on success, as it does for
/// `servo_webview_builder_create`. `delegate`, when non-null, is copied out and becomes
/// the new `WebView`'s delegate; pass `NULL` to create it without one.
///
/// Returns the new `WebView`, whose ownership transfers to the caller and which must be
/// freed with `servo_webview_free`.
///
/// Returns `NULL` **without consuming `context`** if `request` or `context` is `NULL`,
/// if `request` does not name a request currently being offered on this thread, or if
/// that request has already been claimed — so a failed call leaves the caller's context
/// theirs to reuse or free.
///
/// The returned `WebView` carries a user-content manager, so `servo_webview_add_script`
/// works on it, exactly as it does for one built through `servo_webview_builder_build`.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - This is called from inside `request_create_new`, on the same thread, with the
///   `request` pointer that call received, and not after it has returned.
/// - `context` is a non-null pointer to a `RenderingContext` previously returned by one
///   of the `servo_rendering_context_create_*` functions, not yet freed nor passed to
///   another function that takes ownership of it.
/// - `delegate`, when non-null, points to a valid `ServoWebViewDelegate` whose function
///   pointers and `user_data` remain valid for as long as the returned `WebView` lives.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_accept_new(
    request: *const ServoNewWebViewRequest,
    context: *mut RenderingContext,
    delegate: *const ServoWebViewDelegate,
) -> *mut WebView {
    if request.is_null() || context.is_null() {
        return std::ptr::null_mut();
    }

    // Claim the request before taking ownership of anything, so that a call naming no
    // live request leaves the caller's context untouched for them to reuse or free.
    let claimed = IN_FLIGHT_NEW_WEBVIEW.with(|in_flight| {
        let mut in_flight = in_flight.borrow_mut();
        in_flight
            .iter_mut()
            .rev()
            .find(|slot| std::ptr::eq(slot.view, request))
            .and_then(|slot| slot.request.take())
    });
    let Some(claimed) = claimed else {
        return std::ptr::null_mut();
    };

    // `builder` consumes the request, so take the instance first: the user-content
    // manager below needs it.
    let servo = claimed.servo().clone();

    // SAFETY: The caller is assumed to uphold the safety requirements documented above.
    // Ownership of `context` transfers here.
    let rendering_context = unsafe { Box::from_raw(context) }.inner;

    let mut builder = claimed.builder(rendering_context);

    // SAFETY: As documented above, `delegate` is either null or a valid
    // `ServoWebViewDelegate`. It is `Copy`, so this snapshots it rather than aliasing
    // the embedder's storage.
    if let Some(delegate) = unsafe { delegate.as_ref() } {
        builder = builder.delegate(std::rc::Rc::new(*delegate));
    }

    // Matching `servo_webview_builder_build`: every `WebView` gets a user-content
    // manager, so that `servo_webview_add_script` works on it afterwards.
    builder = builder.user_content_manager(std::rc::Rc::new(UserContentManager::new(&servo)));

    Box::into_raw(Box::new(builder.build()))
}

/// Generic embedder control extension, no domain logic.
///
/// A control that has been offered to [`ServoEmbedderController::on_control`] and is
/// still unanswered, together with the address of the borrowed [`ServoEmbedderControl`]
/// view that the embedder was handed for it. The view address is what
/// [`servo_webview_send_response`] matches on, which keeps the response path off the
/// 40-byte `ServoEmbedderControl` ABI.
struct InFlightControl {
    view: *const ServoEmbedderControl,
    /// `None` once the embedder has claimed the control with a response.
    control: Option<EmbedderControl>,
}

thread_local! {
    /// The controls currently being offered on this thread, innermost last. This is a
    /// stack rather than a single slot so that an embedder that re-enters Servo from
    /// `on_control` cannot strand or misroute the outer control.
    static IN_FLIGHT: RefCell<Vec<InFlightControl>> = const { RefCell::new(Vec::new()) };
}

/// Generic embedder control extension, no domain logic.
///
/// The action carried by a `SERVO_EMBEDDER_CONTROL_TAG_CONTEXT_MENU` response payload.
/// The numeric values are part of the ABI. They are Servo's built-in context menu
/// actions; an embedder that shows its own menu entries maps them onto these itself.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum ServoContextMenuAction {
    GoBack = 1,
    GoForward = 2,
    Reload = 3,
    CopyLink = 4,
    OpenLinkInNewWebView = 5,
    CopyImageLink = 6,
    OpenImageInNewView = 7,
    Cut = 8,
    Copy = 9,
    Paste = 10,
    SelectAll = 11,
}

/// Translate a wire action code into the action Servo acts on, or `None` if the code is
/// not one of the [`ServoContextMenuAction`] values.
fn context_menu_action(code: u32) -> Option<ContextMenuAction> {
    Some(match code {
        code if code == ServoContextMenuAction::GoBack as u32 => ContextMenuAction::GoBack,
        code if code == ServoContextMenuAction::GoForward as u32 => ContextMenuAction::GoForward,
        code if code == ServoContextMenuAction::Reload as u32 => ContextMenuAction::Reload,
        code if code == ServoContextMenuAction::CopyLink as u32 => ContextMenuAction::CopyLink,
        code if code == ServoContextMenuAction::OpenLinkInNewWebView as u32 => {
            ContextMenuAction::OpenLinkInNewWebView
        },
        code if code == ServoContextMenuAction::CopyImageLink as u32 => {
            ContextMenuAction::CopyImageLink
        },
        code if code == ServoContextMenuAction::OpenImageInNewView as u32 => {
            ContextMenuAction::OpenImageInNewView
        },
        code if code == ServoContextMenuAction::Cut as u32 => ContextMenuAction::Cut,
        code if code == ServoContextMenuAction::Copy as u32 => ContextMenuAction::Copy,
        code if code == ServoContextMenuAction::Paste as u32 => ContextMenuAction::Paste,
        code if code == ServoContextMenuAction::SelectAll as u32 => ContextMenuAction::SelectAll,
        _ => return None,
    })
}

/// Decode `payload` as a sequence of little-endian `u32`s, or `None` if its length is not
/// a whole number of them.
fn decode_u32s(payload: &[u8]) -> Option<Vec<u32>> {
    if payload.len() % size_of::<u32>() != 0 {
        return None;
    }
    Some(
        payload
            .chunks_exact(size_of::<u32>())
            .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect(),
    )
}

/// The number of selectable options in `options`, flattening `<optgroup>` children, which
/// is the index space that a `SERVO_EMBEDDER_CONTROL_TAG_SELECT` response indexes into.
fn flattened_option_count(options: &[SelectElementOptionOrOptgroup]) -> usize {
    options
        .iter()
        .map(|option| match option {
            SelectElementOptionOrOptgroup::Option(..) => 1,
            SelectElementOptionOrOptgroup::Optgroup { options, .. } => options.len(),
        })
        .sum()
}

/// Decode `payload` for `embedder_control` and send the resulting response, consuming the
/// control. Returns the control untouched if `payload` does not decode for this kind of
/// control, so that the caller can leave Servo's default path intact.
///
/// This is transport only: Servo attaches no meaning to the payload beyond the fixed wire
/// format documented on [`servo_webview_send_response`], and applies no policy of its own
/// to what the embedder answers.
fn send_control_response(
    embedder_control: EmbedderControl,
    payload: &[u8],
) -> Result<(), EmbedderControl> {
    match embedder_control {
        EmbedderControl::ContextMenu(context_menu) => {
            if payload.is_empty() {
                context_menu.dismiss();
                return Ok(());
            }
            let action = decode_u32s(payload)
                .filter(|codes| codes.len() == 1)
                .and_then(|codes| context_menu_action(codes[0]));
            let Some(action) = action else {
                return Err(EmbedderControl::ContextMenu(context_menu));
            };
            context_menu.select(action);
            Ok(())
        },
        EmbedderControl::SelectElement(mut select_element) => {
            let Some(indices) = decode_u32s(payload) else {
                return Err(EmbedderControl::SelectElement(select_element));
            };
            let option_count = flattened_option_count(select_element.options());
            if indices.iter().any(|index| *index as usize >= option_count) {
                return Err(EmbedderControl::SelectElement(select_element));
            }
            if indices.len() > 1 && !select_element.allow_select_multiple() {
                return Err(EmbedderControl::SelectElement(select_element));
            }
            select_element.select(indices.into_iter().map(|index| index as usize).collect());
            select_element.submit();
            Ok(())
        },
        EmbedderControl::FilePicker(file_picker) => {
            // Only an explicit dismissal is expressible on this wire format; selecting
            // files would mean transporting paths, which this ABI does not define.
            if !payload.is_empty() {
                return Err(EmbedderControl::FilePicker(file_picker));
            }
            file_picker.dismiss();
            Ok(())
        },
        // Every other kind of control is untagged, so it is never offered to the embedder
        // and can never reach this point.
        embedder_control => Err(embedder_control),
    }
}

/// Claim the in-flight control that `view` refers to and answer it with `payload`.
///
/// Returns `false`, leaving the control unclaimed, if `view` names no control currently
/// being offered on this thread, if that control has already been answered, or if
/// `payload` does not decode for that kind of control.
///
/// # Safety
///
/// `payload` must be valid for reads of `payload_len` bytes when `payload_len` is
/// non-zero.
unsafe fn claim_and_respond(
    view: *const ServoEmbedderControl,
    payload: *const u8,
    payload_len: usize,
) -> bool {
    if view.is_null() {
        return false;
    }

    let payload: &[u8] = if payload_len == 0 {
        &[]
    } else if payload.is_null() {
        return false;
    } else {
        // SAFETY: The caller guarantees that `payload` is valid for reads of
        // `payload_len` bytes, and the slice is not held past this call.
        unsafe { std::slice::from_raw_parts(payload, payload_len) }
    };

    IN_FLIGHT.with(|in_flight| {
        let mut in_flight = in_flight.borrow_mut();
        // Innermost first, so a re-entrant embedder answers the control it was handed.
        let Some(slot) = in_flight
            .iter_mut()
            .rev()
            .find(|slot| std::ptr::eq(slot.view, view))
        else {
            return false;
        };
        let Some(embedder_control) = slot.control.take() else {
            return false;
        };

        // Sending a response only hands a message to the constellation, so this cannot
        // re-enter `IN_FLIGHT` while the borrow above is held.
        match send_control_response(embedder_control, payload) {
            Ok(()) => true,
            Err(embedder_control) => {
                // Undecodable payload: put the control back so that declining it from
                // `on_control` still reaches Servo's default path.
                slot.control = Some(embedder_control);
                false
            },
        }
    })
}

/// Generic embedder control extension, no domain logic.
///
/// Answer the embedder control that is currently being offered to
/// [`ServoEmbedderController::on_control`], instead of the default response that Servo
/// would otherwise send for it.
///
/// `control` is the pointer that `on_control` received. A successful call consumes the
/// control: the response is sent, Servo sends no default response for it, and the control
/// does not follow Servo's default path even if `on_control` goes on to return `false`.
///
/// This is transport only. Servo does not inspect the origin of the control and applies
/// no policy to the response; deciding which controls may be answered, and with what, is
/// the embedder's job.
///
/// # Payload
///
/// `payload` is a fixed wire format that depends on the control's tag. Integers are
/// little-endian.
///
/// - `SERVO_EMBEDDER_CONTROL_TAG_CONTEXT_MENU`: empty for a dismissal with no selection,
///   or one `uint32_t` [`ServoContextMenuAction`] code for a selection.
/// - `SERVO_EMBEDDER_CONTROL_TAG_SELECT`: zero or more `uint32_t` indices into the
///   flattened option list of the `<select>`, which replace its selection. An empty
///   payload deselects every option. More than one index is only accepted for a
///   `<select multiple>`.
/// - `SERVO_EMBEDDER_CONTROL_TAG_FILE_PICKER`: empty, for a dismissal with no files.
///   Selecting files is not expressible on this wire format.
///
/// # Return value
///
/// `true` if the response was sent and the control consumed. `false` if it was not, in
/// which case nothing has been sent and the control is left exactly as it was: the
/// embedder can still return `false` from `on_control` to hand it back to Servo. A call
/// returns `false` when `control` is `NULL` or does not name a control currently being
/// offered on this thread, when that control has already been answered, or when `payload`
/// does not decode for that kind of control.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - This is called from inside `on_control`, on the same thread, with the `control`
///   pointer that call received, and not after it has returned.
/// - `payload` is either `NULL` with a `payload_len` of zero, or valid for reads of
///   `payload_len` bytes for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_send_response(
    control: *const ServoEmbedderControl,
    payload: *const u8,
    payload_len: usize,
) -> bool {
    // A panic must not unwind into the embedder's C frame. Containing it here also keeps
    // the control unclaimed, so the caller can still fall back to Servo's default path.
    panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The caller is assumed to uphold the safety requirements documented
        // above, which include those of `claim_and_respond`.
        unsafe { claim_and_respond(control, payload, payload_len) }
    }))
    .unwrap_or(false)
}

/// Take no control; every control follows Servo's default path.
pub const SERVO_EMBEDDER_CONTROL_NONE: u64 = 0;
/// A context menu opened on web content.
pub const SERVO_EMBEDDER_CONTROL_CONTEXT_MENU: u64 = 1 << 0;
/// The picker of a `<select>` element.
pub const SERVO_EMBEDDER_CONTROL_SELECT: u64 = 1 << 1;
/// The picker of an `<input type=file>` element.
pub const SERVO_EMBEDDER_CONTROL_FILE_PICKER: u64 = 1 << 2;
/// Reserved for embedder-originated controls.
pub const SERVO_EMBEDDER_CONTROL_WASM_IMPORT: u64 = 1 << 3;
/// Reserved for embedder-originated controls.
pub const SERVO_EMBEDDER_CONTROL_WEBMCP: u64 = 1 << 4;
/// Take every taggable control, including bits not yet assigned.
pub const SERVO_EMBEDDER_CONTROL_ALL: u64 = 0xFFFF_FFFF;

/// The severity of a message logged by page content, passed to
/// [`ServoWebViewDelegate::show_console_message`].
///
/// `servo_api::ConsoleLogLevel` carries no `#[repr]`, so its Rust layout is
/// not part of any ABI. These constants and [`console_log_level_to_c`] are
/// this crate's own stable numbering, kept in sync with the Rust enum by an
/// exhaustive match rather than by a numeric cast.
pub const SERVO_CONSOLE_LOG_LEVEL_LOG: i32 = 0;
pub const SERVO_CONSOLE_LOG_LEVEL_DEBUG: i32 = 1;
pub const SERVO_CONSOLE_LOG_LEVEL_INFO: i32 = 2;
pub const SERVO_CONSOLE_LOG_LEVEL_WARN: i32 = 3;
pub const SERVO_CONSOLE_LOG_LEVEL_ERROR: i32 = 4;
pub const SERVO_CONSOLE_LOG_LEVEL_TRACE: i32 = 5;
pub const SERVO_CONSOLE_LOG_LEVEL_DIR: i32 = 6;

/// Converts a [`servo_api::ConsoleLogLevel`] to one of the
/// `SERVO_CONSOLE_LOG_LEVEL_*` constants.
///
/// An exhaustive match, not a cast: `ConsoleLogLevel` has no stable repr, so
/// this is the only sound way to produce a value that means the same thing on
/// both sides of the ABI. Adding a variant upstream is a compile error here
/// rather than a silent renumbering.
fn console_log_level_to_c(level: &servo_api::ConsoleLogLevel) -> i32 {
    use servo_api::ConsoleLogLevel::*;
    match level {
        Log => SERVO_CONSOLE_LOG_LEVEL_LOG,
        Debug => SERVO_CONSOLE_LOG_LEVEL_DEBUG,
        Info => SERVO_CONSOLE_LOG_LEVEL_INFO,
        Warn => SERVO_CONSOLE_LOG_LEVEL_WARN,
        Error => SERVO_CONSOLE_LOG_LEVEL_ERROR,
        Trace => SERVO_CONSOLE_LOG_LEVEL_TRACE,
        Dir => SERVO_CONSOLE_LOG_LEVEL_DIR,
    }
}

#[cfg(test)]
mod tests {
    use servo_api::{ContextMenuAction, SelectElementOption, SelectElementOptionOrOptgroup};

    use super::{
        ServoContextMenuAction, ServoEmbedderControl, ServoEmbedderControlTag, context_menu_action,
        decode_u32s, flattened_option_count, servo_webview_send_response,
    };

    fn option(label: &str) -> SelectElementOption {
        SelectElementOption {
            id: Default::default(),
            label: label.to_owned(),
            is_disabled: false,
        }
    }

    #[test]
    fn decodes_little_endian_u32s() {
        assert_eq!(decode_u32s(&[]), Some(vec![]));
        assert_eq!(decode_u32s(&[1, 0, 0, 0]), Some(vec![1]));
        assert_eq!(decode_u32s(&[1, 0, 0, 0, 2, 0, 0, 0]), Some(vec![1, 2]));

        // A payload that is not a whole number of `u32`s is rejected rather than
        // truncated, so a malformed response never reaches web content.
        assert_eq!(decode_u32s(&[1]), None);
        assert_eq!(decode_u32s(&[1, 0, 0]), None);
        assert_eq!(decode_u32s(&[1, 0, 0, 0, 2]), None);
    }

    #[test]
    fn context_menu_action_codes_are_total_and_bounded() {
        assert_eq!(context_menu_action(0), None);
        assert_eq!(context_menu_action(12), None);
        assert_eq!(context_menu_action(u32::MAX), None);

        assert_eq!(context_menu_action(1), Some(ContextMenuAction::GoBack));
        assert_eq!(context_menu_action(11), Some(ContextMenuAction::SelectAll));
        assert_eq!(
            context_menu_action(ServoContextMenuAction::Copy as u32),
            Some(ContextMenuAction::Copy)
        );
    }

    #[test]
    fn option_count_flattens_optgroups() {
        // The index space of a select response is the flattened option list, not the
        // group-or-option list, so bounds checks have to flatten too.
        let options = vec![
            SelectElementOptionOrOptgroup::Option(option("a")),
            SelectElementOptionOrOptgroup::Optgroup {
                label: "group".to_owned(),
                options: vec![option("b"), option("c")],
            },
            SelectElementOptionOrOptgroup::Option(option("d")),
        ];

        assert_eq!(options.len(), 3);
        assert_eq!(flattened_option_count(&options), 4);
        assert_eq!(flattened_option_count(&[]), 0);
    }

    #[test]
    fn responding_without_a_live_control_is_refused() {
        // There is no control in flight on this thread, so every call must decline and
        // leave the embedder free to fall back to Servo's default path.
        let payload = (ServoContextMenuAction::Copy as u32).to_le_bytes();

        // SAFETY: A null control, and a payload that is either empty or a valid slice.
        unsafe {
            assert!(!servo_webview_send_response(
                std::ptr::null(),
                payload.as_ptr(),
                payload.len()
            ));

            let control = ServoEmbedderControl {
                tag: ServoEmbedderControlTag::ContextMenu as u32,
                origin_ptr: std::ptr::null(),
                origin_len: 0,
                payload_ptr: std::ptr::null(),
                payload_len: 0,
            };
            assert!(!servo_webview_send_response(
                &control as *const ServoEmbedderControl,
                payload.as_ptr(),
                payload.len()
            ));

            // A non-null length with a null payload is refused rather than dereferenced.
            assert!(!servo_webview_send_response(
                &control as *const ServoEmbedderControl,
                std::ptr::null(),
                4
            ));
        }
    }
}

/// The cursor values passed to [`ServoWebViewDelegate::notify_cursor_changed`].
///
/// These mirror the discriminants of upstream's `Cursor`, which is `#[repr(u8)]`, so the
/// numbering is upstream's rather than this crate's. The compile-time assertion below is
/// what keeps them in step: a variant inserted upstream rather than appended renumbers the
/// set and fails the build here instead of silently changing what a value means.
///
/// `SERVO_CURSOR_DEFAULT` is the fallback an embedder must use for a value it does not
/// recognise. It is upstream's own `#[default]` variant, so it is the value that already
/// means "whatever this platform's ordinary pointer is" rather than a sentinel invented
/// here. Casting an unrecognised value into an embedder-side enum instead is undefined on
/// the embedder's side, which is the whole reason this constant is named.
pub const SERVO_CURSOR_NONE: u32 = Cursor::None as u32;
pub const SERVO_CURSOR_DEFAULT: u32 = Cursor::Default as u32;
pub const SERVO_CURSOR_POINTER: u32 = Cursor::Pointer as u32;
pub const SERVO_CURSOR_CONTEXT_MENU: u32 = Cursor::ContextMenu as u32;
pub const SERVO_CURSOR_HELP: u32 = Cursor::Help as u32;
pub const SERVO_CURSOR_PROGRESS: u32 = Cursor::Progress as u32;
pub const SERVO_CURSOR_WAIT: u32 = Cursor::Wait as u32;
pub const SERVO_CURSOR_CELL: u32 = Cursor::Cell as u32;
pub const SERVO_CURSOR_CROSSHAIR: u32 = Cursor::Crosshair as u32;
pub const SERVO_CURSOR_TEXT: u32 = Cursor::Text as u32;
pub const SERVO_CURSOR_VERTICAL_TEXT: u32 = Cursor::VerticalText as u32;
pub const SERVO_CURSOR_ALIAS: u32 = Cursor::Alias as u32;
pub const SERVO_CURSOR_COPY: u32 = Cursor::Copy as u32;
pub const SERVO_CURSOR_MOVE: u32 = Cursor::Move as u32;
pub const SERVO_CURSOR_NO_DROP: u32 = Cursor::NoDrop as u32;
pub const SERVO_CURSOR_NOT_ALLOWED: u32 = Cursor::NotAllowed as u32;
pub const SERVO_CURSOR_GRAB: u32 = Cursor::Grab as u32;
pub const SERVO_CURSOR_GRABBING: u32 = Cursor::Grabbing as u32;
pub const SERVO_CURSOR_E_RESIZE: u32 = Cursor::EResize as u32;
pub const SERVO_CURSOR_N_RESIZE: u32 = Cursor::NResize as u32;
pub const SERVO_CURSOR_NE_RESIZE: u32 = Cursor::NeResize as u32;
pub const SERVO_CURSOR_NW_RESIZE: u32 = Cursor::NwResize as u32;
pub const SERVO_CURSOR_S_RESIZE: u32 = Cursor::SResize as u32;
pub const SERVO_CURSOR_SE_RESIZE: u32 = Cursor::SeResize as u32;
pub const SERVO_CURSOR_SW_RESIZE: u32 = Cursor::SwResize as u32;
pub const SERVO_CURSOR_W_RESIZE: u32 = Cursor::WResize as u32;
pub const SERVO_CURSOR_EW_RESIZE: u32 = Cursor::EwResize as u32;
pub const SERVO_CURSOR_NS_RESIZE: u32 = Cursor::NsResize as u32;
pub const SERVO_CURSOR_NESW_RESIZE: u32 = Cursor::NeswResize as u32;
pub const SERVO_CURSOR_NWSE_RESIZE: u32 = Cursor::NwseResize as u32;
pub const SERVO_CURSOR_COL_RESIZE: u32 = Cursor::ColResize as u32;
pub const SERVO_CURSOR_ROW_RESIZE: u32 = Cursor::RowResize as u32;
pub const SERVO_CURSOR_ALL_SCROLL: u32 = Cursor::AllScroll as u32;
pub const SERVO_CURSOR_ZOOM_IN: u32 = Cursor::ZoomIn as u32;
pub const SERVO_CURSOR_ZOOM_OUT: u32 = Cursor::ZoomOut as u32;

const _: () = assert!(
    SERVO_CURSOR_NONE == 0 && SERVO_CURSOR_DEFAULT == 1 && SERVO_CURSOR_ZOOM_OUT == 34,
    "the SERVO_CURSOR_* values are part of the ABI and must not be renumbered"
);
