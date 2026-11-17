// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/net/resource_thread.rs`.

#[allow(unused_imports)]
use super::*;

/// Combine `data`, serialized by `CoreResourceMsg::ExportSiteState`, with the site state
/// of `http_state`.
pub(super) fn import_site_state(
    http_state: &HttpState,
    kind: SiteStateKind,
    data: &[u8],
    mode: SiteStateImportMode,
) -> Result<(), String> {
    let error = |error: serde_json::Error| format!("Could not parse {kind:?}: {error}");
    match kind {
        SiteStateKind::Cookies => {
            let imported: CookieStorage = serde_json::from_slice(data).map_err(error)?;
            let mut jar = http_state.cookie_jar.write();
            match mode {
                SiteStateImportMode::Merge => jar.merge(imported),
                SiteStateImportMode::Replace => *jar = imported,
            }
        },
        SiteStateKind::Hsts => {
            let imported: HstsList = serde_json::from_slice(data).map_err(error)?;
            let mut list = http_state.hsts_list.write();
            match mode {
                SiteStateImportMode::Merge => list.merge(imported),
                SiteStateImportMode::Replace => *list = imported,
            }
        },
        SiteStateKind::AuthCache => {
            let imported: AuthCache = serde_json::from_slice(data).map_err(error)?;
            let mut cache = http_state.auth_cache.write();
            match mode {
                SiteStateImportMode::Merge => cache.entries.extend(imported.entries),
                SiteStateImportMode::Replace => *cache = imported,
            }
        },
    }
    Ok(())
}
