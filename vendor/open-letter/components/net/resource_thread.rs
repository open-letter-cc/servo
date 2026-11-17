// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/net/resource_thread.rs`.

#[allow(unused_imports)]
use super::*;

use std::borrow::ToOwned;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::thread;
use cookie::Cookie;
use embedder_traits::GenericEmbedderProxy;
use hyper_serde::Serde;
use log::{debug, trace, warn};
use malloc_size_of_derive::MallocSizeOf;
use net_traits::blob_url_store::{BlobTokenCommunicator, parse_blob_url};
use net_traits::filemanager_thread::FileTokenCheck;
use net_traits::pub_domains::public_suffix_list_size_of;
use net_traits::request::{Destination, PreloadEntry, PreloadId, RequestBuilder, RequestId};
use net_traits::response::{Response, ResponseInit};
use net_traits::{
    AsyncRuntime, CookieAsyncResponse, CookieData, CookieSource, CoreResourceMsg,
    CoreResourceThread, CustomResponseMediator, DiscardFetch, FetchChannels, FetchTaskTarget,
    NetworkError, ResourceFetchTiming, ResourceThreads, ResourceTimingType, SiteStateImportMode,
    SiteStateKind, WebSocketDomAction, WebSocketNetworkEvent,
};
use parking_lot::{Mutex, RwLock};
use profile_traits::generic_callback::GenericCallback as ProfileGenericCallback;
use profile_traits::mem::{
    ProcessReports, ProfilerChan as MemProfilerChan, Report, ReportKind, ReportsChan,
    perform_memory_report,
};
use profile_traits::path;
use profile_traits::time::ProfilerChan;
use rustc_hash::FxHashMap;
use rustls_pki_types::CertificateDer;
use rustls_pki_types::pem::PemObject;
use serde::{Deserialize, Serialize};
use servo_base::generic_channel::{
    self, CallbackSetter, GenericCallback, GenericReceiver, GenericReceiverSet,
    GenericSelectionResult, GenericSender,
};
use servo_base::id::CookieStoreId;
use servo_url::{ImmutableOrigin, ServoUrl};
use tokio::sync::Mutex as TokioMutex;
use crate::async_runtime::{init_async_runtime, spawn_task};
use crate::connector::{
    CACertificates, CertificateErrorOverrideManager, create_http_client, create_tls_config,
};
use crate::cookie::ServoCookie;
use crate::cookie_storage::CookieStorage;
use crate::embedder::NetToEmbedderMsg;
use crate::fetch::cors_cache::CorsCache;
use crate::fetch::fetch_params::{FetchParams, SharedPreloadedResources};
use crate::fetch::methods::{
    AutoRequestBodyStreamCloser, CancellationListener, FetchContext,
    SharedInflightKeepAliveRecords, WebSocketChannel, fetch,
    transfers_request_body_stream_to_later_manual_redirect,
};
use crate::filemanager_thread::FileManager;
use crate::hsts::{self, HstsList};
use crate::http_cache::{HttpCache, HttpCacheAssignment};
use crate::http_loader::{HttpState, http_redirect_fetch};
use crate::protocols::ProtocolRegistry;
use crate::request_interceptor::RequestInterceptor;
use crate::websocket_loader::create_handshake_request;

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
