// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `ffi/capi/webview_delegate.rs`.

#[allow(unused_imports)]
use super::*;

// Adding a field here changes this number. Update it deliberately, state the
// new size in the specification as part of the ABI contract, and append new
// fields after the existing ones rather than reordering, so that a caller
// built against an older header fails by reading a struct that is too short
// rather than by misreading an existing field.
const _: () = assert!(
    size_of::<ServoWebViewDelegate>() == 152,
    "ServoWebViewDelegate must stay 152 bytes wide"
);

impl ServoWebViewDelegate {
    /// A delegate with every callback `NULL` and no `user_data`, which keeps Servo's
    /// default for everything: each callback's `NULL` path is that default.
    pub(crate) fn empty() -> Self {
        // SAFETY: Every field is a raw pointer or an `Option` of a function pointer, for
        // which all-zero bytes are a valid `NULL` / `None`.
        unsafe { std::mem::zeroed() }
    }
}

/// A request for a top-level document, in [`ServoResourceRequest::kind`].
pub const SERVO_RESOURCE_REQUEST_TOP_LEVEL_NAVIGATION: u8 = 1;

/// A request for a document in a nested frame: an `<iframe>` or `<frame>` loading its
/// `src`, or a navigation inside one.
pub const SERVO_RESOURCE_REQUEST_FRAME_NAVIGATION: u8 = 2;

/// Any other request a document or a dedicated or shared worker makes: images, scripts,
/// styles, fonts, `fetch()`, `XMLHttpRequest`, WebSocket handshakes, and the like.
pub const SERVO_RESOURCE_REQUEST_SUBRESOURCE: u8 = 3;

/// One request header, in [`ServoResourceRequest::headers`].
///
/// Both strings are borrowed for the duration of the callback only and are not
/// NUL-terminated. `name` is lower-case ASCII. `value` is the header's bytes, which
/// HTTP does not require to be UTF-8.
#[repr(C)]
pub struct ServoHttpHeader {
    pub name_ptr: *const u8,
    pub name_len: usize,
    pub value_ptr: *const u8,
    pub value_len: usize,
}

const _: () = assert!(
    size_of::<ServoHttpHeader>() == 32,
    "ServoHttpHeader must stay 32 bytes wide"
);

/// A resource request offered to a [`ServoResourceRequestCallback`], passed by pointer.
///
/// Every pointer in it, and the struct itself, is borrowed for the duration of the
/// callback only. The strings are not NUL-terminated; copy what you need to keep.
#[repr(C)]
pub struct ServoResourceRequest {
    /// The URL being requested, serialized. For a redirect, the URL redirected to.
    pub url_ptr: *const u8,
    pub url_len: usize,
    /// The HTTP method, e.g. `GET` or `POST`.
    pub method_ptr: *const u8,
    pub method_len: usize,
    /// The request's headers as they stand when the request is offered, `headers_len`
    /// of them; `NULL` when there are none. These are the headers the page or the
    /// engine's caller set (for `fetch()`, the script's own). Headers the network layer
    /// adds later, on the way out (`User-Agent`, `Accept-Encoding`, `Cookie`, `Origin`,
    /// `Referer`, `Host` and the like), are not in it yet.
    pub headers: *const ServoHttpHeader,
    pub headers_len: usize,
    /// The Fetch standard's request destination: `document`, `iframe`, `frame`,
    /// `image`, `script`, `style`, `font`, and so on, or empty for `fetch()` and
    /// `XMLHttpRequest`. Finer-grained than `kind`; `kind` is what to gate on.
    pub destination_ptr: *const u8,
    pub destination_len: usize,
    /// One of the `SERVO_RESOURCE_REQUEST_*` constants.
    pub kind: u8,
    /// Whether this request follows a redirect. Each hop of a redirect chain is
    /// offered on its own, with the URL it is redirected to.
    pub is_redirect: bool,
}

const _: () = assert!(
    size_of::<ServoResourceRequest>() == 72,
    "ServoResourceRequest must stay 72 bytes wide"
);

/// Called for every resource request a `WebView` makes, before the request leaves the
/// engine, so the embedder can allow or refuse it. Return `true` to allow the request,
/// `false` to refuse it. A refused request fails as a network error, exactly as if the
/// connection had failed: a refused navigation does not load, a refused image fires
/// `error`, a refused `fetch()` rejects with a `TypeError`.
///
/// This fires for the top-level document, for every nested frame's document, and for
/// every subresource, whatever the URL's scheme (`http`, `https`, `data`, `blob`,
/// `file` and registered custom schemes alike), and again for each redirect hop. It
/// runs on the thread that calls `servo_spin_event_loop`. The request waits for the
/// answer, so the callback should answer promptly; it cannot defer the answer.
///
/// Only requests the engine attributes to this `WebView` are offered: its documents',
/// and those of its dedicated and shared workers. A request made on behalf of no
/// `WebView`, which includes every request a service worker makes, is not offered and
/// proceeds as today.
///
/// Navigations are also offered to `request_navigation` on the delegate, before this.
/// That one is the place to refuse a navigation by its URL; this one sees every
/// request, the navigation's own fetch included.
///
/// `webview` is a temporary handle with the same rules as the delegate's callbacks.
/// `request` and everything it points to are borrowed for the call only.
///
/// A callback that unwinds is treated as a refusal, for the same reason as
/// `request_navigation`'s: a vetting hook must not turn into a bypass because the
/// embedder panicked.
///
/// The alias is itself the nullable pointer, rather than an `Option` of a plain function
/// type at each use, so that the generated header declares it as one C function-pointer
/// typedef: cbindgen does not see through an `Option` of an alias.
pub type ServoResourceRequestCallback = Option<
    unsafe extern "C" fn(
        webview: *mut WebView,
        request: *const ServoResourceRequest,
        user_data: *mut c_void,
    ) -> bool,
