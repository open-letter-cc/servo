// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/servo/site_data_manager.rs`.

#[allow(unused_imports)]
use super::*;

impl SiteDataManager {
    /// Serialize one kind of persistent site state as JSON, in the form of its file in
    /// the config directory, so that an embedder can move it to another engine with
    /// [`Self::import_site_state`]. Only the public (non-private) state is exported.
    pub fn export_site_state(&self, kind: SiteStateKind) -> Vec<u8> {
        self.public_resource_threads.export_site_state(kind)
    }

    /// Combine site state serialized by [`Self::export_site_state`] with the public
    /// (non-private) state held. Returns an error, leaving the state as it was, if `data`
    /// cannot be parsed.
    pub fn import_site_state(
        &self,
        kind: SiteStateKind,
        data: Vec<u8>,
        mode: SiteStateImportMode,
    ) -> Result<(), String> {
        self.public_resource_threads
            .import_site_state(kind, data, mode)
    }
}
