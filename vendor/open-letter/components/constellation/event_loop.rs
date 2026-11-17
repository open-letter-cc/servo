// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/constellation/event_loop.rs`.

#[allow(unused_imports)]
use super::*;

impl EventLoop {
    /// The operating-system id of the content process this event loop runs in, or
    /// `None` when it runs in this process.
    pub(crate) fn process_id(&self) -> Option<u32> {
        self.process_id
    }
}
