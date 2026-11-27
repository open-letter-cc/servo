// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `components/fonts/system_font_service_proxy.rs`.

use servo_base::generic_channel;
use fonts_traits::SystemFontServiceMessage;

use super::SystemFontServiceProxy;
use crate::{FontDataAndIndex, LocalFontIdentifier};

impl SystemFontServiceProxy {
    /// The bytes of a local font and the index of the face within them, fetched from
    /// the [`SystemFontService`] and cached per process. `None` if the font cannot be
    /// read, which is also cached, so an unreadable font is asked for once.
    pub fn get_local_font_data(
        &self,
        identifier: &LocalFontIdentifier,
    ) -> Option<FontDataAndIndex> {
        if let Some(data) = self.local_font_data.read().get(identifier) {
            return data.clone();
        }

        let (response_chan, response_port) =
            generic_channel::channel().expect("failed to create IPC channel");
        self.sender
            .send(SystemFontServiceMessage::GetFontData(
                identifier.clone(),
                response_chan,
            ))
            .expect("failed to send message to system font service");
        let Ok(data) = response_port.recv() else {
            let font_thread_has_closed = self.sender.send(SystemFontServiceMessage::Ping).is_err();
            assert!(
                font_thread_has_closed,
                "Failed to receive a response from live font cache"
            );
            panic!("SystemFontService has already exited.");
        };

        self.local_font_data
            .write()
            .entry(identifier.clone())
            .or_insert(data)
            .clone()
    }
}
