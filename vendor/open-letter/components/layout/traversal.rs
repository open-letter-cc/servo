// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/layout/traversal.rs`.

#[allow(unused_imports)]
use super::*;

use std::sync::Arc;
use layout_api::{
    DangerousStyleElement, DangerousStyleNode, LayoutDamage, LayoutElement, LayoutNode,
    svg_paint_values,
};
use script::layout_dom::ServoLayoutNode;
use style::context::{SharedStyleContext, StyleContext};
use style::dom::{NodeInfo, TElement, TNode};
use style::selector_parser::RestyleDamage;
use style::traversal::{DomTraversal, recalc_style_at};
use crate::BoxTree;
use crate::context::LayoutContext;
use crate::dom::{DOMLayoutData, NodeExt};
use crate::layout_root::LayoutRoot;

/// The outermost `<svg>` element among the inclusive ancestors of `element` that are in
/// the SVG namespace: the inline `<svg>` whose serialization contains `element`.
pub(super) fn outermost_svg_element<E: TElement>(element: E) -> Option<E> {
    let mut outermost = None;
    let mut current = Some(element);
    while let Some(candidate) = current.filter(|candidate| candidate.is_svg_element()) {
        if candidate.has_local_name(&web_atoms::local_name!("svg")) {
            outermost = Some(candidate);
        }
        current = candidate.traversal_parent();
    }
    outermost
}
