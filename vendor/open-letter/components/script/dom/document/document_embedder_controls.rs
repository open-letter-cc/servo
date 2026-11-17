// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/script/dom/document/document_embedder_controls.rs`.

#[allow(unused_imports)]
use super::*;

use std::ptr;
use embedder_traits::{
    ClipboardAction, ContextMenuAction, ContextMenuElementInformation,
    ContextMenuElementInformationFlags, ContextMenuItem, ContextMenuRequest, EmbedderControlId,
    EmbedderControlRequest, EmbedderControlResponse, EmbedderMsg,
};
use euclid::{Point2D, Rect, Size2D};
use js::context::JSContext;
use js::jsapi::JSObject;
use js::typedarray::Uint8;
use net_traits::CoreResourceMsg;
use net_traits::filemanager_thread::FileManagerThreadMsg;
use rustc_hash::FxHashMap;
use script_bindings::cell::DomRefCell;
use script_bindings::codegen::GenericBindings::HTMLAnchorElementBinding::HTMLAnchorElementMethods;
use script_bindings::codegen::GenericBindings::HTMLImageElementBinding::HTMLImageElementMethods;
use script_bindings::codegen::GenericBindings::HistoryBinding::HistoryMethods;
use script_bindings::codegen::GenericBindings::WindowBinding::WindowMethods;
use script_bindings::inheritance::Castable;
use script_bindings::root::{Dom, DomRoot};
use servo_base::Epoch;
use servo_base::generic_channel::GenericSend;
use servo_url::ServoUrl;
use webrender_api::units::{DeviceIntRect, DevicePoint};
use crate::dom::activation::Activatable;
use crate::dom::bindings::buffer_source::create_buffer_source;
use crate::dom::bindings::error::Error;
use crate::dom::bindings::trace::NoTrace;
use crate::dom::inputevent::HitTestResult;
use crate::dom::iterators::ShadowIncluding;
use crate::dom::links::htmlhyperlinkelementutils::HyperlinkElementTraits;
use crate::dom::node::{Node, NodeTraits};
use crate::dom::promise::{RootedPromise, TracedPromise};
use crate::dom::types::{
    Element, HTMLAnchorElement, HTMLElement, HTMLImageElement, HTMLInputElement, HTMLSelectElement,
    Window,
};
use crate::messaging::MainThreadScriptMsg;
use crate::realms::enter_auto_realm;

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
