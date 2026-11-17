// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/storage/client_storage.rs`.

#[allow(unused_imports)]
use super::*;

use std::fmt::Debug;
use std::path::PathBuf;
use std::str::FromStr;
use std::{fs, thread};
use log::warn;
use rusqlite::{Connection, OptionalExtension, Transaction};
use servo_base::generic_channel::{self, GenericReceiver, GenericSender};
use servo_base::id::{BrowsingContextId, WebViewId};
use servo_url::ImmutableOrigin;
use storage_traits::client_storage::{
    ClientStorageErrorr, ClientStorageThreadHandle, ClientStorageThreadMessage, Mode,
    StorageIdentifier, StorageProxyMap, StorageType,
};
use uuid::Uuid;

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
