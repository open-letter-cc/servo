// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/storage/client_storage.rs`.

#[allow(unused_imports)]
use super::*;

impl SqliteEngine {
    /// An in-memory registry for temporary storage, whose databases are in memory too,
    /// so no directory is created for them.
    pub(super) fn temporary() -> rusqlite::Result<Self> {
        Ok(SqliteEngine {
            in_memory: true,
            ..Self::memory()?
        })
    }
}
