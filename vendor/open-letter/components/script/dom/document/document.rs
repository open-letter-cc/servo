// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `components/script/dom/document/document.rs`.

#[allow(unused_imports)]
use super::*;

impl Document {
    /// Called when the system visibility state of this document's `WebView` changes,
    /// because the embedder hid or showed it. Nested documents follow their `WebView`.
    ///
    /// <https://html.spec.whatwg.org/multipage/#system-visibility-state>
    pub(crate) fn system_visibility_changed(&self) {
        // Documents that are not active (for instance those in the session history)
        // get their visibility state again when they are reactivated.
        if !self.is_active() {
            return;
        }
        // Queue a global task on the user interaction task source given the document's
        // relevant global object to update the visibility state of the document to the
        // new visibility state.
        let document = Trusted::new(self);
        self.owner_global()
            .task_manager()
            .user_interaction_task_source()
            .queue(task!(update_visibility_state: move |cx| {
                let document = document.root();
                if !document.is_active() {
                    return;
                }
                let visibility_state = document.window().system_visibility_state();
                document.update_visibility_state(cx, visibility_state);
            }));
    }
}
