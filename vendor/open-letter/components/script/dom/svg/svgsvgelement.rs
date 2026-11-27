// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `components/script/dom/svg/svgsvgelement.rs`.

#[allow(unused_imports)]
use super::*;

impl SVGSVGElement {
    /// One line of computed paint values, as written by
    /// [`Self::write_computed_paint_values`].
    pub(super) fn paint_values_line(values: &[String; 6]) -> String {
        values.join("\u{1f}") + "\n"
    }

    /// The computed paint values of every styled SVG element in `root`'s subtree, in
    /// the form [`Self::write_computed_paint_values`] returns them.
    pub(super) fn computed_paint_values(root: &Node) -> String {
        let mut lines = String::new();
        for node in root.traverse_preorder(ShadowIncluding::No) {
            let Some(element) = node.downcast::<Element>() else {
                continue;
            };
            if *element.namespace() != ns!(svg) {
                continue;
            }
            if let Some(style) = element.style_without_reflow() {
                lines.push_str(&Self::paint_values_line(&svg_paint_values(&style)));
            }
        }
        lines
    }

    /// Write the computed paint values of each element of `original`'s subtree into the
    /// matching element of `clone`, its unmodified deep clone, returning the values.
    ///
    /// The clone is rasterized from its serialization alone, so anything it is painted
    /// with from outside itself (the page's stylesheets, inheritance from the `<svg>`'s
    /// ancestors, custom properties, `:hover`) would otherwise be lost, and paint would
    /// fall back to black. Elements later cloned in for `<use>` are not written, so that
    /// they still inherit from the `<use>` element.
    pub(super) fn write_computed_paint_values(cx: &mut JSContext, original: &Node, clone: &Node) -> String {
        let pairs: Vec<_> = original
            .traverse_preorder(ShadowIncluding::No)
            .zip(clone.traverse_preorder(ShadowIncluding::No))
            .collect();
        let mut paint_values = String::new();
        for (original_node, cloned_node) in pairs {
            let (Some(original_element), Some(cloned_element)) = (
                original_node.downcast::<Element>(),
                cloned_node.downcast::<Element>(),
            ) else {
                continue;
            };
            if *original_element.namespace() != ns!(svg) {
                continue;
            }
            if let Some(image) = original_element.downcast::<SVGImageElement>() &&
                let Some(data_url) = image.image_data_url()
            {
                cloned_element.set_string_attribute(
                    cx,
                    &local_name!("href"),
                    DOMString::from(data_url),
                );
            }
            let Some(style) = original_element.style_without_reflow() else {
                continue;
            };
            let values = svg_paint_values(&style);
            let mut declarations = cloned_element
                .get_string_attribute(&local_name!("style"))
                .to_string();
            for ((name, _), value) in SVG_PAINT_PROPERTIES.iter().zip(values.iter()) {
                // A paint server's computed value is an absolute URL, which the
                // rasterizer does not resolve; its `fill` or `stroke` attribute
                // already names it in a form it does.
                if value.contains("url(") {
                    continue;
                }
                // The rasterizer only recognizes the `currentColor` keyword in this case.
                let value = value.replace("currentcolor", "currentColor");
                if !declarations.trim().is_empty() && !declarations.trim_end().ends_with(';') {
                    declarations.push(';');
                }
                declarations.push_str(&format!("{name}:{value};"));
            }
            cloned_element.set_string_attribute(
                cx,
                &local_name!("style"),
                DOMString::from(declarations),
            );
            paint_values.push_str(&Self::paint_values_line(&values));
        }
        paint_values
    }

    /// Serialize this subtree again if a restyle changed the paint values written into
    /// its cached serialization.
    pub(crate) fn reserialize_if_paint_changed(&self, cx: &mut JSContext) {
        let Some(cached) = self.cached_paint_values.borrow().clone() else {
            return;
        };
        if cached == Self::computed_paint_values(self.upcast()) {
            return;
        }
        // Something visible changed: rasterize again from a new serialization.
        self.invalidate_cached_serialized_subtree_and_rasterization_result(cx.no_gc());
        self.serialize_and_cache_subtree(cx);
    }

    /// Discard the serialization and rasterization of this subtree, because something it
    /// depends on that is not a DOM mutation (such as a loaded image) changed.
    pub(crate) fn invalidate_serialization(&self, no_gc: &NoGC) {
        self.invalidate_cached_serialized_subtree_and_rasterization_result(no_gc);
    }
}
