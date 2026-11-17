// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/shared/embedder/embedder_controls.rs`.

#[allow(unused_imports)]
use super::*;

use std::fmt::Debug;
use std::hash::Hash;
use std::path::PathBuf;
use std::time::SystemTime;
use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use servo_base::Epoch;
use servo_base::generic_channel::GenericSender;
use servo_base::id::{BrowsingContextId, PipelineId, WebViewId};
use servo_url::ImmutableOrigin;
use url::Url;
use uuid::Uuid;
use crate::{InputMethodType, RgbColor};

impl EmbedderControlId {
    /// The browsing context of the frame whose document asked for this control.
    pub fn browsing_context_id(&self) -> BrowsingContextId {
        self.browsing_context_id
    }
}
