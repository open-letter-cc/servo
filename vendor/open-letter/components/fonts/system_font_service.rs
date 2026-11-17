// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/fonts/system_font_service.rs`.

#[allow(unused_imports)]
use super::*;

use std::borrow::ToOwned;
use std::cell::OnceCell;
use std::collections::HashMap;
use std::thread;
use app_units::Au;
use fonts_traits::{
    FontData, FontDataAndIndex, FontDescriptor, FontIdentifier, FontTemplate, FontTemplateRef,
    LocalFontIdentifier, LowercaseFontFamilyName, SystemFontServiceMessage,
    SystemFontServiceProxySender,
};
use malloc_size_of::MallocSizeOf as MallocSizeOfTrait;
use malloc_size_of_derive::MallocSizeOf;
use paint_api::CrossProcessPaintApi;
use profile_traits::mem::{
    ProcessReports, ProfilerChan, Report, ReportKind, ReportsChan, perform_memory_report,
};
use profile_traits::path;
use rustc_hash::FxHashMap;
use servo_base::generic_channel::{self, GenericReceiver};
use servo_base::id::PainterId;
use servo_config::pref;
use style::values::computed::font::{GenericFontFamily, SingleFontFamily};
use webrender_api::{
    FontInstanceFlags, FontInstanceKey, FontInstancePlatformOptions, FontKey, FontVariation,
};
use crate::font_store::FontStore;
use crate::platform::font_list::{
    default_system_generic_font_family, for_each_available_family, for_each_variation,
};

impl SystemFontService {
    /// Read a local font's bytes, once, into shared memory. The mapping of the file
    /// is turned into a shared-memory segment here rather than at every send, since
    /// sending a memory-mapped [`FontData`] copies it each time.
    #[servo_tracing::instrument(skip_all)]
    pub(super) fn get_local_font_data(
        &mut self,
        identifier: LocalFontIdentifier,
    ) -> Option<FontDataAndIndex> {
        self.local_font_data
            .entry(identifier)
            .or_insert_with_key(|identifier| {
                identifier
                    .font_data_and_index()
                    .map(|FontDataAndIndex { data, index }| FontDataAndIndex {
                        data: FontData::SharedMemory(data.as_ipc_shared_memory()),
                        index,
                    })
            })
            .clone()
    }
}
