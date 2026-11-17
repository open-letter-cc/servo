// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/servo/webview_delegate.rs`.

#[allow(unused_imports)]
use super::*;

impl PermissionRequest {
    /// Get the origin of the global that requested the permission. This is supplied by
    /// the engine, and is the origin of a nested frame when one is the requester, which
    /// can differ from the origin of the [`WebView`]'s top-level document.
    pub fn requester_origin(&self) -> &ImmutableOrigin {
        &self.requester_origin
    }

    /// Get the browsing context of the frame that requested the permission, supplied by
    /// the engine. Unlike an origin, this tells frames apart even when their origins are
    /// equal or opaque. `None` when the requester has no frame of its own, as for a worker.
    pub fn requester_browsing_context_id(&self) -> Option<BrowsingContextId> {
        self.requester_browsing_context_id
    }
}

/// What happened to a frame, in [`FrameEvent`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameEventKind {
    /// The frame was created. Delivered before anything the frame's own script sends.
    Created,
    /// A new document became active in the frame, including its first document and a
    /// navigation that replaced the current history entry.
    Navigated,
    /// The frame was removed. A frame's descendants are reported removed before it.
    Removed,
}

/// A change to one frame of a [`WebView`], as passed to
/// [`WebViewDelegate::notify_frame_changed`].
#[derive(Clone, Debug)]
pub struct FrameEvent {
    /// The frame's browsing context, which keeps its identity across navigations in it.
    pub browsing_context_id: BrowsingContextId,
    /// The parent frame's browsing context, `None` for a top-level frame. Given on every
    /// event.
    pub parent_browsing_context_id: Option<BrowsingContextId>,
    pub kind: FrameEventKind,
    /// The frame's URL for [`FrameEventKind::Created`] and [`FrameEventKind::Navigated`].
    pub url: Option<Url>,
    /// For [`FrameEventKind::Navigated`], the operating-system process id of the content
    /// process the newly active document runs in. `None` for the other kinds, and when
    /// content runs in the embedder's own process. Several frames, and several
    /// `WebView`s, can share one content process.
    pub content_process_id: Option<u32>,
}

impl EmbedderControl {
    /// Generic embedder control extension, no domain logic.
    ///
    /// The [`EmbedderControlTag`] for this control, or `None` if this kind of control has no
    /// tag assigned and therefore always follows Servo's default path.
    pub fn tag(&self) -> Option<EmbedderControlTag> {
        match self {
            EmbedderControl::ContextMenu(..) => Some(EmbedderControlTag::ContextMenu),
            EmbedderControl::SelectElement(..) => Some(EmbedderControlTag::Select),
            EmbedderControl::FilePicker(..) => Some(EmbedderControlTag::FilePicker),
            EmbedderControl::PageRequest(..) => Some(EmbedderControlTag::PageRequest),
            EmbedderControl::ColorPicker(..)
            | EmbedderControl::InputMethod(..)
            | EmbedderControl::SimpleDialog(..) => None,
        }
    }
}

/// Generic embedder control extension, no domain logic.
///
/// A stable tag for the kinds of [`EmbedderControl`] an embedder may choose to take
/// ownership of, plus tags reserved for controls that the embedder itself originates.
/// Servo attaches no meaning to a tag beyond routing: it never inspects the origin of a
/// control and never parses its payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum EmbedderControlTag {
    /// A context menu opened on web content.
    ContextMenu = 1,
    /// The picker of a `<select>` element.
    Select = 2,
    /// The picker of an `<input type=file>` element.
    FilePicker = 3,
    /// Opaque bytes sent by page script, answered with opaque bytes. See [`PageRequest`].
    PageRequest = 4,
    /// Reserved for embedder-originated requests. Servo never emits this tag.
    HostRequest = 5,
}

impl EmbedderControlTag {
    /// The bit this tag occupies in an [`WebViewDelegate::embedder_control_flags`] mask.
    pub fn bit(self) -> u64 {
        1 << (self as u32 - 1)
    }
}

/// Take no control; every [`EmbedderControl`] follows Servo's default path.
pub const EMBEDDER_CONTROL_NONE: u64 = 0;

/// See [`EmbedderControlTag::ContextMenu`].
pub const EMBEDDER_CONTROL_CONTEXT_MENU: u64 = 1 << 0;

/// See [`EmbedderControlTag::Select`].
pub const EMBEDDER_CONTROL_SELECT: u64 = 1 << 1;

/// See [`EmbedderControlTag::FilePicker`].
pub const EMBEDDER_CONTROL_FILE_PICKER: u64 = 1 << 2;

/// See [`EmbedderControlTag::PageRequest`].
pub const EMBEDDER_CONTROL_PAGE_REQUEST: u64 = 1 << 3;

/// See [`EmbedderControlTag::HostRequest`].
pub const EMBEDDER_CONTROL_HOST_REQUEST: u64 = 1 << 4;

/// Take every taggable control, including bits not yet assigned.
pub const EMBEDDER_CONTROL_ALL: u64 = 0xFFFF_FFFF;

/// Generic embedder control extension, no domain logic.
///
/// Opaque bytes that page script passed to the page-request function, waiting for an
/// answer. Servo never parses the bytes and applies no policy: which requests are
/// answered, and with what, is the embedder's decision.
///
/// Unlike the other controls, dropping this one unanswered **rejects** the page's
/// promise, since there is no default answer to give on the embedder's behalf.
pub struct PageRequest {
    pub(crate) id: EmbedderControlId,
    pub(crate) bytes: Vec<u8>,
    pub(crate) response_sent: bool,
    pub(crate) constellation_proxy: ConstellationProxy,
}

impl PageRequest {
    /// Return the [`EmbedderControlId`] associated with this request.
    pub fn id(&self) -> EmbedderControlId {
        self.id
    }

    /// The bytes page script passed, exactly as passed.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Resolve the page's promise with `bytes`, which may be empty.
    pub fn respond(mut self, bytes: Vec<u8>) {
        self.send(Some(bytes));
    }

    /// Reject the page's promise.
    pub fn reject(mut self) {
        self.send(None);
    }

    fn send(&mut self, response: Option<Vec<u8>>) {
        self.constellation_proxy
            .send(EmbedderToConstellationMessage::EmbedderControlResponse(
                self.id,
                EmbedderControlResponse::PageRequest(response),
            ));
        self.response_sent = true;
    }
}

impl Drop for PageRequest {
    fn drop(&mut self) {
        if !self.response_sent {
            self.send(None);
        }
    }
}

impl CreateNewWebViewRequest {
    /// The [`Servo`] instance this request belongs to.
    ///
    /// [`Self::builder`] consumes the request, so an embedder that needs the instance
    /// while building — to construct a [`UserContentManager`](crate::UserContentManager),
    /// for example — takes it from here first.
    pub fn servo(&self) -> &Servo {
        &self.servo
    }
}
