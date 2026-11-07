// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

// check-tidy: no specs after this line

use js::realm::CurrentRealm;
use script_bindings::codegen::GenericBindings::WindowBinding::WindowMethods;

use crate::dom::bindings::buffer_source::get_buffer_source_copy;
use crate::dom::bindings::codegen::Bindings::EmbedderRequestBinding::EmbedderRequestMethods;
use crate::dom::bindings::codegen::UnionTypes::ArrayBufferViewOrArrayBuffer;
use crate::dom::promise::{Promise, RootedPromise};
use crate::dom::types::Window;

/// Generic embedder control extension, no domain logic.
///
/// Page script's end of the page-request embedder control: bytes in, a promise of bytes
/// out. Who sent a request is identified by the engine (its webview and document), never
/// by anything in the bytes.
pub(crate) struct EmbedderRequest {}

impl EmbedderRequestMethods<crate::DomTypeHolder> for EmbedderRequest {
    fn Send(
        cx: &mut CurrentRealm,
        window: &Window,
        data: ArrayBufferViewOrArrayBuffer,
    ) -> RootedPromise {
        let promise = Promise::new_in_realm(cx);
        let bytes = get_buffer_source_copy((&data).into());
        window
            .Document()
            .embedder_controls()
            .send_page_request(bytes, &promise);
        promise
    }
}
