// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/shared/fonts/lib.rs`.

#[allow(unused_imports)]
use super::*;

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use malloc_size_of_derive::MallocSizeOf;
use serde::{Deserialize, Serialize};
use servo_arc::Arc as ServoArc;
use servo_base::generic_channel::GenericSharedMemory;
use style::font_face::Descriptors;
use style::stylesheets::LockedFontFaceRule;

impl std::fmt::Debug for FontDataAndIndex {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FontDataAndIndex")
            .field("len", &self.data.as_ref().len())
            .field("index", &self.index)
            .finish()
    }
}
