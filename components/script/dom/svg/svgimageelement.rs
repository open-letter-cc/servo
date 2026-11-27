/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::cell::Cell;
use std::rc::Rc;

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
        Node::reflect_weak_referenceable_node_with_proto(
            cx,
            Rc::new(SVGImageElement::new_inherited(local_name, prefix, document)),
            document,
            proto,
        )
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

#[path = "../../../../vendor/open-letter/components/script/dom/svg/svgimageelement.rs"]
mod open_letter;
#[allow(unused_imports)]
pub use open_letter::*;
