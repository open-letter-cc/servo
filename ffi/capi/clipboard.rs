/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::panic::{self, AssertUnwindSafe};
use std::sync::{Mutex, OnceLock};

use arboard::Clipboard;
use servo_api::{ClipboardDelegate, StringRequest, WebView};

use crate::webview_delegate::ServoWebViewDelegate;

/// Gates every clipboard operation behind `ServoWebViewDelegate::request_clipboard_access`,
/// then performs it against the real OS clipboard on an allow.
///
/// Installed as the `WebView`'s `ClipboardDelegate` in place of Servo's own default
/// whenever an embedder sets a `ServoWebViewDelegate` at all (see
/// `servo_webview_builder_build`), so the gate cannot be bypassed by a `WebView` that
/// never configured one. Servo's own default clipboard delegate is `pub(crate)` to
/// `components/servo`, so this talks to the OS clipboard directly through `arboard`
/// rather than composing with it.
pub(crate) struct CapiClipboardDelegate(pub ServoWebViewDelegate);

impl CapiClipboardDelegate {
    fn allowed(&self, webview: &mut WebView) -> bool {
        let Some(callback) = self.0.request_clipboard_access else {
            // Keep Servo's default path, which allows clipboard access.
            return true;
        };

        // SAFETY: The embedder is assumed to uphold the safety requirements of the
        // `ServoWebViewDelegate` struct.
        //
        // The `webview` raw pointer is derived from a valid `webview` handle.
        //
        // The callback is contracted not to unwind, but it is contained here anyway
        // so that a panicking embedder cannot unwind into Servo's event loop.
        let allowed = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            callback(webview as *mut WebView, self.0.user_data)
        }));

        // Refused for the same reason as an unanswered navigation: see
        // `request_navigation`.
        allowed.unwrap_or(false)
    }
}

impl ClipboardDelegate for CapiClipboardDelegate {
    fn clear(&self, mut webview: WebView) {
        if self.allowed(&mut webview) {
            let _ = with_clipboard(|clipboard| clipboard.clear());
        }
    }

    fn get_text(&self, mut webview: WebView, request: StringRequest) {
        if !self.allowed(&mut webview) {
            request.failure("clipboard access denied by the embedder".to_string());
            return;
        }

        match with_clipboard(|clipboard| clipboard.get_text()) {
            Ok(text) => request.success(text),
            Err(error) => request.failure(error.to_string()),
        }
    }

    fn set_text(&self, mut webview: WebView, new_contents: String) {
        if self.allowed(&mut webview) {
            let _ = with_clipboard(|clipboard| clipboard.set_text(&new_contents));
        }
    }
}

/// Shared with every `CapiClipboardDelegate`: `arboard::Clipboard` holds the one OS
/// clipboard handle, and only one should exist per process. See the same pattern in
/// `components/servo/clipboard_delegate.rs`'s own default delegate.
static SHARED_CLIPBOARD: OnceLock<Option<Mutex<Clipboard>>> = OnceLock::new();

fn with_clipboard<T>(
    callback: impl FnOnce(&mut Clipboard) -> Result<T, arboard::Error>,
) -> Result<T, arboard::Error> {
    match SHARED_CLIPBOARD.get_or_init(|| Clipboard::new().ok().map(Mutex::new)) {
        Some(clipboard) => {
            let mut clipboard = clipboard
                .lock()
                .map_err(|_| arboard::Error::ClipboardNotSupported)?;
            callback(&mut clipboard)
        },
        None => Err(arboard::Error::ClipboardNotSupported),
    }
}
