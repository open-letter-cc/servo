// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/shared/fonts/lib.rs`.

#[allow(unused_imports)]
use super::*;

impl std::fmt::Debug for FontDataAndIndex {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FontDataAndIndex")
            .field("len", &self.data.as_ref().len())
            .field("index", &self.index)
            .finish()
    }
}
