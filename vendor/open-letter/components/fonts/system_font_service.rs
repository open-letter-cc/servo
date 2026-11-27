// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `components/fonts/system_font_service.rs`.

#[allow(unused_imports)]
use super::*;

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
