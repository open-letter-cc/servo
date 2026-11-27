// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

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

#[cfg(test)]
mod tests {
    use super::super::{ClientStorageThreadFactory, ClientStorageThreadHandle};

    /// Temporary storage with a config directory lives in memory: dropping its handle
    /// leaves no `clientstorage/temporary` directory and nothing under `clientstorage`.
    #[test]
    fn test_client_tmp_storage_with_config_dir_writes_nothing() {
        let tmp_config_dir = tempfile::tempdir().unwrap();
        let config_dir = tmp_config_dir.path();

        let handle = <ClientStorageThreadHandle as ClientStorageThreadFactory>::new(
            Some(config_dir.to_path_buf()),
            true,
        );
        drop(handle);

        let storage = config_dir.join("clientstorage");
        assert!(!storage.join("temporary").exists());
        assert!(
            !storage.exists() || std::fs::read_dir(&storage).unwrap().next().is_none()
        );
    }
}
