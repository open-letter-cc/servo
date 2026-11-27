// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `components/shared/net/lib.rs`.

#[allow(unused_imports)]
use super::*;

impl ResourceThreads {
    /// Serialize one kind of site state as JSON. See [`CoreResourceMsg::ExportSiteState`].
    pub fn export_site_state(&self, kind: SiteStateKind) -> Vec<u8> {
        let (sender, receiver) = generic_channel::channel().unwrap();
        let _ = self
            .core_thread
            .send(CoreResourceMsg::ExportSiteState(kind, sender));
        receiver.recv().unwrap_or_default()
    }

    /// Combine serialized site state with the state held. See
    /// [`CoreResourceMsg::ImportSiteState`].
    pub fn import_site_state(
        &self,
        kind: SiteStateKind,
        data: Vec<u8>,
        mode: SiteStateImportMode,
    ) -> Result<(), String> {
        let (sender, receiver) = generic_channel::channel().unwrap();
        let _ = self
            .core_thread
            .send(CoreResourceMsg::ImportSiteState(kind, data, mode, sender));
        receiver
            .recv()
            .unwrap_or_else(|_| Err("The resource thread is gone".to_owned()))
    }
}

/// A kind of persistent state a resource thread keeps for the sites it talks to, which an
/// embedder can export from one engine and import into another.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SiteStateKind {
    /// The cookie jar.
    Cookies,
    /// The HTTP Strict Transport Security list (not the preload list).
    Hsts,
    /// The HTTP authentication cache. It holds user names and passwords in the clear.
    AuthCache,
}

/// How imported site state is combined with the state already held.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SiteStateImportMode {
    /// Keep what is held and add what is imported. Where both have the same cookie
    /// (name, domain and path), HSTS host or authentication entry, the more recent one
    /// wins: the later last access for cookies, the later expiry for HSTS entries, and
    /// the imported one for authentication entries. Nothing held is removed.
    Merge,
    /// Discard what is held and keep only what is imported.
    Replace,
}
