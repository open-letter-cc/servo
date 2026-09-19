/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::cell::RefCell;
use std::ffi::c_void;
use std::panic::{self, AssertUnwindSafe};

pub use servo_api::LoadStatus;
use servo_api::{
    ContextMenuAction, EmbedderControl, EmbedderControlTag, SelectElementOptionOrOptgroup, WebView,
    WebViewDelegate,
};

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
