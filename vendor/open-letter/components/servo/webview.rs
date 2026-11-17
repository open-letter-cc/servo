// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/servo/webview.rs`.

#[allow(unused_imports)]
use super::*;

impl WebView {
    /// Generic embedder control extension, no domain logic.
    ///
    /// Route `embedder_control` to the delegate. A control whose
    /// [`EmbedderControlTag`](crate::webview_delegate::EmbedderControlTag) bit is set in
    /// [`WebViewDelegate::embedder_control_flags`] is offered to
    /// [`WebViewDelegate::handle_embedder_control`] first; everything else, and anything the
    /// delegate declines, follows the ordinary
    /// [`WebViewDelegate::show_embedder_control`] path.
    ///
    /// `requester_origin` is the origin of the document that asked for the control, which
    /// may be a nested frame's. `None` only for a control with no tag, which is never
    /// offered to the delegate's handler.
    pub(crate) fn dispatch_embedder_control(
        &self,
        embedder_control: EmbedderControl,
        requester_origin: Option<&ImmutableOrigin>,
    ) {
        let delegate = self.delegate();

        let taken_by_embedder = embedder_control
            .tag()
            .is_some_and(|tag| delegate.embedder_control_flags() & tag.bit() != 0);
        // A page request has no user interface to show, so one that is not taken, or that
        // the embedder declines, is dropped here, which rejects the page's promise.
        let show = |embedder_control| {
            if !matches!(embedder_control, EmbedderControl::PageRequest(..)) {
                delegate.show_embedder_control(self.clone(), embedder_control);
            }
        };

        if !taken_by_embedder {
            show(embedder_control);
            return;
        }

        // The embedder returns the control to us if it declined to handle it.
        if let Some(embedder_control) =
            delegate.handle_embedder_control(self.clone(), embedder_control, requester_origin)
        {
            show(embedder_control);
        }
    }
}
