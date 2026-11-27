// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `components/script/dom/permission/permissions.rs`.

#[allow(unused_imports)]
use super::*;

/// The browsing context of the frame `global_scope` belongs to, reported to the embedder
/// with a permission request so that it can tell frames apart. `None` for a global with
/// no frame of its own, such as a worker.
pub(crate) fn requesting_browsing_context(
    global_scope: &GlobalScope,
) -> Option<servo_base::id::BrowsingContextId> {
    global_scope
        .downcast::<Window>()?
        .undiscarded_window_proxy()
        .map(|window_proxy| window_proxy.browsing_context_id())
}
