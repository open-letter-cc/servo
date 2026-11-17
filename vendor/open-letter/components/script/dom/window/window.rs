// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/script/dom/window/window.rs`.

#[allow(unused_imports)]
use super::*;

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
