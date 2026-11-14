/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

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

/// <https://svgwg.org/svg2-draft/embedded.html#Placement>
const DEFAULT_WIDTH: u32 = 300;
const DEFAULT_HEIGHT: u32 = 150;

#[dom_struct]
pub(crate) struct SVGImageElement {
    svggraphicselement: SVGGraphicsElement,
    /// The fetched image resource as a `data:` URL, once it has loaded. An inline
    /// `<svg>` is rasterized from a serialization of its subtree with no access to the
    /// network, so the serialization refers to the image by this URL.
    #[no_trace]
    image_data_url: AtomicRefCell<Option<String>>,
    /// Incremented for each fetch, so that the response of a superseded one is ignored.
    fetch_generation: Cell<u32>,
}

/// The MIME type of an image the SVG rasterizer can decode, from its first bytes.
fn sniff_image_type(bytes: &[u8], content_type: Option<&str>) -> Option<&'static str> {
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

struct SVGImageFetchContext {
    element: Trusted<SVGImageElement>,
    generation: u32,
    url: ServoUrl,
    content_type: Option<String>,
    bytes: Vec<u8>,
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
    fn new_inherited(
        local_name: LocalName,
        prefix: Option<Prefix>,
        document: &Document,
    ) -> SVGImageElement {
        SVGImageElement {
            svggraphicselement: SVGGraphicsElement::new_inherited(local_name, prefix, document),
            image_data_url: Default::default(),
            fetch_generation: Cell::new(0),
        }
    }

    pub(crate) fn new(
        cx: &mut js::context::JSContext,
        local_name: LocalName,
        prefix: Option<Prefix>,
        document: &Document,
        proto: Option<HandleObject>,
    ) -> DomRoot<SVGImageElement> {
        Node::reflect_node_with_proto(
            cx,
            Box::new(SVGImageElement::new_inherited(local_name, prefix, document)),
            document,
            proto,
        )
    }

    /// The loaded image as a `data:` URL, for the serialization of an inline `<svg>`.
    pub(crate) fn image_data_url(&self) -> Option<String> {
        self.image_data_url.borrow().clone()
    }

    /// Make the inline `<svg>` containing this element serialize and rasterize again.
    fn invalidate_owner_svg(&self, no_gc: &js::context::NoGC) {
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
    fn clear_image(&self, no_gc: &js::context::NoGC) {
        self.fetch_generation
            .set(self.fetch_generation.get().wrapping_add(1));
        let had_image = self.image_data_url.borrow_mut().take().is_some();
        if had_image {
            self.invalidate_owner_svg(no_gc);
        }
    }

    /// <https://svgwg.org/svg2-draft/linking.html#processingURL>
    fn fetch_image_resource(&self, no_gc: &js::context::NoGC) {
        self.clear_image(no_gc);

        let element = self.upcast::<crate::dom::element::Element>();
        let href = element
            .get_attribute_string_value_with_namespace(&ns!(), &local_name!("href"))
            .or_else(|| {
                element.get_attribute_string_value_with_namespace(&ns!(xlink), &local_name!("href"))
            })
            .unwrap_or_default();
        let document = self.owner_document();
        let Ok(url) = document.base_url().join(&href) else {
            self.owner_global()
                .task_manager()
                .dom_manipulation_task_source()
                .queue_simple_event(self.upcast(), atom!("error"));
            return;
        };

        let context = SVGImageFetchContext {
            element: Trusted::new(self),
            generation: self.fetch_generation.get(),
            url: url.clone(),
            content_type: None,
            bytes: Vec::new(),
        };
        let global = self.owner_global();
        let request = RequestBuilder::new(
            Some(document.webview_id()),
            UrlWithBlobClaim::from_url_without_having_claimed_blob(url.clone()),
            global.get_referrer(),
        )
        .destination(Destination::Image)
        .with_global_scope(&global);
        document.fetch_blocking(LoadType::Image(url), request, context);
    }
}

impl VirtualMethods for SVGImageElement {
    fn super_type(&self) -> Option<&dyn VirtualMethods> {
        Some(self.upcast::<SVGGraphicsElement>() as &dyn VirtualMethods)
    }

    fn attribute_mutated(
        &self,
        cx: &mut js::context::JSContext,
        attr: AttrRef<'_>,
        mutation: AttributeMutation,
    ) {
        self.super_type()
            .unwrap()
            .attribute_mutated(cx, attr, mutation);
        if attr.local_name() == &local_name!("href") &&
            matches!(attr.namespace(), &ns!() | &ns!(xlink))
        {
            match mutation {
                AttributeMutation::Set(_) => self.fetch_image_resource(cx.no_gc()),
                // `href` takes precedence over `xlink:href`; either may remain.
                AttributeMutation::Removed => {
                    let element = self.upcast::<crate::dom::element::Element>();
                    if element.has_attribute(&local_name!("href")) ||
                        element
                            .get_attribute_string_value_with_namespace(
                                &ns!(xlink),
                                &local_name!("href"),
                            )
                            .is_some()
                    {
                        self.fetch_image_resource(cx.no_gc());
                    } else {
                        self.clear_image(cx.no_gc());
                    }
                },
            }
        }
    }

    fn attribute_affects_presentational_hints(&self, attr: AttrRef<'_>) -> bool {
        match attr.local_name() {
            &local_name!("width") | &local_name!("height") => true,
            _ => self
                .super_type()
                .unwrap()
                .attribute_affects_presentational_hints(attr),
        }
    }

    fn parse_plain_attribute(&self, name: &LocalName, value: DOMString) -> AttrValue {
        match *name {
            local_name!("width") => AttrValue::from_u32(value.into(), DEFAULT_WIDTH),
            local_name!("height") => AttrValue::from_u32(value.into(), DEFAULT_HEIGHT),
            _ => self
                .super_type()
                .unwrap()
                .parse_plain_attribute(name, value),
        }
    }
}
