// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `components/storage/indexeddb/engines/sqlite.rs`.

#[allow(unused_imports)]
use super::*;

impl SqliteEngine {
    /// Opens the database for `path` in memory, for temporary storage, so that nothing
    /// it holds outlives the process.
    ///
    /// Transactions open their own connections by `db_path`, so it names a database in
    /// the `memdb` VFS: a name beginning with "/" is shared by every connection in this
    /// process, with ordinary locking, and lives as long as `connection` does. The
    /// registry hands out a directory name that is unique per database, and creates no
    /// directory for it.
    pub fn new_temporary(
        path: PathBuf,
        created: bool,
        db_info: &IndexedDBDescription,
        pool: Arc<ThreadPool>,
    ) -> Result<Self, Error> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| Error::InvalidPath(path.clone()))?;
        let db_path = PathBuf::from(format!("file:/{name}?vfs=memdb"));
        Self::open(db_path, created, db_info, pool)
    }

    pub(super) fn open(
        db_path: PathBuf,
        created: bool,
        db_info: &IndexedDBDescription,
        pool: Arc<ThreadPool>,
    ) -> Result<Self, Error> {
        let connection = Self::init_db(&db_path, db_info)?;

        for stmt in DB_PRAGMAS {
            // TODO: Handle errors properly
            let _ = connection.execute(stmt, ());
        }

        Ok(Self {
            connection,
            db_path,
            read_pool: pool.clone(),
            write_pool: pool,
            created_db_path: created,
        })
    }
}
