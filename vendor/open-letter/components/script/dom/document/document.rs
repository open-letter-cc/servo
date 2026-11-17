// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/script/dom/document/document.rs`.

#[allow(unused_imports)]
use super::*;

use std::cmp::Ordering;
use std::collections::hash_map::Entry::{Occupied, Vacant};
use std::collections::{HashMap, HashSet, VecDeque};
use std::default::Default;
use std::ops::Deref;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::{Arc as StdArc, LazyLock};
use std::time::Duration;
use atomic_refcell::{AtomicRef, AtomicRefCell};
use bitflags::bitflags;
use chrono::Local;
use content_security_policy::sandboxing_directive::SandboxingFlagSet;
use content_security_policy::{CspList, Policy as CspPolicy, PolicyDisposition};
use cookie::Cookie;
use data_url::mime::Mime;
use devtools_traits::ScriptToDevtoolsControlMsg;
use dom_struct::dom_struct;
use embedder_traits::{
    AllowOrDeny, AnimationState, CustomHandlersAutomationMode, EmbedderMsg, Image, LoadStatus,
    Theme,
};
use encoding_rs::{Encoding, UTF_8};
use html5ever::{LocalName, QualName, local_name, ns};
use hyper_serde::Serde;
use indexmap::IndexSet;
use js::context::{JSContext, NoGC};
use js::jsapi::JSObject;
use js::realm::CurrentRealm;
use js::rust::{HandleObject, HandleValue};
use layout_api::{
    ContainerTimingRecord, LCPCandidate, PendingRestyle, ReflowGoal, ReflowPhasesRun,
    ReflowStatistics, RestyleReason, ScrollContainerQueryFlags, TrustedNodeAddress,
};
use malloc_size_of::MallocSizeOfOps;
use metrics::{InteractiveFlag, InteractiveWindow, ProgressiveWebMetrics};
use net_traits::CookieSource::NonHTTP;
use net_traits::CoreResourceMsg::{GetCookieStringForUrl, SetCookiesForUrl};
use net_traits::image_cache::ImageCache;
use net_traits::policy_container::PolicyContainer;
use net_traits::pub_domains::{Site, is_pub_domain};
use net_traits::request::{
    InsecureRequestsPolicy, PreloadId, PreloadKey, PreloadedResources, RequestBuilder,
};
use net_traits::{ReferrerPolicy, ResourceFetchTiming};
use paint_api::display_list::PaintTimingInfo;
use percent_encoding::percent_decode;
use profile_traits::mem::{Report, ReportKind};
use profile_traits::time::TimerMetadataFrameType;
use profile_traits::{generic_channel as profile_generic_channel, path};
use regex::bytes::Regex;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use script_bindings::callback::{RootedCallback, ThisReflector};
use script_bindings::cell::{DomRefCell, Ref, RefMut};
use script_bindings::domstring::TracedDOMString;
use script_bindings::interfaces::DocumentHelpers;
use script_bindings::reflector::reflect_dom_object_with_proto;
use script_bindings::tasks::NonSendTaskBox;
use script_bindings::trace::CustomTraceable;
use script_traits::{DocumentActivity, ProgressiveWebMetricType};
use servo_arc::Arc;
use servo_base::cross_process_instant::CrossProcessInstant;
use servo_base::generic_channel::GenericSend;
use servo_base::id::{ContainerTimingID, LCPCandidateID, PipelineId, WebViewId};
use servo_base::{Epoch, generic_channel};
use servo_config::pref;
use servo_constellation_traits::{
    NavigationHistoryBehavior, PaintMetricEvent, ScriptToConstellationMessage,
};
use servo_media::{ClientContextId, ServoMedia};
use servo_url::{ImmutableOrigin, MutableOrigin, ServoUrl};
use style::attr::AttrValue;
use style::context::QuirksMode;
use style::dom::OpaqueNode;
use style::invalidation::element::restyle_hints::RestyleHint;
use style::selector_parser::Snapshot;
use style::shared_lock::{SharedRwLock, SharedRwLockReadGuard};
use style::str::{split_html_space_chars, str_join};
use style::stylesheet_set::DocumentStylesheetSet;
use style::stylesheets::{Origin, OriginSet, Stylesheet};
use style::stylist::Stylist;
use stylo_atoms::Atom;
use stylo_dom::ElementState;
use time::Duration as TimeDuration;
use url::{Host, Position};
use crate::css::stylesheet_loader::StylesheetContextId;
use crate::css::stylesheet_set::StylesheetSetRef;
use crate::dom::animationtimeline::AnimationTimeline;
use crate::dom::attr::Attr;
use crate::dom::beforeunloadevent::BeforeUnloadEvent;
use crate::dom::bindings::callback::{ExceptionHandling, TracedCallback};
use crate::dom::bindings::codegen::Bindings::AnimationFrameProviderBinding::FrameRequestCallback;
use crate::dom::bindings::codegen::Bindings::BeforeUnloadEventBinding::BeforeUnloadEvent_Binding::BeforeUnloadEventMethods;
use crate::dom::bindings::codegen::Bindings::DocumentBinding::{
    DocumentMethods, DocumentReadyState, DocumentVisibilityState, ElementCreationOptions,
    NamedPropertyValue,
};
use crate::dom::bindings::codegen::Bindings::ElementBinding::ScrollLogicalPosition;
use crate::dom::bindings::codegen::Bindings::EventBinding::Event_Binding::EventMethods;
use crate::dom::bindings::codegen::Bindings::HTMLElementBinding::HTMLElementMethods;
use crate::dom::bindings::codegen::Bindings::HTMLIFrameElementBinding::HTMLIFrameElement_Binding::HTMLIFrameElementMethods;
use crate::dom::bindings::codegen::Bindings::NavigatorBinding::Navigator_Binding::NavigatorMethods;
use crate::dom::bindings::codegen::Bindings::NodeBinding::NodeMethods;
use crate::dom::bindings::codegen::Bindings::NodeFilterBinding::NodeFilter;
use crate::dom::bindings::codegen::Bindings::PerformanceBinding::PerformanceMethods;
use crate::dom::bindings::codegen::Bindings::PermissionStatusBinding::PermissionName;
use crate::dom::bindings::codegen::Bindings::SanitizerBinding::{
    SetHTMLOptions, SetHTMLUnsafeOptions,
};
use crate::dom::bindings::codegen::Bindings::WindowBinding::{ScrollBehavior, WindowMethods};
use crate::dom::bindings::codegen::Bindings::XPathEvaluatorBinding::XPathEvaluatorMethods;
use crate::dom::bindings::codegen::Bindings::XPathNSResolverBinding::XPathNSResolver;
use crate::dom::bindings::codegen::UnionTypes::{
    BooleanOrImportNodeOptions, NodeOrString, StringOrElementCreationOptions, TrustedHTMLOrString,
};
use crate::dom::bindings::domname::{
    self, is_valid_attribute_local_name, is_valid_element_local_name, namespace_from_domstring,
};
use crate::dom::bindings::error::{Error, ErrorInfo, ErrorResult, Fallible};
use crate::dom::bindings::inheritance::{Castable, ElementTypeId, HTMLElementTypeId, NodeTypeId};
use crate::dom::bindings::num::Finite;
use crate::dom::bindings::refcounted::Trusted;
use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::bindings::root::{
    Dom, DomRoot, LayoutDom, MutNullableDom, ToLayout, ToLayoutOptional, UnrootedDom,
};
use crate::dom::bindings::str::{DOMString, USVString};
use crate::dom::bindings::trace::{HashMapTracedValues, NoTrace};
use crate::dom::bindings::weakref::DOMTracker;
use crate::dom::bindings::xmlname::matches_name_production;
use crate::dom::cdatasection::CDATASection;
use crate::dom::comment::Comment;
use crate::dom::compositionevent::CompositionEvent;
use crate::dom::css::cssstylesheet::CSSStyleSheet;
use crate::dom::css::fontfaceset::FontFaceSet;
use crate::dom::css::stylesheetlist::{StyleSheetList, StyleSheetListOwner};
use crate::dom::customelementregistry::{CustomElementReactionStack, CustomElementRegistry};
use crate::dom::customevent::CustomEvent;
use crate::dom::document::accessibility_data::AccessibilityData;
use crate::dom::document::animation_manager::AnimationManager;
use crate::dom::document::focus::{DocumentFocusHandler, FocusableArea};
use crate::dom::document::iframe_collection::IFrameCollection;
use crate::dom::document::tree_ordered_index_map::TreeOrderedIndexMap;
use crate::dom::document::websocket::WebSocket;
use crate::dom::document_embedder_controls::DocumentEmbedderControls;
use crate::dom::document_event_handler::DocumentEventHandler;
use crate::dom::documentfragment::DocumentFragment;
use crate::dom::documentorshadowroot::{
    DocumentOrShadowRoot, ServoStylesheetInDocument, StylesheetSource,
};
use crate::dom::documenttimeline::DocumentTimeline;
use crate::dom::documenttype::DocumentType;
use crate::dom::domimplementation::DOMImplementation;
use crate::dom::domstringlist::DOMStringList;
use crate::dom::element::attributes::storage::AttrRef;
use crate::dom::element::{CustomElementCreationMode, Element, ElementCreator};
use crate::dom::event::{Event, EventBubbles, EventCancelable};
use crate::dom::eventtarget::EventTarget;
use crate::dom::execcommand::basecommand::{CommandName, DefaultSingleLineContainerName};
use crate::dom::execcommand::execcommands::DocumentExecCommandSupport;
use crate::dom::focusevent::FocusEvent;
use crate::dom::globalscope::GlobalScope;
use crate::dom::hashchangeevent::HashChangeEvent;
use crate::dom::history::History;
use crate::dom::html::htmlanchorelement::HTMLAnchorElement;
use crate::dom::html::htmlareaelement::HTMLAreaElement;
use crate::dom::html::htmlbaseelement::HTMLBaseElement;
use crate::dom::html::htmlcollection::{CollectionFilter, HTMLCollection};
use crate::dom::html::htmldialogelement::HTMLDialogElement;
use crate::dom::html::htmlelement::HTMLElement;
use crate::dom::html::htmlembedelement::HTMLEmbedElement;
use crate::dom::html::htmlformelement::{FormControl, FormControlElementHelpers, HTMLFormElement};
use crate::dom::html::htmlheadelement::HTMLHeadElement;
use crate::dom::html::htmlhtmlelement::HTMLHtmlElement;
use crate::dom::html::htmliframeelement::HTMLIFrameElement;
use crate::dom::html::htmlimageelement::HTMLImageElement;
use crate::dom::html::htmlmediaelement::HTMLMediaElement;
use crate::dom::html::htmlscriptelement::{HTMLScriptElement, ScriptResult, TracedScriptResult};
use crate::dom::html::htmltitleelement::HTMLTitleElement;
use crate::dom::htmldetailselement::DetailsNameGroups;
use crate::dom::intersectionobserver::IntersectionObserver;
use crate::dom::iterators::ShadowIncluding;
use crate::dom::keyboardevent::KeyboardEvent;
use crate::dom::largestcontentfulpaint::LargestContentfulPaint;
use crate::dom::location::Location;
use crate::dom::messageevent::MessageEvent;
use crate::dom::mouseevent::MouseEvent;
use crate::dom::node::focus::FocusTrigger;
use crate::dom::node::treewalker::TreeWalker;
use crate::dom::node::virtualmethods::vtable_for;
use crate::dom::node::{Node, NodeDamage, NodeFlags, NodeTraits};
use crate::dom::nodeiterator::NodeIterator;
use crate::dom::nodelist::NodeList;
use crate::dom::pagetransitionevent::PageTransitionEvent;
use crate::dom::performance::performancecontainertiming::PerformanceContainerTiming;
use crate::dom::performance::performanceentry::PerformanceEntry;
use crate::dom::performance::performancepainttiming::PerformancePaintTiming;
use crate::dom::processinginstruction::ProcessingInstruction;
use crate::dom::range::Range;
use crate::dom::resizeobserver::{ResizeObservationDepth, ResizeObserver};
use crate::dom::sanitizer::Sanitizer;
use crate::dom::selection::Selection;
use crate::dom::servoparser::ServoParser;
use crate::dom::shadowroot::shadowroot::ShadowRoot;
use crate::dom::storageevent::StorageEvent;
use crate::dom::text::Text;
use crate::dom::textevent::TextEvent;
use crate::dom::touchevent::TouchEvent as DomTouchEvent;
use crate::dom::touchlist::TouchList;
use crate::dom::trustedtypes::trustedhtml::TrustedHTML;
use crate::dom::types::{HTMLCanvasElement, VisibilityStateEntry};
use crate::dom::uievent::UIEvent;
use crate::dom::window::Window;
use crate::dom::window::scrolling_box::{ScrollAxisState, ScrollingBox};
use crate::dom::windowproxy::WindowProxy;
use crate::dom::xpathevaluator::XPathEvaluator;
use crate::dom::xpathexpression::XPathExpression;
use crate::dom::{FlatTreeParent, RootedPromise, WeakRangeVec};
use crate::event_loop::document_loader::{DocumentLoader, LoadType};
use crate::event_loop::script_thread::{ScriptThread, SharedRwLocks};
use crate::event_loop::timers::{OneshotTimerCallback, OneshotTimers};
use crate::fetch::fetch::DeferredFetchRecordInvokeState;
use crate::fetch::network_listener::FetchResponseListener;
use crate::mime::{APPLICATION, CHARSET};
use crate::navigation::navigate;
use crate::runtime::script_runtime::compute_size;
use crate::scripting::script_module::{ModuleRequest, ModuleStatus};
use crate::tasks::task_manager::TaskManager;
use crate::tasks::task_source::TaskSourceName;
use crate::xpath::parse_expression;

impl Document {
    /// Called when the system visibility state of this document's `WebView` changes,
    /// because the embedder hid or showed it. Nested documents follow their `WebView`.
    ///
    /// <https://html.spec.whatwg.org/multipage/#system-visibility-state>
    pub(crate) fn system_visibility_changed(&self) {
        // Documents that are not active (for instance those in the session history)
        // get their visibility state again when they are reactivated.
        if !self.is_active() {
            return;
        }
        // Queue a global task on the user interaction task source given the document's
        // relevant global object to update the visibility state of the document to the
        // new visibility state.
        let document = Trusted::new(self);
        self.owner_global()
            .task_manager()
            .user_interaction_task_source()
            .queue(task!(update_visibility_state: move |cx| {
                let document = document.root();
                if !document.is_active() {
                    return;
                }
                let visibility_state = document.window().system_visibility_state();
                document.update_visibility_state(cx, visibility_state);
            }));
    }
}
