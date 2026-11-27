// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `components/layout/traversal.rs`.

#[allow(unused_imports)]
use super::*;

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
