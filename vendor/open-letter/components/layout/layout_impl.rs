// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/layout/layout_impl.rs`.

#[allow(unused_imports)]
use super::*;

use std::collections::HashMap;
use std::fmt::Debug;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use accesskit::{ActionRequest, Node};
use app_units::Au;
use bitflags::bitflags;
use embedder_traits::{
    EmbedderMsg, ScriptToEmbedderChan, Theme, UntrustedNodeAddress, ViewportDetails,
};
use encoding_rs::ISO_8859_8;
use euclid::{Point2D, Rect, Scale, Size2D};
use fonts::{FontContext, FontContextWebFontMethods};
use fonts_traits::{StylesheetWebFontLoadFinishedCallback, WebFontSetDifference};
use icu_locale_core::subtags::Language;
use layout_api::{
    AccessibilityActionRequest, AccessibilityDamage, AxesOverflow, BoxAreaType, CSSPixelRectVec,
    ContainerTimingRecord, DangerousStyleNode, HitTestFlags, HitTestResult, IFrameSizes, Layout,
    LayoutConfig, LayoutDamage, LayoutElement, LayoutFactory, LayoutNode, NodeRenderingType,
    OffsetParentResponse, PhysicalSides, QueryMsg, ReflowGoal, ReflowPhasesRun, ReflowRequest,
    ReflowRequestAccessibility, ReflowRequestRestyle, ReflowResult, ReflowStatistics,
    ScrollContainerQueryFlags, ScrollContainerResponse, SegmentGranularity, TrustedNodeAddress,
    with_layout_state,
};
use log::{debug, warn};
use malloc_size_of::{MallocConditionalSizeOf, MallocSizeOf, MallocSizeOfOps};
use net_traits::image_cache::ImageCache;
use paint_api::CrossProcessPaintApi;
use paint_api::display_list::{AxesScrollSensitivity, PaintDisplayListInfo, ScrollType};
use parking_lot::{Mutex, RwLock};
use profile_traits::mem::{Report, ReportKind};
use profile_traits::time::{
    self as profile_time, TimerMetadata, TimerMetadataFrameType, TimerMetadataReflowType,
};
use profile_traits::{path, time_profile};
use rustc_hash::FxHashMap;
use script::layout_dom::{
    ServoDangerousStyleDocument, ServoDangerousStyleElement, ServoLayoutElement, ServoLayoutNode,
};
use script_traits::{DrawAPaintImageResult, PaintWorkletError, Painter, ScriptThreadMessage};
use servo_arc::Arc as ServoArc;
use servo_base::Epoch;
use servo_base::id::{PipelineId, WebViewId};
use servo_base::text::{Utf32CodeUnits, Utf32CodeUnitsOrNodeOffset};
use servo_config::opts::{self, DiagnosticsLogging, DiagnosticsLoggingOption};
use servo_config::pref;
use servo_url::ServoUrl;
use style::animation::DocumentAnimationSet;
use style::context::{
    QuirksMode, RegisteredSpeculativePainter, RegisteredSpeculativePainters, SharedStyleContext,
};
use style::device::Device;
use style::device::servo::{FontMetricsProvider, MediaData};
use style::dom::{OpaqueNode, ShowSubtreeDataAndPrimaryValues, TDocument, TElement, TNode};
use style::font_metrics::FontMetrics;
use style::global_style_data::GLOBAL_STYLE_DATA;
use style::invalidation::element::restyle_hints::RestyleHint;
use style::invalidation::stylesheets::StylesheetInvalidationSet;
use style::media_queries::{MediaList, MediaType};
use style::properties::style_structs::Font;
use style::properties::{ComputedValues, LonghandId, NonCustomPropertyId, PropertyId, ShorthandId};
use style::queries::values::PrefersColorScheme;
use style::selector_parser::{PseudoElement, SnapshotMap};
use style::servo::media_features::PointerCapabilities;
use style::shared_lock::{SharedRwLock, StylesheetGuards};
use style::stylesheets::{DocumentStyleSheet, Origin, Stylesheet};
use style::stylist::Stylist;
use style::traversal::DomTraversal;
use style::traversal_flags::TraversalFlags;
use style::values::computed::font::GenericFontFamily;
use style::values::computed::{CSSPixelLength, FontSize, Length, NonNegativeLength};
use style::values::specified::font::{KeywordInfo, QueryFontMetricsFlags};
use style::{Zero, driver};
use style_traits::{CSSPixel, SpeculativePainter};
use stylo_atoms::Atom;
use url::Url;
use webrender_api::ExternalScrollId;
use webrender_api::units::{DevicePixel, LayoutVector2D};
use crate::accessibility::{AccessibilityContext, AccessibilityDamageMap, AccessibilityTree};
use crate::context::{CachedImageOrError, ImageResolver, LayoutContext};
use crate::display_list::{DisplayListBuilder, HitTest, PaintTimingHandler, StackingContextTree};
use crate::dom::NodeExt;
use crate::query::{
    BoxAreaInclusion, find_character_offset_in_fragment_descendants, get_the_text_steps,
    process_box_area_request, process_box_areas_request, process_client_rect_request,
    process_containing_block_descendant_query, process_containing_block_query,
    process_current_css_zoom_query, process_effective_overflow_query,
    process_node_scroll_area_request, process_offset_parent_query, process_padding_request,
    process_resolved_font_style_query, process_resolved_style_request,
    process_scroll_container_query,
};
use crate::selection::PositionInInlineFormattingContext;
use crate::traversal::{RecalcStyle, compute_damage_and_rebuild_box_tree};
use crate::{BoxTree, FragmentTree};

/// The most threads the style system's per-thread data supports in one pool.
pub(super) const MAX_LAYOUT_POOL_THREADS: usize = 6;

/// A pool to style and lay out in parallel on, used in place of the style system's
/// pool when the `thread_layout_stack_size_kb` preference asks for larger stacks than
/// that pool's threads have. Parallel layout recurses with the depth of the page, and
/// unlike styling it has no depth limiter, so a deeply nested page can exhaust the style
/// threads' stacks. Built from the preferences when first used.
pub(super) static LAYOUT_THREAD_POOL: LazyLock<Option<rayon::ThreadPool>> = LazyLock::new(|| {
    let stack_size_kb = pref!(thread_layout_stack_size_kb).max(0) as usize;
    // Smaller stacks than the style system's would defeat its depth limiter.
    if stack_size_kb <= style::parallel::STYLE_THREAD_STACK_SIZE_KB {
        return None;
    }
    // The same number of threads as the style system's pool.
    let threads = pref!(layout_threads);
    let threads = if threads >= 0 {
        threads as usize
    } else {
        std::thread::available_parallelism().map_or(1, |cores| (cores.get() * 3 / 4).max(1))
    };
    let threads = threads.min(MAX_LAYOUT_POOL_THREADS);
    if threads <= 1 {
        return None;
    }
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|index| format!("Layout#{index}"))
        .stack_size(stack_size_kb * 1024)
        .start_handler(|_| style::thread_state::initialize_layout_worker_thread())
        .build()
        .ok()
});