>;

/// A set [`ServoResourceRequestCallback`] and the `user_data` it is called with.
#[derive(Clone, Copy)]
pub(crate) struct ResourceRequestHook {
    pub(crate) callback: unsafe extern "C" fn(
        webview: *mut WebView,
        request: *const ServoResourceRequest,
        user_data: *mut c_void,
    ) -> bool,
    pub(crate) user_data: *mut c_void,
}

/// Records that `webview_id` was built with a `ServoWebViewDelegate`, so that a
/// resource request callback can be set on it, and sets `hook` as its callback.
pub(crate) fn register_capi_webview(webview_id: WebViewId, hook: Option<ResourceRequestHook>) {
    RESOURCE_REQUEST_HOOKS.with_borrow_mut(|hooks| {
        hooks.insert(webview_id, hook);
    });
}

/// Sets or clears `webview_id`'s resource request callback. Returns `false`, changing
/// nothing, if the `WebView` was not built with a `ServoWebViewDelegate`.
pub(crate) fn set_resource_request_hook(
    webview_id: WebViewId,
    hook: Option<ResourceRequestHook>,
) -> bool {
    RESOURCE_REQUEST_HOOKS.with_borrow_mut(|hooks| match hooks.get_mut(&webview_id) {
        Some(slot) => {
            *slot = hook;
            true
        },
        None => false,
    })
}

/// Forgets everything recorded for `webview_id`.
pub(crate) fn forget_webview(webview_id: WebViewId) {
    RESOURCE_REQUEST_HOOKS.with_borrow_mut(|hooks| {
        hooks.remove(&webview_id);
    });
}

pub(super) fn resource_request_hook(webview_id: WebViewId) -> Option<ResourceRequestHook> {
    RESOURCE_REQUEST_HOOKS.with_borrow(|hooks| hooks.get(&webview_id).copied().flatten())
}

pub(super) fn record_top_level_content_process(webview_id: WebViewId, event: &FrameEvent) {
    TOP_LEVEL_CONTENT_PROCESS.with_borrow_mut(|map| match event.kind {
        FrameEventKind::Navigated => match event.content_process_id {
            Some(pid) if pid != 0 => {
                map.insert(webview_id, pid);
            },
            _ => {
                map.remove(&webview_id);
            },
        },
        FrameEventKind::Removed => {
            map.remove(&webview_id);
        },
        FrameEventKind::Created => {},
    });
}

/// The content process of `webview_id`'s active top-level document, 0 if none.
pub(crate) fn top_level_content_process(webview_id: WebViewId) -> u32 {
    TOP_LEVEL_CONTENT_PROCESS.with_borrow(|map| map.get(&webview_id).copied().unwrap_or(0))
}

/// A frame was created, in [`ServoWebViewDelegate::notify_frame_changed`].
pub const SERVO_FRAME_EVENT_CREATED: u8 = 1;

/// A new document became active in a frame.
pub const SERVO_FRAME_EVENT_NAVIGATED: u8 = 2;

/// A frame was removed.
pub const SERVO_FRAME_EVENT_REMOVED: u8 = 3;

impl ServoWebViewDelegate {

