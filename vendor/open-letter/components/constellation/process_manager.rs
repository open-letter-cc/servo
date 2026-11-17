// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/constellation/process_manager.rs`.

#[allow(unused_imports)]
use super::*;

impl ProcessManager {
    pub fn pid_at(&self, index: usize) -> u32 {
        let (process, _) = &self.processes[index];
        process.pid()
    }
}
