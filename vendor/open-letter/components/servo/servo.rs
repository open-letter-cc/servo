// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `components/servo/servo.rs`.

#[allow(unused_imports)]
use super::*;

impl Servo {
    pub(crate) fn embedder_proxy(&self) -> &EmbedderProxy {
        &self.0.embedder_proxy
    }
}
