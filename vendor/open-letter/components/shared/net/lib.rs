// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/shared/net/lib.rs`.

#[allow(unused_imports)]
use super::*;

use std::fmt::{self, Debug, Display};
use std::sync::{LazyLock, OnceLock};
use std::thread::{self, JoinHandle};
use bytes::Bytes;
use content_security_policy::{self as csp};
use cookie::Cookie;
use crossbeam_channel::{Receiver, Sender, unbounded};
use headers::{ContentType, HeaderMapExt, ReferrerPolicy as ReferrerPolicyHeader};
use http::{HeaderMap, HeaderValue, StatusCode, header};
use hyper_serde::Serde;
use hyper_util::client::legacy::Error as HyperError;
use malloc_size_of::malloc_size_of_is_0;
use malloc_size_of_derive::MallocSizeOf;
use mime::Mime;
use parking_lot::RwLock;
use pixels::CorsStatus;
use profile_traits::generic_callback::GenericCallback as ProfileGenericCallback;
use profile_traits::mem::ReportsChan;
use rand::{Rng, rng};
use request::RequestId;
use rustc_hash::FxHashMap;
use rustls::{CipherSuite, NamedGroup, ProtocolVersion};
use rustls_pki_types::CertificateDer;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};
use servo_base::generic_channel::{
    self, CallbackSetter, GenericCallback, GenericOneshotSender, GenericSend, GenericSender,
    SendResult,
};
use servo_base::id::{CookieStoreId, HistoryStateId, PipelineId};
use servo_url::{ImmutableOrigin, ServoUrl};
use uuid::Uuid;
use crate::response::RedirectTaint;
use crate::fetch::headers::determine_nosniff;
use crate::filemanager_thread::FileManagerThreadMsg;
use crate::http_status::HttpStatus;
use crate::mime_classifier::{ApacheBugFlag, MimeClassifier};
use crate::request::{Request, RequestBuilder};
use crate::response::{Response, ResponseInit};

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
