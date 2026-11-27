// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `components/script/dom/document/document_embedder_controls.rs`.

#[allow(unused_imports)]
use super::*;

impl DocumentEmbedderControls {
    /// Every id for one control must compare equal, since a hide or a response is matched
    /// to its control by id: the window proxy's browsing context never changes for a
    /// window, where `undiscarded_window_proxy` could stop answering in between.
    pub(super) fn control_id(&self, index: Epoch) -> EmbedderControlId {
        EmbedderControlId {
            webview_id: self.window.webview_id(),
            pipeline_id: self.window.pipeline_id(),
            browsing_context_id: self.window.window_proxy().browsing_context_id(),
            index,
        }
    }

    /// Generic embedder control extension, no domain logic.
    ///
    /// Offer `bytes` to the embedder as a page-request control, settling `promise` with
    /// its answer. The bytes are not inspected.
    pub(crate) fn send_page_request(&self, bytes: Vec<u8>, promise: &RootedPromise) {
        let id = self.next_control_id();
        self.pending_page_requests
            .borrow_mut()
            .insert(id.index.into(), promise.to_traced());
        self.send_embedder_control_request(
            EmbedderControlRequest::PageRequest(bytes),
            id,
            DeviceIntRect::zero(),
        );
    }

    pub(super) fn handle_page_request_response(
        &self,
        cx: &mut JSContext,
        id: EmbedderControlId,
        response: Option<Vec<u8>>,
    ) {
        let Some(promise) = self
            .pending_page_requests
            .borrow_mut()
            .remove(&id.index.into())
        else {
            return;
        };
        let promise = promise.root(cx);

        // Never settle a promise on an inactive `Document`.
        if !self.window.Document().is_active() {
            return;
        }

        let mut realm = enter_auto_realm(cx, &*promise);
        let cx = &mut realm.current_realm();
        let Some(bytes) = response else {
            promise.reject_error(cx, Error::NotAllowed(None));
            return;
        };
        rooted!(&in(cx) let mut js_object = ptr::null_mut::<JSObject>());
        match create_buffer_source::<Uint8>(cx, &bytes, js_object.handle_mut()) {
            Ok(array) => promise.resolve_native(cx, &array),
            Err(_) => promise.reject_error(cx, Error::JSFailed),
        }
    }
}