    /// Body of [`WebViewDelegate::notify_crashed`] for this delegate.
    pub(super) fn open_letter_notify_crashed(&self, mut webview: WebView, reason: String, backtrace: Option<String>) {
        // Before the callback check: whatever served this webview's document is gone,
        // so `servo_webview_content_process_id` must stop naming it, whether or not
        // the embedder listens. The replacement document's `Navigated` event records
        // the next one.
        TOP_LEVEL_CONTENT_PROCESS.with_borrow_mut(|map| {
            map.remove(&webview.id());
        });

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

    /// Body of [`WebViewDelegate::notify_closed`] for this delegate.
    pub(super) fn open_letter_notify_closed(&self, mut webview: WebView) {
        // No further requests come for a closed `WebView`, whether or not the embedder
        // listens for the close.
        forget_webview(webview.id());

        let Some(callback) = self.notify_closed else {
            return;
        };

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle.
        unsafe { callback(&mut webview as *mut WebView, self.user_data) };
    }

    /// Body of [`WebViewDelegate::show_console_message`] for this delegate.
    pub(super) fn open_letter_show_console_message(
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

    /// Body of [`WebViewDelegate::notify_url_changed`] for this delegate.
    pub(super) fn open_letter_notify_url_changed(&self, mut webview: WebView, url: url::Url) {
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

    /// Body of [`WebViewDelegate::request_navigation`] for this delegate.
    pub(super) fn open_letter_request_navigation(&self, mut webview: WebView, navigation_request: NavigationRequest) {
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

    /// Body of [`WebViewDelegate::load_web_resource`] for this delegate.
    pub(super) fn open_letter_load_web_resource(&self, mut webview: WebView, load: WebResourceLoad) {
        let Some(hook) = resource_request_hook(webview.id()) else {
            // Keep Servo's default path. Dropping the load without answering lets the
            // request proceed untouched, which is what an unhandled load does upstream.
            return;
        };

        let request = load.request();
        let url = request.url.as_str();
        let method = request.method.as_str();
        let destination = request.destination.as_str();
        let headers: Vec<ServoHttpHeader> = request
            .headers
            .iter()
            .map(|(name, value)| ServoHttpHeader {
                name_ptr: name.as_str().as_ptr(),
                name_len: name.as_str().len(),
                value_ptr: value.as_bytes().as_ptr(),
                value_len: value.as_bytes().len(),
            })
            .collect();
        let kind = if request.is_for_main_frame {
            SERVO_RESOURCE_REQUEST_TOP_LEVEL_NAVIGATION
        } else if matches!(destination, "document" | "iframe" | "frame") {
            SERVO_RESOURCE_REQUEST_FRAME_NAVIGATION
        } else {
            SERVO_RESOURCE_REQUEST_SUBRESOURCE
        };

        let c_request = ServoResourceRequest {
            url_ptr: url.as_ptr(),
            url_len: url.len(),
            method_ptr: method.as_ptr(),
            method_len: method.len(),
            headers: if headers.is_empty() {
                std::ptr::null()
            } else {
                headers.as_ptr()
            },
            headers_len: headers.len(),
            destination_ptr: destination.as_ptr(),
            destination_len: destination.len(),
            kind,
            is_redirect: request.is_redirect,
        };

        // SAFETY: The embedder is assumed to uphold the safety requirements documented
        // on `ServoResourceRequestCallback` and its setters.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle, and
        // `c_request` borrows `load` and `headers`, both of which outlive the call.
        //
        // The callback is contracted not to unwind, but it is contained here anyway
        // so that a panicking embedder cannot unwind into Servo's event loop.
        let allowed = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            (hook.callback)(
                &mut webview as *mut WebView,
                &c_request as *const ServoResourceRequest,
                hook.user_data,
            )
        }));

        match allowed {
            // Dropping the load lets the request proceed untouched.
            Ok(true) => {},
            // Refused, or the embedder was asked and did not answer: fail the load as
            // a network error. A panicking callback must not become a silent bypass,
            // as for `request_navigation`.
            Ok(false) | Err(..) => {
                let url = load.request().url.clone().into_url();
                load.intercept(WebResourceResponse::new(url)).cancel();
            },
        }
    }

    /// Body of [`WebViewDelegate::notify_fullscreen_state_changed`] for this delegate.
    pub(super) fn open_letter_notify_fullscreen_state_changed(&self, mut webview: WebView, is_fullscreen: bool) {
        let Some(callback) = self.notify_fullscreen_state_changed else {
            return;
        };

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle.
        unsafe { callback(&mut webview as *mut WebView, is_fullscreen, self.user_data) };
    }

    /// Body of [`WebViewDelegate::notify_cursor_changed`] for this delegate.
    pub(super) fn open_letter_notify_cursor_changed(&self, mut webview: WebView, cursor: Cursor) {
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

    /// Body of [`WebViewDelegate::notify_page_title_changed`] for this delegate.
    pub(super) fn open_letter_notify_page_title_changed(&self, mut webview: WebView, title: Option<String>) {
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

    /// Body of [`WebViewDelegate::request_move_to`] for this delegate.
    pub(super) fn open_letter_request_move_to(&self, mut webview: WebView, point: DeviceIntPoint) {
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

    /// Body of [`WebViewDelegate::request_resize_to`] for this delegate.
    pub(super) fn open_letter_request_resize_to(&self, mut webview: WebView, requested_outer_size: DeviceIntSize) {
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

    /// Body of [`WebViewDelegate::notify_frame_changed`] for this delegate.
    pub(super) fn open_letter_notify_frame_changed(&self, mut webview: WebView, event: FrameEvent) {
        // Before the callback check: `servo_webview_content_process_id` must answer
        // whether or not the embedder subscribed to frame events.
        if event.parent_browsing_context_id.is_none() {
            record_top_level_content_process(webview.id(), &event);
        }

        let Some(callback) = self.notify_frame_changed else {
            return;
        };

        let kind = match event.kind {
            FrameEventKind::Created => SERVO_FRAME_EVENT_CREATED,
            FrameEventKind::Navigated => SERVO_FRAME_EVENT_NAVIGATED,
            FrameEventKind::Removed => SERVO_FRAME_EVENT_REMOVED,
        };
        let (url_ptr, url_len) = match &event.url {
            Some(url) => (url.as_str().as_ptr(), url.as_str().len()),
            None => (std::ptr::null(), 0),
        };

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle, and `event`
        // outlives the call, so the borrowed URL remains valid for its duration.
        //
        // The callback is contracted not to unwind, but it is contained here anyway
        // so that a panicking embedder cannot unwind into Servo's event loop.
        let _ = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            callback(
                &mut webview as *mut WebView,
                browsing_context_id_to_c(event.browsing_context_id),
                optional_browsing_context_id_to_c(event.parent_browsing_context_id),
                kind,
                url_ptr,
                url_len,
                self.user_data,
            )
        }));
    }

    /// Body of [`WebViewDelegate::request_unload`] for this delegate.
    pub(super) fn open_letter_request_unload(&self, mut webview: WebView, unload_request: AllowOrDenyRequest) {
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

    /// Body of [`WebViewDelegate::request_permission`] for this delegate.
    pub(super) fn open_letter_request_permission(&self, mut webview: WebView, permission_request: PermissionRequest) {
        let Some(callback) = self.request_permission else {
            // Keep Servo's default path: dropping the request denies it.
            return;
        };

        let feature = permission_feature_to_c(permission_request.feature());
        let requester_browsing_context_id =
            optional_browsing_context_id_to_c(permission_request.requester_browsing_context_id());
        let requester_origin = permission_request
            .requester_origin()
            .ascii_serialization()
            .to_string();
        let top_level_origin = webview
            .url()
            .map(|url| url.origin().ascii_serialization())
            .unwrap_or_default();

        // Register the request before calling out, without holding the borrow across
        // the call, so that the embedder can answer from inside the callback.
        let id = PENDING_PERMISSIONS.with(|pending| {
            let mut pending = pending.borrow_mut();
            pending.next_id += 1;
            let id = pending.next_id;
            pending.requests.insert(id, permission_request);
            id
        });

        let request = ServoPermissionRequest {
            id,
            feature,
            _reserved: 0,
            requester_origin_ptr: requester_origin.as_ptr(),
            requester_origin_len: requester_origin.len(),
            top_level_origin_ptr: top_level_origin.as_ptr(),
            top_level_origin_len: top_level_origin.len(),
            requester_browsing_context_id,
            top_level_browsing_context_id: browsing_context_id_to_c(webview.id().into()),
        };

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle, and
        // `request` points to a view that, with the strings it borrows, outlives the call.
        //
        // The callback is contracted not to unwind, but it is contained here anyway
        // so that a panicking embedder cannot unwind into Servo's event loop.
        let result = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            callback(
                &mut webview as *mut WebView,
                &request as *const ServoPermissionRequest,
                self.user_data,
            )
        }));

        if result.is_err() {
            // Dropping the request denies it, as with a `NULL` callback.
            PENDING_PERMISSIONS.with(|pending| pending.borrow_mut().requests.remove(&id));
        }
    }

