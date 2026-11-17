// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/script/dom/svg/svgimageelement.rs`.

#[allow(unused_imports)]
use super::*;

use std::cell::Cell;
use atomic_refcell::AtomicRefCell;
use base64::Engine as _;
use bytes::Bytes;
use dom_struct::dom_struct;
use html5ever::{LocalName, Prefix, local_name, ns};
use js::rust::HandleObject;
use net_traits::blob_url_store::UrlWithBlobClaim;
use net_traits::request::{Destination, RequestBuilder, RequestId};
use net_traits::{FetchMetadata, NetworkError, ResourceFetchTiming};
use servo_url::ServoUrl;
use style::attr::AttrValue;
use crate::dom::bindings::inheritance::Castable;
use crate::dom::bindings::refcounted::Trusted;
use crate::dom::bindings::root::DomRoot;
use crate::dom::bindings::str::DOMString;
use crate::dom::csp::{GlobalCspReporting, Violation};
use crate::dom::document::Document;
use crate::dom::element::AttributeMutation;
use crate::dom::element::attributes::storage::AttrRef;
use crate::dom::globalscope::GlobalScope;
use crate::dom::node::virtualmethods::VirtualMethods;
use crate::dom::node::{Node, NodeTraits};
use crate::dom::performance::performanceresourcetiming::InitiatorType;
use crate::dom::svg::svggraphicselement::SVGGraphicsElement;
use crate::dom::svg::svgsvgelement::SVGSVGElement;
use crate::event_loop::document_loader::LoadType;
use crate::fetch::fetch::RequestWithGlobalScope;
use crate::fetch::network_listener::{self, FetchResponseListener, ResourceTimingListener};

/// The MIME type of an image the SVG rasterizer can decode, from its first bytes.
pub(super) fn sniff_image_type(bytes: &[u8], content_type: Option<&str>) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if content_type.is_some_and(|content_type| content_type.contains("svg")) {
        Some("image/svg+xml")
    } else {
        None
    }
}

pub(super) struct SVGImageFetchContext {
    pub(super) element: Trusted<SVGImageElement>,
    pub(super) generation: u32,
    pub(super) url: ServoUrl,
    pub(super) content_type: Option<String>,
    pub(super) bytes: Vec<u8>,
}

impl FetchResponseListener for SVGImageFetchContext {
    fn process_request_body(&mut self, _: &GlobalScope, _: RequestId) {}

    fn process_response(
        &mut self,
        _: &mut js::context::JSContext,
        _: &GlobalScope,
        _: RequestId,
        metadata: Result<FetchMetadata, NetworkError>,
    ) {
        // An image may be drawn from a cross-origin, opaque response, as with `<img>`;
        // its pixels never reach script.
        self.content_type = metadata.ok().and_then(|metadata| {
            let metadata = match metadata {
                FetchMetadata::Unfiltered(metadata) => metadata,
                FetchMetadata::Filtered { unsafe_, .. } => unsafe_,
            };
            metadata
                .content_type
                .map(|content_type| content_type.into_inner().to_string())
        });
    }

    fn process_response_chunk(
        &mut self,
        _: &mut js::context::JSContext,
        _: &GlobalScope,
        _: RequestId,
        payload: Bytes,
    ) {
        self.bytes.extend_from_slice(&payload);
    }

    fn process_response_eof(
        self,
        cx: &mut js::context::JSContext,
        global: &GlobalScope,
        _: RequestId,
        response: Result<(), NetworkError>,
        timing: ResourceFetchTiming,
    ) {
        network_listener::submit_timing(cx, global, &self, &response, &timing);
        let element = self.element.root();
        element
            .owner_document()
            .finish_load(LoadType::Image(self.url.clone()), cx);
        if element.fetch_generation.get() != self.generation {
            return;
        }

        let image_type = response
            .ok()
            .and_then(|_| sniff_image_type(&self.bytes, self.content_type.as_deref()));
        let Some(image_type) = image_type else {
            element
                .upcast::<crate::dom::eventtarget::EventTarget>()
                .fire_event(cx, atom!("error"));
            return;
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(&self.bytes);
        *element.image_data_url.borrow_mut() = Some(format!("data:{image_type};base64,{encoded}"));
        element.invalidate_owner_svg(cx.no_gc());
        element
            .upcast::<crate::dom::eventtarget::EventTarget>()
            .fire_event(cx, atom!("load"));
    }

    fn process_csp_violations(
        &mut self,
        cx: &mut js::context::JSContext,
        global: &GlobalScope,
        _: RequestId,
        violations: Vec<Violation>,
    ) {
        global.report_csp_violations(cx, violations, None, None);
    }
}

impl ResourceTimingListener for SVGImageFetchContext {
    fn resource_timing_information(&self) -> (InitiatorType, ServoUrl) {
        (
            InitiatorType::LocalName(local_name!("image")),
            self.url.clone(),
        )
    }
}

impl SVGImageElement {
    /// The loaded image as a `data:` URL, for the serialization of an inline `<svg>`.
    pub(crate) fn image_data_url(&self) -> Option<String> {
        self.image_data_url.borrow().clone()
    }

    /// Make the inline `<svg>` containing this element serialize and rasterize again.
    pub(super) fn invalidate_owner_svg(&self, no_gc: &js::context::NoGC) {
        let outermost_svg = self
            .upcast::<Node>()
            .inclusive_ancestors_unrooted(no_gc, crate::dom::iterators::ShadowIncluding::No)
            .filter_map(|ancestor| ancestor.downcast::<SVGSVGElement>().map(DomRoot::from_ref))
            .last();
        if let Some(svg) = outermost_svg {
            svg.invalidate_serialization(no_gc);
        }
    }

    /// Forget the loaded image, and any fetch in progress.
    pub(super) fn clear_image(&self, no_gc: &js::context::NoGC) {
        self.fetch_generation
            .set(self.fetch_generation.get().wrapping_add(1));
        let had_image = self.image_data_url.borrow_mut().take().is_some();
        if had_image {
            self.invalidate_owner_svg(no_gc);
        }
    }
}
