// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/constellation/event_loop.rs`.

#[allow(unused_imports)]
use super::*;

use std::marker::PhantomData;
use std::rc::Rc;
use background_hang_monitor_api::{BackgroundHangMonitorControlMsg, HangAlert};
use embedder_traits::ScriptToEmbedderChan;
use layout_api::ScriptThreadFactory;
use log::error;
use media::WindowGLContext;
use script_traits::{InitialScriptState, ScriptThreadMessage};
use serde::{Deserialize, Serialize};
use servo_base::generic_channel::{GenericReceiver, GenericSender, SendError};
use servo_base::id::ScriptEventLoopId;
use servo_config::opts::{self, Opts};
use servo_config::prefs::{self, Preferences};
use servo_constellation_traits::ServiceWorkerManagerFactory;
use crate::Constellation;

impl EventLoop {
    /// The operating-system id of the content process this event loop runs in, or
    /// `None` when it runs in this process.
    pub(crate) fn process_id(&self) -> Option<u32> {
        self.process_id
    }
}