    /// Body of [`WebViewDelegate::request_create_new`] for this delegate.
    pub(super) fn open_letter_request_create_new(
        &self,
        mut webview: WebView,
        create_new_webview_request: CreateNewWebViewRequest,
    ) {
        let Some(callback) = self.request_create_new else {
            // Keep Servo's default path: dropping the request answers it with `None`,
            // which script sees as a blocked open.
            return;
        };

        let request = ServoNewWebViewRequest { _reserved: 0 };

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
        // `request` outlives the call.
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

    /// Body of [`WebViewDelegate::embedder_control_flags`] for this delegate.
    pub(super) fn open_letter_embedder_control_flags(&self) -> u64 {
        self.controller().map_or(0, |controller| controller.flags)
    }

    /// Body of [`WebViewDelegate::handle_embedder_control`] for this delegate.
    pub(super) fn open_letter_handle_embedder_control(
        &self,
        mut webview: WebView,
        embedder_control: EmbedderControl,
        requester_origin: Option<&ImmutableOrigin>,
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

        // The requesting document's own origin, a nested frame's when a frame asked, not
        // the top-level document's: the embedder gates on it. Servo does not interpret it.
        // `origin` owns the string for the duration of the callback.
        let origin = requester_origin
            .map(ImmutableOrigin::ascii_serialization)
            .unwrap_or_default();
        let requester_browsing_context_id =
            browsing_context_id_to_c(embedder_control.id().browsing_context_id());
        let top_level_browsing_context_id = browsing_context_id_to_c(webview.id().into());

        // Only a page request carries bytes, and they are the page's, borrowed from the
        // control for the duration of the callback. Servo serializes nothing of its own.
        let (payload_ptr, payload_len) = match &embedder_control {
            EmbedderControl::PageRequest(page_request) => {
                (page_request.bytes().as_ptr(), page_request.bytes().len())
            },
            _ => (std::ptr::null(), 0),
        };

        let control = ServoEmbedderControl {
            tag: tag as u32,
            origin_ptr: origin.as_ptr(),
            origin_len: origin.len(),
            payload_ptr,
            payload_len,
            requester_browsing_context_id,
            top_level_browsing_context_id,
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
/// `SERVO_EMBEDDER_CONTROL_TAG_PAGE_REQUEST` carries bytes from page script, passed to
/// `EmbedderRequest.send()`, and is answered with bytes; see
/// [`servo_webview_send_response`].
///
/// `SERVO_EMBEDDER_CONTROL_TAG_HOST_REQUEST` is reserved for controls that the embedder
/// itself originates. Servo never emits it; it exists so that an embedder can reuse this
/// tag space and wire format for its own controls.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum ServoEmbedderControlTag {
    ContextMenu = 1,
    Select = 2,
    FilePicker = 3,
    PageRequest = 4,
    HostRequest = 5,
}

const _: () = assert!(
    ServoEmbedderControlTag::ContextMenu as u32 == EmbedderControlTag::ContextMenu as u32
        && ServoEmbedderControlTag::Select as u32 == EmbedderControlTag::Select as u32
        && ServoEmbedderControlTag::FilePicker as u32 == EmbedderControlTag::FilePicker as u32
        && ServoEmbedderControlTag::PageRequest as u32 == EmbedderControlTag::PageRequest as u32
        && ServoEmbedderControlTag::HostRequest as u32 == EmbedderControlTag::HostRequest as u32,
    "the C and Rust embedder control tags must agree"
);

/// Generic embedder control extension, no domain logic.
///
/// A control offered to the embedder. This is a borrowed view that is only valid for
/// the duration of the [`ServoEmbedderController::on_control`] call that receives it;
/// the embedder must copy anything it needs to retain.
///
/// `payload_ptr`/`payload_len` are raw bytes. Servo never parses them and never
/// produces them. A page request carries the bytes page script passed, exactly as passed;
/// every other control Servo originates carries a `NULL`, zero-length payload.
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

    /// The browsing context of the frame that triggered this control, or `0` if
    /// unknown. Opaque, the same values [`ServoPermissionRequest`] and
    /// [`ServoWebViewDelegate::notify_frame_changed`] carry.
    pub requester_browsing_context_id: u64,
    /// The browsing context of the `WebView`'s top-level frame, or `0` if unknown.
    pub top_level_browsing_context_id: u64,
}

// Passed by pointer, so growing it needs no ABI version bump; still pinned here and in
// `tests::abi_struct_sizes`.
const _: () = assert!(
    size_of::<ServoEmbedderControl>() == 56,
    "ServoEmbedderControl must stay 56 bytes wide"
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
/// **Opaque.** It is a token to hand back to [`servo_webview_accept_new`] and nothing
/// else: there is no field to read, and the pointer is valid only for the duration of
/// the callback that received it.
///
/// It carries no URL deliberately. A new browsing context is created on `about:blank`
/// and navigated afterwards, so at the moment this request exists the URL the opener
/// asked for does not exist anywhere in the engine — see
/// [`ServoWebViewDelegate::request_create_new`] for where it does appear. Being opaque
/// also means a field could be added here later without an ABI break, since the
/// embedder only ever holds a pointer to it.
// cbindgen:opaque
pub struct ServoNewWebViewRequest {
    /// Reserved. This type exists to be addressed, not read; the field keeps it
    /// non-zero-sized, so two live requests cannot share an address and be mistaken
    /// for one another.
    pub(super) _reserved: u64,
}

const _: () = assert!(
    size_of::<ServoNewWebViewRequest>() == 8,
    "ServoNewWebViewRequest must stay 8 bytes wide"
);

/// A new-browsing-context request currently being offered to
/// [`ServoWebViewDelegate::request_create_new`] and not yet claimed, together with the
/// address of the borrowed [`ServoNewWebViewRequest`] view the embedder was handed. The
/// view address is what [`servo_webview_accept_new`] matches on, the same way
/// [`servo_webview_send_response`] matches a control.
pub(super) struct InFlightNewWebView {
    pub(super) view: *const ServoNewWebViewRequest,
    /// `None` once the embedder has claimed it.
    pub(super) request: Option<CreateNewWebViewRequest>,
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
    let has_capi_delegate = !delegate.is_null();
    if let Some(delegate) = unsafe { delegate.as_ref() } {
        builder = builder.delegate(std::rc::Rc::new(*delegate));
    }

    // Matching `servo_webview_builder_build`: every `WebView` gets a user-content
    // manager, so that `servo_webview_add_script` works on it afterwards.
    builder = builder.user_content_manager(std::rc::Rc::new(UserContentManager::new(&servo)));

    let webview = builder.build();
    // A `WebView` accepted with a delegate can take a resource request callback through
    // `servo_webview_set_resource_request_callback`. None is set yet. A request made
    // before one is set proceeds as today. None can be made before this returns: the
    // engine answers requests only while the embedder spins its event loop.
    if has_capi_delegate {
        register_capi_webview(webview.id(), None);
    }
    Box::into_raw(Box::new(webview))
}

/// Generic embedder control extension, no domain logic.
///
/// A control that has been offered to [`ServoEmbedderController::on_control`] and is
/// still unanswered, together with the address of the borrowed [`ServoEmbedderControl`]
/// view that the embedder was handed for it. The view address is what
/// [`servo_webview_send_response`] matches on, which keeps the response path off the
/// 40-byte `ServoEmbedderControl` ABI.
pub(super) struct InFlightControl {
    pub(super) view: *const ServoEmbedderControl,
    /// `None` once the embedder has claimed the control with a response.
    pub(super) control: Option<EmbedderControl>,
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
pub(super) fn context_menu_action(code: u32) -> Option<ContextMenuAction> {
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
pub(super) fn decode_u32s(payload: &[u8]) -> Option<Vec<u32>> {
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
pub(super) fn flattened_option_count(options: &[SelectElementOptionOrOptgroup]) -> usize {
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
pub(super) fn send_control_response(
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
        EmbedderControl::PageRequest(page_request) => {
            page_request.respond(payload.to_vec());
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
pub(super) unsafe fn claim_and_respond(
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
/// - `SERVO_EMBEDDER_CONTROL_TAG_PAGE_REQUEST`: opaque bytes of any length, including
///   zero, which resolve the page's promise verbatim. There is no response that rejects
///   it: to refuse, return `false` from `on_control` without responding, and the
///   promise rejects. So does a request whose tag bit is not set in `flags`.
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

/// Bytes sent by page script through `EmbedderRequest.send()`.
pub const SERVO_EMBEDDER_CONTROL_PAGE_REQUEST: u64 = 1 << 3;

/// Reserved for embedder-originated controls.
pub const SERVO_EMBEDDER_CONTROL_HOST_REQUEST: u64 = 1 << 4;

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

pub const SERVO_CONSOLE_LOG_LEVEL_TABLE: i32 = 7;

/// Converts a [`servo_api::ConsoleLogLevel`] to one of the
/// `SERVO_CONSOLE_LOG_LEVEL_*` constants.
///
/// An exhaustive match, not a cast: `ConsoleLogLevel` has no stable repr, so
/// this is the only sound way to produce a value that means the same thing on
/// both sides of the ABI. Adding a variant upstream is a compile error here
/// rather than a silent renumbering.
pub(super) fn console_log_level_to_c(level: &servo_api::ConsoleLogLevel) -> i32 {
    use servo_api::ConsoleLogLevel::*;
    match level {
        Log => SERVO_CONSOLE_LOG_LEVEL_LOG,
        Debug => SERVO_CONSOLE_LOG_LEVEL_DEBUG,
        Info => SERVO_CONSOLE_LOG_LEVEL_INFO,
        Warn => SERVO_CONSOLE_LOG_LEVEL_WARN,
        Error => SERVO_CONSOLE_LOG_LEVEL_ERROR,
        Trace => SERVO_CONSOLE_LOG_LEVEL_TRACE,
        Dir => SERVO_CONSOLE_LOG_LEVEL_DIR,
        Table => SERVO_CONSOLE_LOG_LEVEL_TABLE,
    }
}

/// A permission request, as offered to [`ServoWebViewDelegate::request_permission`].
///
/// A view borrowed for the duration of that call: the origin strings are UTF-8, not
/// NUL-terminated, and must be copied if they are needed afterwards. `id` remains valid
/// after the call until the request is answered.
///
/// # Origins
///
/// Both are supplied by the engine and cannot be asserted by the page. They are ASCII
/// serializations, `"null"` for an opaque origin.
///
/// - `requester_origin` is the origin of the global that asked. When a nested frame
///   asks, this is the frame's origin, not the page's.
/// - `top_level_origin` is the origin of the `WebView`'s current top-level document,
///   empty if it has none. The two are equal unless the request came from a frame.
///
/// # Frames
///
/// The two `*_browsing_context_id` fields identify frames by their browsing context,
/// the frame slot, which keeps its identity across navigations inside it. See
/// [`ServoWebViewDelegate::notify_frame_changed`], which reports the same values.
/// They are opaque: compare them and use them as keys, nothing else. `0` means
/// unknown, never a real frame.
#[repr(C)]
pub struct ServoPermissionRequest {
    /// Answers this request through [`servo_webview_respond_permission`]. Never zero.
    pub id: u64,
    /// One of the `SERVO_PERMISSION_*` constants.
    pub feature: u32,
    /// Always zero.
    pub _reserved: u32,
    pub requester_origin_ptr: *const u8,
    pub requester_origin_len: usize,
    pub top_level_origin_ptr: *const u8,
    pub top_level_origin_len: usize,
    /// The browsing context of the frame that asked, or `0` if unknown.
    pub requester_browsing_context_id: u64,
    /// The browsing context of the `WebView`'s top-level frame, or `0` if unknown.
    pub top_level_browsing_context_id: u64,
}

/// The opaque `u64` a browsing context crosses this ABI as. Never `0`, which the ABI
/// reserves for "unknown": the low half is a `NonZeroU32` index.
pub(crate) fn browsing_context_id_to_c(id: BrowsingContextId) -> u64 {
    (u64::from(id.namespace_id.0) << 32) | u64::from(id.index.0.get())
}

pub(super) fn optional_browsing_context_id_to_c(id: Option<BrowsingContextId>) -> u64 {
    id.map_or(0, browsing_context_id_to_c)
}

// Passed by pointer, so growing it needs no ABI version bump; still pinned here and in
// `tests::abi_struct_sizes`.
const _: () = assert!(
    size_of::<ServoPermissionRequest>() == 64,
    "ServoPermissionRequest must stay 64 bytes wide"
);

/// The permission being requested, in [`ServoPermissionRequest::feature`].
///
/// `servo_api::PermissionFeature` carries no `#[repr]` and has a variant with a
/// payload, so these constants and [`permission_feature_to_c`] are this crate's own
/// stable numbering, kept in sync with the Rust enum by an exhaustive match.
pub const SERVO_PERMISSION_GEOLOCATION: u32 = 0;

pub const SERVO_PERMISSION_NOTIFICATIONS: u32 = 1;

pub const SERVO_PERMISSION_PUSH: u32 = 2;

pub const SERVO_PERMISSION_MIDI: u32 = 3;

pub const SERVO_PERMISSION_CAMERA: u32 = 4;

pub const SERVO_PERMISSION_MICROPHONE: u32 = 5;

pub const SERVO_PERMISSION_SPEAKER: u32 = 6;

pub const SERVO_PERMISSION_DEVICE_INFO: u32 = 7;

pub const SERVO_PERMISSION_BACKGROUND_SYNC: u32 = 8;

pub const SERVO_PERMISSION_BLUETOOTH: u32 = 9;

pub const SERVO_PERMISSION_PERSISTENT_STORAGE: u32 = 10;

pub const SERVO_PERMISSION_SCREEN_WAKE_LOCK: u32 = 11;

pub const SERVO_PERMISSION_GAMEPAD: u32 = 12;

/// Converts a [`servo_api::PermissionFeature`] to one of the `SERVO_PERMISSION_*`
/// constants. Adding a variant upstream is a compile error here rather than a silent
/// renumbering.
pub(super) fn permission_feature_to_c(feature: servo_api::PermissionFeature) -> u32 {
    use servo_api::PermissionFeature::*;
    use servo_api::WakeLockType;
    match feature {
        Geolocation => SERVO_PERMISSION_GEOLOCATION,
        Notifications => SERVO_PERMISSION_NOTIFICATIONS,
        Push => SERVO_PERMISSION_PUSH,
        Midi => SERVO_PERMISSION_MIDI,
        Camera => SERVO_PERMISSION_CAMERA,
        Microphone => SERVO_PERMISSION_MICROPHONE,
        Speaker => SERVO_PERMISSION_SPEAKER,
        DeviceInfo => SERVO_PERMISSION_DEVICE_INFO,
        BackgroundSync => SERVO_PERMISSION_BACKGROUND_SYNC,
        Bluetooth => SERVO_PERMISSION_BLUETOOTH,
        PersistentStorage => SERVO_PERMISSION_PERSISTENT_STORAGE,
        ScreenWakeLock(WakeLockType::Screen) => SERVO_PERMISSION_SCREEN_WAKE_LOCK,
        Gamepad => SERVO_PERMISSION_GAMEPAD,
    }
}

/// Permission requests offered to the embedder and not yet answered, by id.
#[derive(Default)]
pub(super) struct PendingPermissions {
    pub(super) next_id: u64,
    pub(super) requests: std::collections::HashMap<u64, PermissionRequest>,
}

/// Answers a permission request offered to [`ServoWebViewDelegate::request_permission`].
///
/// `allow` grants the permission; `false` denies it. This may be called during that
/// callback or at any later point, from the thread that calls `servo_spin_event_loop`.
///
/// # Return value
///
/// `true` if the answer was delivered. `false` if `id` does not name an unanswered
/// request on this thread: it was already answered, its callback unwound, it came from
/// another thread, or it was never issued. Nothing is sent in that case, so answering
/// twice is harmless.
#[unsafe(no_mangle)]
pub extern "C" fn servo_webview_respond_permission(id: u64, allow: bool) -> bool {
    // A panic must not unwind into the embedder's C frame.
    panic::catch_unwind(|| {
        let Some(request) =
            PENDING_PERMISSIONS.with(|pending| pending.borrow_mut().requests.remove(&id))
        else {
            return false;
        };
        if allow {
            request.allow();
        } else {
            request.deny();
        }
        true
    })
    .unwrap_or(false)
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
                requester_browsing_context_id: 0,
                top_level_browsing_context_id: 0,
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
pub const SERVO_CURSOR_NONE: u32 = 0;

pub const SERVO_CURSOR_DEFAULT: u32 = 1;

pub const SERVO_CURSOR_POINTER: u32 = 2;

pub const SERVO_CURSOR_CONTEXT_MENU: u32 = 3;

pub const SERVO_CURSOR_HELP: u32 = 4;

pub const SERVO_CURSOR_PROGRESS: u32 = 5;

pub const SERVO_CURSOR_WAIT: u32 = 6;

pub const SERVO_CURSOR_CELL: u32 = 7;

pub const SERVO_CURSOR_CROSSHAIR: u32 = 8;

pub const SERVO_CURSOR_TEXT: u32 = 9;

pub const SERVO_CURSOR_VERTICAL_TEXT: u32 = 10;

pub const SERVO_CURSOR_ALIAS: u32 = 11;

pub const SERVO_CURSOR_COPY: u32 = 12;

pub const SERVO_CURSOR_MOVE: u32 = 13;

pub const SERVO_CURSOR_NO_DROP: u32 = 14;

pub const SERVO_CURSOR_NOT_ALLOWED: u32 = 15;

pub const SERVO_CURSOR_GRAB: u32 = 16;

pub const SERVO_CURSOR_GRABBING: u32 = 17;

pub const SERVO_CURSOR_E_RESIZE: u32 = 18;

pub const SERVO_CURSOR_N_RESIZE: u32 = 19;

pub const SERVO_CURSOR_NE_RESIZE: u32 = 20;

pub const SERVO_CURSOR_NW_RESIZE: u32 = 21;

pub const SERVO_CURSOR_S_RESIZE: u32 = 22;

pub const SERVO_CURSOR_SE_RESIZE: u32 = 23;

pub const SERVO_CURSOR_SW_RESIZE: u32 = 24;

pub const SERVO_CURSOR_W_RESIZE: u32 = 25;

pub const SERVO_CURSOR_EW_RESIZE: u32 = 26;

pub const SERVO_CURSOR_NS_RESIZE: u32 = 27;

pub const SERVO_CURSOR_NESW_RESIZE: u32 = 28;

pub const SERVO_CURSOR_NWSE_RESIZE: u32 = 29;

pub const SERVO_CURSOR_COL_RESIZE: u32 = 30;

pub const SERVO_CURSOR_ROW_RESIZE: u32 = 31;

pub const SERVO_CURSOR_ALL_SCROLL: u32 = 32;

pub const SERVO_CURSOR_ZOOM_IN: u32 = 33;

pub const SERVO_CURSOR_ZOOM_OUT: u32 = 34;

// Each value is written as a literal rather than as `Cursor::X as u32`, so that it
// reaches the generated header as a usable `#define`. A cast of an enum belonging to
// another crate does not: cbindgen cannot resolve it to a number and emits a reference
// to a type this header does not declare, which is how these constants came to be
// missing from the header entirely while still being documented as the ABI.
//
// The assertion below is what the cast used to provide. It covers every value, so a
// variant inserted upstream rather than appended fails the build here instead of
// silently changing what a value means on the far side of the ABI.
const _: () = assert!(
    SERVO_CURSOR_NONE == Cursor::None as u32
        && SERVO_CURSOR_DEFAULT == Cursor::Default as u32
        && SERVO_CURSOR_POINTER == Cursor::Pointer as u32
        && SERVO_CURSOR_CONTEXT_MENU == Cursor::ContextMenu as u32
        && SERVO_CURSOR_HELP == Cursor::Help as u32
        && SERVO_CURSOR_PROGRESS == Cursor::Progress as u32
        && SERVO_CURSOR_WAIT == Cursor::Wait as u32
        && SERVO_CURSOR_CELL == Cursor::Cell as u32
        && SERVO_CURSOR_CROSSHAIR == Cursor::Crosshair as u32
        && SERVO_CURSOR_TEXT == Cursor::Text as u32
        && SERVO_CURSOR_VERTICAL_TEXT == Cursor::VerticalText as u32
        && SERVO_CURSOR_ALIAS == Cursor::Alias as u32
        && SERVO_CURSOR_COPY == Cursor::Copy as u32
        && SERVO_CURSOR_MOVE == Cursor::Move as u32
        && SERVO_CURSOR_NO_DROP == Cursor::NoDrop as u32
        && SERVO_CURSOR_NOT_ALLOWED == Cursor::NotAllowed as u32
        && SERVO_CURSOR_GRAB == Cursor::Grab as u32
        && SERVO_CURSOR_GRABBING == Cursor::Grabbing as u32
        && SERVO_CURSOR_E_RESIZE == Cursor::EResize as u32
        && SERVO_CURSOR_N_RESIZE == Cursor::NResize as u32
        && SERVO_CURSOR_NE_RESIZE == Cursor::NeResize as u32
        && SERVO_CURSOR_NW_RESIZE == Cursor::NwResize as u32
        && SERVO_CURSOR_S_RESIZE == Cursor::SResize as u32
        && SERVO_CURSOR_SE_RESIZE == Cursor::SeResize as u32
        && SERVO_CURSOR_SW_RESIZE == Cursor::SwResize as u32
        && SERVO_CURSOR_W_RESIZE == Cursor::WResize as u32
        && SERVO_CURSOR_EW_RESIZE == Cursor::EwResize as u32
        && SERVO_CURSOR_NS_RESIZE == Cursor::NsResize as u32
        && SERVO_CURSOR_NESW_RESIZE == Cursor::NeswResize as u32
        && SERVO_CURSOR_NWSE_RESIZE == Cursor::NwseResize as u32
        && SERVO_CURSOR_COL_RESIZE == Cursor::ColResize as u32
        && SERVO_CURSOR_ROW_RESIZE == Cursor::RowResize as u32
        && SERVO_CURSOR_ALL_SCROLL == Cursor::AllScroll as u32
        && SERVO_CURSOR_ZOOM_IN == Cursor::ZoomIn as u32
        && SERVO_CURSOR_ZOOM_OUT == Cursor::ZoomOut as u32,
    "the SERVO_CURSOR_* values are the ABI and must agree with upstream's Cursor"
);
