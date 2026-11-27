// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `components/shared/embedder/embedder_controls.rs`.

#[allow(unused_imports)]
use super::*;

impl EmbedderControlId {
    /// The browsing context of the frame whose document asked for this control.
    pub fn browsing_context_id(&self) -> BrowsingContextId {
        self.browsing_context_id
    }
}
