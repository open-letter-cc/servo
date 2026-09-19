/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::ffi::c_void;

pub use servo_api::LoadStatus;
use servo_api::{EmbedderControl, EmbedderControlTag, WebView, WebViewDelegate};

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
#[repr(C)]
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
}

const _: () = assert!(
    size_of::<ServoWebViewDelegate>() == 32,
    "ServoWebViewDelegate must stay 32 bytes wide"
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

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoEmbedderController` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle, and
        // `control` borrows `origin`, which outlives the call.
        let handled = unsafe {
            on_control(
                &mut webview as *mut WebView,
                &control as *const ServoEmbedderControl,
                self.user_data,
            )
        };

        drop(origin);

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
    /// control sends its default response (a dismissal, or the current selection).
    pub on_control: Option<
        unsafe extern "C" fn(
            webview: *mut WebView,
            control: *const ServoEmbedderControl,
            user_data: *mut c_void,
        ) -> bool,
    >,
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
