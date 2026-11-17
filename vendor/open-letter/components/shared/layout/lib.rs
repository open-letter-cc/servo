// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/shared/layout/lib.rs`.

#[allow(unused_imports)]
use super::*;

use std::any::Any;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicIsize;
use std::thread::JoinHandle;
use std::time::Duration;
use accesskit::{Action, ActionData, ActionRequest};
use app_units::Au;
use background_hang_monitor_api::BackgroundHangMonitorRegister;
use bitflags::bitflags;
use embedder_traits::{Cursor, ScriptToEmbedderChan, Theme, UntrustedNodeAddress, ViewportDetails};
use euclid::{Point2D, Rect};
use fonts::{FontContext, WebFontDocumentContext, WebFontSetDifference};
use libc::c_void;
use malloc_size_of::{MallocSizeOf as MallocSizeOfTrait, MallocSizeOfOps, malloc_size_of_is_0};
use malloc_size_of_derive::MallocSizeOf;
use net_traits::image_cache::{ImageCache, ImageCacheFactory, PendingImageId};
use net_traits::request::InternalRequest;
use paint_api::CrossProcessPaintApi;
use paint_api::display_list::PaintTimingInfo;
use parking_lot::RwLock;
use pixels::{RasterImage, Repeat};
use profile_traits::mem::Report;
use profile_traits::time;
use rustc_hash::{FxHashMap, FxHashSet};
use script_traits::{InitialScriptState, Painter, ScriptThreadMessage};
use serde::{Deserialize, Serialize};
use servo_arc::Arc as ServoArc;
use servo_base::Epoch;
use servo_base::generic_channel::GenericSender;
use servo_base::id::{BrowsingContextId, PipelineId, WebViewId};
use servo_base::text::{RangeAny, Utf32CodeUnits, Utf32CodeUnitsOrNodeOffset};
use servo_url::{ImmutableOrigin, ServoUrl};
use style::Atom;
use style::animation::DocumentAnimationSet;
use style::attr::{AttrValue, parse_double};
use style::context::QuirksMode;
use style::data::ElementDataWrapper;
use style::device::Device;
use style::dom::OpaqueNode;
use style::invalidation::element::restyle_hints::RestyleHint;
use style::properties::style_structs::Font;
use style::properties::{ComputedValues, LonghandId, PropertyDeclarationId, PropertyId};
use style::selector_parser::{PseudoElement, RestyleDamage, Snapshot};
use style::str::char_is_whitespace;
use style::stylesheets::{DocumentStyleSheet, Stylesheet};
use style::stylist::Stylist;
use style::thread_state::{self, ThreadState};
use style::values::computed::Overflow;
use style_traits::CSSPixel;
use uuid::Uuid;
use webrender_api::units::{DeviceIntSize, LayoutPoint, LayoutVector2D};
use webrender_api::{ExternalScrollId, ImageKey};

/// The computed properties an inline `<svg>` subtree is painted with that can come from
/// outside the subtree (the page's stylesheets, inheritance from the `<svg>`'s
/// ancestors, custom properties). The subtree is rasterized from its serialization,
/// which carries none of these, so their computed values are written into it.
pub const SVG_PAINT_PROPERTIES: [(&str, LonghandId); 6] = [
    ("fill", LonghandId::Fill),
    ("stroke", LonghandId::Stroke),
    ("color", LonghandId::Color),
    ("opacity", LonghandId::Opacity),
    ("fill-opacity", LonghandId::FillOpacity),
    ("stroke-opacity", LonghandId::StrokeOpacity),
];

/// The computed values of [`SVG_PAINT_PROPERTIES`] in `style`, serialized as CSS. These
/// are computed values, so `var()` is already substituted; `currentcolor` in `fill` or
/// `stroke` resolves against the `color` written beside it.
pub fn svg_paint_values(style: &ComputedValues) -> [String; 6] {
    SVG_PAINT_PROPERTIES
        .map(|(_, id)| style.computed_value_to_string(PropertyDeclarationId::Longhand(id)))
}
