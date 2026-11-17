// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/constellation/process_manager.rs`.

#[allow(unused_imports)]
use super::*;

use std::process::Child;
use crossbeam_channel::{Receiver, Select};
use log::{debug, warn};
use profile_traits::mem::{ProfilerChan, ProfilerMsg};
use servo_base::generic_channel::SendError;

impl ProcessManager {
    pub fn pid_at(&self, index: usize) -> u32 {
        let (process, _) = &self.processes[index];
        process.pid()
    }
}
