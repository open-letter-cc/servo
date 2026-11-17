// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/shared/layout/lib.rs`.

#[allow(unused_imports)]
use super::*;

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
