/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::panic::{self, AssertUnwindSafe};

pub use servo_api::LoadStatus;
use servo_api::{
    AllowOrDenyRequest, BrowsingContextId, ContextMenuAction, CreateNewWebViewRequest, Cursor,
    DeviceIntPoint, DeviceIntSize, EmbedderControl, EmbedderControlTag, FrameEvent, FrameEventKind,
    ImmutableOrigin, NavigationRequest, PermissionRequest, SelectElementOptionOrOptgroup,
    UserContentManager, WebResourceLoad, WebResourceResponse, WebView, WebViewDelegate, WebViewId,
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
    /// With multiprocess on, a content process that exits for any reason, including
    /// being killed from outside, is reported here for every `WebView` whose active
    /// document it was serving, as soon as the engine sees the process go. The engine
    /// then replaces each such document with its crash page; `notify_frame_changed`
    /// reports that replacement like any other navigation. Reloading is the way to
    /// recover. A content process that is merely hung is not reported: nothing in the
    /// engine interrupts one, and this fires only when the process is gone.
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
    /// `webview` is the opener. `request` is an opaque token, valid only for the
    /// duration of this call.
    ///
    /// Call [`servo_webview_accept_new`] with `request` to accept, or do nothing to
    /// decline. Declining means script sees the blocked open it already has to handle,
    /// and no browsing context is created.
    ///
    /// # This callback cannot tell you where the popup is going
    ///
    /// A new browsing context is created on `about:blank` and navigated afterwards, so
    /// the URL the opener asked for does not exist anywhere in the engine yet. There is
    /// nothing to pass here and nothing to gate on.
    ///
    /// Where it does appear: **an accepted context's first navigation is the URL the
    /// opener asked for**, delivered to `request_navigation` on the delegate passed to
    /// `servo_webview_accept_new` — which returns `bool`, so it can be refused. The
    /// exception is an open that named no URL, or named `about:blank`: then there is no
    /// subsequent navigation at all and the context stays on `about:blank`.
    ///
    /// So an embedder that must see the URL before committing to a window accepts into
    /// a context it is willing to throw away, reads the URL from that first
    /// `request_navigation`, refuses it, and frees the `WebView` — having fetched
    /// nothing. Declining here fetches nothing either, but yields no URL.
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

    /// Called when page content asks for a permission, for example through
    /// `navigator.permissions.request()`, `Notification.requestPermission()` or
    /// `navigator.wakeLock.request()`.
    ///
    /// `request` is a view borrowed for the duration of this call; copy what you need
    /// from it. Its `id` is what answers the request: pass it to
    /// [`servo_webview_respond_permission`], during this call or at any later point,
    /// on this same thread. Each request must be answered exactly once.
    ///
    /// # The page waits for the answer
    ///
    /// The requesting script is blocked until the request is answered, but the
    /// embedder is not: an answer can wait on the person. Wake lock requests are the
    /// exception, settling a promise instead of blocking.
    ///
    /// # Defaults
    ///
    /// A `NULL` callback **denies** every request, matching Servo's own default. A
    /// callback that unwinds denies the request it was handed, and its `id` stops
    /// naming anything. The two origins in the view are engine-supplied; see
    /// [`ServoPermissionRequest`].
    pub request_permission: Option<
        unsafe extern "C" fn(
            webview: *mut WebView,
            request: *const ServoPermissionRequest,
            user_data: *mut c_void,
        ),
    >,

    /// Called before a copy or paste gesture against the OS clipboard — a keyboard
    /// shortcut, a right-click copy/paste, or `document.execCommand('copy'/'paste')` —
    /// from any frame in this `WebView`, so the embedder can allow or refuse it.
    /// Return `true` to allow, `false` to refuse.
    ///
    /// Same shape as `request_unload`: a `NULL` callback **allows** clipboard access,
    /// matching every other ungated default in this ABI, while a callback that
    /// **unwinds is treated as a refusal** so that a panicking embedder cannot become a
    /// silent bypass. Clipboard access itself needs no engine work to gate here — it
    /// already reaches the real OS clipboard without this field set at all; setting it
    /// is what adds the per-view policy point.
    pub request_clipboard_access:
        Option<unsafe extern "C" fn(webview: *mut WebView, user_data: *mut c_void) -> bool>,

    /// Called when a frame in this `WebView` is created, navigates, or is removed,
    /// so that the embedder can keep its own record of the frame tree.
    ///
    /// `browsing_context_id` identifies the frame by its browsing context, the frame
    /// slot, which keeps its identity across every navigation inside it. A frame whose
    /// origin serializes to `"null"` (`srcdoc`, `about:blank`, sandboxed) still has
    /// its own id. `parent_browsing_context_id` is the frame's parent, or `0` for the
    /// top-level frame, and is given on every event. Both are opaque: compare them
    /// and use them as keys, nothing else. `0` is never a real frame.
    ///
    /// `kind` is one of the `SERVO_FRAME_EVENT_*` constants. `url_ptr`/`url_len`
    /// are the frame's URL for `CREATED` and `NAVIGATED`, borrowed for the duration
    /// of this call and not NUL-terminated, and `NULL`/`0` for `REMOVED`.
    /// `NAVIGATED` fires when a new document becomes active in the frame, which
    /// includes the frame's first document, a navigation that replaces the current
    /// history entry, and back/forward.
    ///
    /// # Ordering
    ///
    /// - A frame's `CREATED` arrives before any permission request, control or page
    ///   request from that frame.
    /// - A frame's `CREATED` arrives before its `NAVIGATED` and `REMOVED` events.
    /// - Removing a frame emits one `REMOVED` per descendant, children before their
    ///   parent, then one for the frame itself.
    ///
    /// All three hold in a single-process build. With content in separate processes,
    /// the first still holds for anything the frame sends from the content process it
    /// was created in, but not after it navigates into a different one; the second is
    /// not guaranteed at all. `parent_browsing_context_id` is on every event so that an
    /// embedder can place a frame from whichever event reaches it first.
    ///
    /// # Gaps
    ///
    /// - A frame removed before its first document became active gets `CREATED` but no
    ///   `REMOVED`. Ids are never reused, so its stale entry can never be mistaken for
    ///   another frame.
    /// - No events are delivered for a `WebView` the embedder has already freed.
    pub notify_frame_changed: Option<
        unsafe extern "C" fn(
            webview: *mut WebView,
            browsing_context_id: u64,
            parent_browsing_context_id: u64,
            kind: u8,
            url_ptr: *const u8,
            url_len: usize,
            user_data: *mut c_void,
        ),
    >,
}

