// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/script/dom/window/window.rs`.

#[allow(unused_imports)]
use super::*;

use std::cell::{Cell, RefCell, RefMut};
use std::collections::hash_map::Entry;
use std::default::Default;
use std::ffi::c_void;
use std::io::{Write, stderr, stdout};
use std::ptr::NonNull;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::{Duration, Instant};
use accesskit::Action;
use app_units::Au;
use base64::Engine;
use content_security_policy::Violation;
use content_security_policy::sandboxing_directive::SandboxingFlagSet;
use crossbeam_channel::{Sender, unbounded};
use cssparser::SourceLocation;
use dom_struct::dom_struct;
use embedder_traits::user_contents::UserScript;
use embedder_traits::{
    AlertResponse, ConfirmResponse, EmbedderMsg, LoadStatus, PromptResponse, ScriptToEmbedderChan,
    SimpleDialogRequest, Theme, UntrustedNodeAddress, ViewportDetails, WebDriverLoadStatus,
};
use euclid::{Point2D, Rect, Scale, Size2D, Vector2D};
use fonts::{
    CspViolationHandler, FontContext, NetworkTimingHandler, WebFontDocumentContext,
    WebFontSetDifference,
};
use html5ever::LocalName;
use js::context::{JSContext, NoGC};
use js::conversions::ToJSValConvertible;
use js::glue::DumpJSStack;
use js::jsapi::{
    GCReason, GetObjectRealmOrNull, JSContext as RawJSContext, JSObject, SetRealmPrincipals,
};
use js::jsval::{NullValue, UndefinedValue};
use js::realm::{AutoRealm, CurrentRealm};
use js::rust::wrappers2::{JS_GC, JS_MaybeGC};
use js::rust::{
    CustomAutoRooterGuard, HandleObject, HandleValue, MutableHandleObject, MutableHandleValue,
};
use layout_api::{
    AccessibilityActionRequest, AxesOverflow, BoxAreaType, CSSPixelRectVec, ContainerTimingRecord,
    FragmentType, HitTestFlags, LCPCandidate, Layout, LayoutImageDestination, PendingImage,
    PendingImageState, PendingRasterizationImage, PhysicalSides, QueryMsg, ReflowGoal,
    ReflowPhasesRun, ReflowRequest, ReflowRequestAccessibility, ReflowRequestRestyle,
    ReflowStatistics, RestyleReason, ScrollContainerQueryFlags, ScrollContainerResponse,
    SegmentGranularity, TrustedNodeAddress, combine_id_with_fragment_type,
};
use malloc_size_of::MallocSizeOf;
use media::WindowGLContext;
use net_traits::image_cache::{
    ImageCache, ImageCacheResponseCallback, ImageCacheResponseMessage, ImageLoadListener,
    ImageResponse, PendingImageId, PendingImageResponse, RasterizationCompleteResponse,
};
use net_traits::request::{Origin, Referrer, RequestClient};
use net_traits::{ResourceFetchTiming, ResourceThreads};
use num_traits::ToPrimitive;
use paint_api::{CrossProcessPaintApi, PinchZoomInfos};
use profile_traits::generic_channel as ProfiledGenericChannel;
use profile_traits::mem::ProfilerChan as MemProfilerChan;
use profile_traits::time::ProfilerChan as TimeProfilerChan;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use script_bindings::cell::{DomRefCell, Ref};
use script_bindings::dom::UnrootedDom;
use script_bindings::domstring::TracedDOMString;
use script_bindings::interfaces::{HasOrigin, WindowHelpers};
use script_bindings::js::{JS_DefineProperty, PropertyFlags};
use script_bindings::like::Setlike;
use script_bindings::principals::ServoJSPrincipals;
use script_bindings::reflector::DomObject;
use script_bindings::root::Root;
use script_traits::{ConstellationInputEvent, ScriptThreadMessage, WebViewState};
use selectors::attr::CaseSensitivity;
use servo_arc::Arc as ServoArc;
use servo_base::cross_process_instant::CrossProcessInstant;
use servo_base::generic_channel::{self, GenericCallback, GenericSender};
use servo_base::id::{BrowsingContextId, PipelineId, WebViewId};
use servo_base::text::{Utf32CodeUnits, Utf32CodeUnitsOrNodeOffset};
use servo_bluetooth_traits::BluetoothRequest;
use servo_canvas_traits::webgl::WebGLChan;
use servo_config::pref;
use servo_constellation_traits::{
    LoadData, LoadOrigin, ScreenshotReadinessResponse, ScriptToConstellationMessage,
    ScriptToConstellationSender, StructuredSerializedData, WindowSizeType,
};
use servo_geometry::DeviceIndependentIntRect;
use servo_url::{ImmutableOrigin, MutableOrigin, ServoUrl};
use storage_traits::StorageThreads;
use storage_traits::webstorage_thread::WebStorageType;
use style::dom::OpaqueNode;
use style::error_reporting::{ContextualParseError, ParseErrorReporter};
use style::properties::PropertyId;
use style::properties::style_structs::Font;
use style::selector_parser::PseudoElement;
use style::str::HTML_SPACE_CHARACTERS;
use style::stylesheets::UrlExtraData;
use style_traits::CSSPixel;
use stylo_atoms::Atom;
use time::Duration as TimeDuration;
use webrender_api::ExternalScrollId;
use webrender_api::units::{DeviceIntSize, DevicePixel, LayoutPixel, LayoutPoint};
use crate::dom::StatelessWorkletThreadPool;
use crate::dom::bindings::callback::RootedCallback;
use crate::dom::bindings::codegen::Bindings::AnimationFrameProviderBinding::FrameRequestCallback;
use crate::dom::bindings::codegen::Bindings::DocumentBinding::{
    DocumentMethods, DocumentReadyState, DocumentVisibilityState, NamedPropertyValue,
};
use crate::dom::bindings::codegen::Bindings::HTMLIFrameElementBinding::HTMLIFrameElementMethods;
use crate::dom::bindings::codegen::Bindings::HistoryBinding::History_Binding::HistoryMethods;
use crate::dom::bindings::codegen::Bindings::ImageBitmapBinding::{
    ImageBitmapOptions, ImageBitmapSource,
};
use crate::dom::bindings::codegen::Bindings::MediaQueryListBinding::MediaQueryList_Binding::MediaQueryListMethods;
use crate::dom::bindings::codegen::Bindings::MessagePortBinding::StructuredSerializeOptions;
use crate::dom::bindings::codegen::Bindings::ReportingObserverBinding::Report;
use crate::dom::bindings::codegen::Bindings::RequestBinding::{RequestInfo, RequestInit};
use crate::dom::bindings::codegen::Bindings::VoidFunctionBinding::VoidFunction;
use crate::dom::bindings::codegen::Bindings::WindowBinding::{
    self, DeferredRequestInit, ScrollBehavior, ScrollToOptions, WindowMethods,
    WindowPostMessageOptions,
};
use crate::dom::bindings::codegen::UnionTypes::{
    RequestOrUSVString, TrustedScriptOrString, TrustedScriptOrStringOrFunction,
};
use crate::dom::bindings::error::{Error, ErrorResult, Fallible};
use crate::dom::bindings::inheritance::{Castable, ElementTypeId, HTMLElementTypeId, NodeTypeId};
use crate::dom::bindings::num::Finite;
use crate::dom::bindings::refcounted::Trusted;
use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::bindings::root::{Dom, DomRoot, MutNullableDom};
use crate::dom::bindings::str::{DOMString, USVString};
use crate::dom::bindings::structuredclone;
use crate::dom::bindings::trace::{CustomTraceable, HashMapTracedValues, JSTraceable};
use crate::dom::bindings::utils::GlobalStaticData;
use crate::dom::bindings::weakref::DOMTracker;
use crate::dom::bluetooth::BluetoothExtraPermissionData;
use crate::dom::closewatchermanager::CloseWatcherManager;
use crate::dom::cookiestore::cookiestore::CookieStore;
use crate::dom::csp::GlobalCspReporting;
use crate::dom::css::cssstyledeclaration::{
    CSSModificationAccess, CSSStyleDeclaration, CSSStyleOwner,
};
use crate::dom::customelementregistry::CustomElementRegistry;
use crate::dom::document::focus::FocusableArea;
use crate::dom::document::{
    AbortReason, AnimationFrameCallback, Document, LocalDescendantNavigablesIterator,
};
use crate::dom::element::Element;
use crate::dom::event::{Event, EventBubbles, EventCancelable};
use crate::dom::eventtarget::EventTarget;
use crate::dom::fetchlaterresult::FetchLaterResult;
use crate::dom::geolocation::Geolocation;
use crate::dom::globalscope::GlobalScope;
use crate::dom::history::History;
use crate::dom::html::htmlcollection::{CollectionFilter, HTMLCollection};
use crate::dom::html::htmliframeelement::HTMLIFrameElement;
use crate::dom::idbfactory::IDBFactory;
use crate::dom::inputevent::HitTestResult;
use crate::dom::location::Location;
use crate::dom::medialist::MediaList;
use crate::dom::mediaquerylist::{MediaQueryList, MediaQueryListMatchState};
use crate::dom::mediaquerylistevent::MediaQueryListEvent;
use crate::dom::messageevent::MessageEvent;
use crate::dom::navigator::Navigator;
use crate::dom::node::focus::FocusTrigger;
use crate::dom::node::{Node, NodeDamage, NodeTraits, from_untrusted_node_address};
use crate::dom::performance::performance::Performance;
use crate::dom::performanceresourcetiming::InitiatorType;
use crate::dom::promise::RootedPromise;
use crate::dom::reporting::reportingendpoint::{ReportingEndpoint, SendReportsToEndpoints};
use crate::dom::reporting::reportingobserver::ReportingObserver;
use crate::dom::selection::Selection;
use crate::dom::selection_range::RootedSelectionBoundary;
use crate::dom::serviceworker::cachestorage::CacheStorage;
use crate::dom::shadowroot::shadowroot::ShadowRoot;
use crate::dom::storage::Storage;
use crate::dom::testrunner::TestRunner;
use crate::dom::trustedtypes::trustedtypepolicyfactory::TrustedTypePolicyFactory;
use crate::dom::types::{FontFace, ImageBitmap, SVGSVGElement, UIEvent};
use crate::dom::visualviewport::{VisualViewport, VisualViewportChanges};
use crate::dom::webgpu::identityhub::IdentityHub;
use crate::dom::window::layout_image::fetch_image_for_layout;
use crate::dom::window::screen::Screen;
use crate::dom::window::scrolling_box::{ScrollingBox, ScrollingBoxSource};
use crate::dom::window::useractivation::UserActivationTimestamp;
use crate::dom::windowproxy::{WindowProxy, WindowProxyHandler};
use crate::dom::worklet::Worklet;
use crate::dom::workletglobalscope::WorkletGlobalScopeType;
use crate::event_loop::script_thread::ScriptThread;
use crate::event_loop::script_window_proxies::ScriptWindowProxies;
use crate::event_loop::timers::{IsInterval, OneshotTimers, TimerCallback};
use crate::event_loop::webdriver_handlers::find_node_by_unique_id_in_document;
use crate::fetch::fetch;
use crate::fetch::network_listener::{ResourceTimingListener, submit_timing};
use crate::messaging::{MainThreadScriptMsg, ScriptEventLoopReceiver, ScriptEventLoopSender};
use crate::realms::enter_auto_realm;
use crate::runtime::job_queue::UserMicrotask;
use crate::runtime::script_runtime::Runtime;
use crate::tasks::task_manager::TaskManager;
use crate::tasks::task_source::SendableTaskSource;
use crate::unminify::unminified_path;
use crate::window_named_properties;

impl Window {
    /// The visibility state that this [`Window`]'s documents should have given whether
    /// the embedder has hidden its `WebView`.
    /// <https://html.spec.whatwg.org/multipage/#system-visibility-state>
    pub(crate) fn system_visibility_state(&self) -> DocumentVisibilityState {
        if self.webview_state.hidden.get() {
            DocumentVisibilityState::Hidden
        } else {
            DocumentVisibilityState::Visible
        }
    }
}
