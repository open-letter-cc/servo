// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `components/shared/fonts/lib.rs`.

#[allow(unused_imports)]
use super::*;

// FontDataAndIndex's Debug is upstream's own derive since the carry onto
// upstream main; ours, which printed the length in place of the bytes, is dropped.