thread_local! {
    /// The resource request callback of each `WebView` whose delegate is a
    /// `ServoWebViewDelegate`, keyed by `WebView`. Only those can receive one: the
    /// hook is read from `ServoWebViewDelegate::load_web_resource`, and a `WebView`'s
    /// delegate is fixed when it is built. An entry of `None` is such a `WebView`
    /// with no callback set, which keeps Servo's default and lets every request
    /// through. A `WebView` with no entry has some other delegate.
    static RESOURCE_REQUEST_HOOKS: RefCell<HashMap<WebViewId, Option<ResourceRequestHook>>> =
        RefCell::new(HashMap::new());
}

thread_local! {
    /// The content process of each `WebView`'s active top-level document, keyed by
    /// `WebView`. Written by `notify_frame_changed` and read by
    /// `servo_webview_content_process_id`, both on the embedder's thread. Absent means
    /// no out-of-process document is active, which reads as 0.
    static TOP_LEVEL_CONTENT_PROCESS: RefCell<HashMap<WebViewId, u32>> =
        RefCell::new(HashMap::new());
}

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

    fn notify_crashed(&self, webview: WebView, reason: String, backtrace: Option<String>) {
        self.open_letter_notify_crashed(webview, reason, backtrace)
    }

    fn notify_closed(&self, webview: WebView) {
        self.open_letter_notify_closed(webview)
    }

    fn show_console_message(&self, webview: WebView, level: servo_api::ConsoleLogLevel, message: String) {
        self.open_letter_show_console_message(webview, level, message)
    }

    fn notify_url_changed(&self, webview: WebView, url: url::Url) {
        self.open_letter_notify_url_changed(webview, url)
    }

    fn request_navigation(&self, webview: WebView, navigation_request: NavigationRequest) {
        self.open_letter_request_navigation(webview, navigation_request)
    }

    fn load_web_resource(&self, webview: WebView, load: WebResourceLoad) {
        self.open_letter_load_web_resource(webview, load)
    }

    fn notify_fullscreen_state_changed(&self, webview: WebView, is_fullscreen: bool) {
        self.open_letter_notify_fullscreen_state_changed(webview, is_fullscreen)
    }

    fn notify_cursor_changed(&self, webview: WebView, cursor: Cursor) {
        self.open_letter_notify_cursor_changed(webview, cursor)
    }

    fn notify_page_title_changed(&self, webview: WebView, title: Option<String>) {
        self.open_letter_notify_page_title_changed(webview, title)
    }

    fn request_move_to(&self, webview: WebView, point: DeviceIntPoint) {
        self.open_letter_request_move_to(webview, point)
    }

    fn request_resize_to(&self, webview: WebView, requested_outer_size: DeviceIntSize) {
        self.open_letter_request_resize_to(webview, requested_outer_size)
    }

    fn notify_frame_changed(&self, webview: WebView, event: FrameEvent) {
        self.open_letter_notify_frame_changed(webview, event)
    }

    fn request_unload(&self, webview: WebView, unload_request: AllowOrDenyRequest) {
        self.open_letter_request_unload(webview, unload_request)
    }

    fn request_permission(&self, webview: WebView, permission_request: PermissionRequest) {
        self.open_letter_request_permission(webview, permission_request)
    }

    fn request_create_new(&self, webview: WebView, create_new_webview_request: CreateNewWebViewRequest) {
        self.open_letter_request_create_new(webview, create_new_webview_request)
    }

    fn embedder_control_flags(&self) -> u64 {
        self.open_letter_embedder_control_flags()
    }

    fn handle_embedder_control(&self, webview: WebView, embedder_control: EmbedderControl, requester_origin: Option<&ImmutableOrigin>) -> Option<EmbedderControl> {
        self.open_letter_handle_embedder_control(webview, embedder_control, requester_origin)
    }
}

thread_local! {
    /// The new-browsing-context requests currently being offered on this thread,
    /// innermost last, so that an embedder whose handling of one open triggers another
    /// cannot strand or misroute the outer request.
    static IN_FLIGHT_NEW_WEBVIEW: RefCell<Vec<InFlightNewWebView>> =
        const { RefCell::new(Vec::new()) };
}

thread_local! {
    /// The controls currently being offered on this thread, innermost last. This is a
    /// stack rather than a single slot so that an embedder that re-enters Servo from
    /// `on_control` cannot strand or misroute the outer control.
    static IN_FLIGHT: RefCell<Vec<InFlightControl>> = const { RefCell::new(Vec::new()) };
}

thread_local! {
    /// Requests are answered on the thread they were offered on, which is the thread
    /// that spins the event loop, so the registry is per thread.
    static PENDING_PERMISSIONS: RefCell<PendingPermissions> =
        RefCell::new(PendingPermissions::default());
}

#[path = "../../vendor/open-letter/ffi/capi/webview_delegate.rs"]
mod open_letter;
#[allow(unused_imports)]
pub use open_letter::*;
