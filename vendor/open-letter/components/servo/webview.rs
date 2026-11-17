// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/servo/webview.rs`.

#[allow(unused_imports)]
use super::*;

use std::cell::{Cell, Ref, RefCell, RefMut};
use std::hash::Hash;
use std::rc::{Rc, Weak};
use accesskit::{
    Affine as AccesskitAffine, Node as AccesskitNode, NodeId, Rect as AccesskitRect, Role, Tree,
    TreeId, TreeUpdate, Uuid as AccesskitUuid,
};
use dpi::PhysicalSize;
use embedder_traits::{
    ContextMenuAction, ContextMenuItem, Cursor, EmbedderControlId, EmbedderControlRequest,
    EmbedderMsg, Image,
    InputEvent, InputEventAndId, InputEventId, JSValue, JavaScriptEvaluationError, LoadStatus,
    MediaSessionActionType, NewWebViewDetails, ScreenGeometry, ScreenshotCaptureError, Scroll,
    Theme, TraversalId, UrlRequest, ViewportDetails, WebViewPoint, WebViewRect,
};
use euclid::{Scale, Size2D};
use image::RgbaImage;
use log::debug;
use paint_api::WebViewTrait;
use paint_api::rendering_context::RenderingContext;
use servo_base::Epoch;
use servo_base::generic_channel::GenericSender;
use servo_base::id::WebViewId;
use servo_config::pref;
use servo_constellation_traits::{
    EmbedderToConstellationMessage, HistoryTraversalSource, SessionHistoryTraversalRequest,
    TraversalDirection,
};
use servo_geometry::DeviceIndependentPixel;
use servo_url::{ImmutableOrigin, ServoUrl};
use style_traits::CSSPixel;
use url::Url;
use webrender_api::units::{DeviceIntRect, DevicePixel, DevicePoint, DeviceSize};
use crate::clipboard_delegate::{ClipboardDelegate, DefaultClipboardDelegate};
use crate::gamepad_delegate::{DefaultGamepadDelegate, GamepadDelegate};
use crate::geolocation_delegate::{DefaultGeolocationDelegate, GeolocationDelegate};
use crate::responders::AutomaticResponder;
use crate::servo::PendingHandledInputEvent;
use crate::webview_delegate::{CreateNewWebViewRequest, DefaultWebViewDelegate, WebViewDelegate};
use crate::{
    ColorPicker, ContextMenu, EmbedderControl, InputMethodControl, PageRequest, SelectElement,
    Servo, UserContentManager, WebRenderDebugOption,
};

impl WebView {
    /// Generic embedder control extension, no domain logic.
    ///
    /// Route `embedder_control` to the delegate. A control whose
    /// [`EmbedderControlTag`](crate::webview_delegate::EmbedderControlTag) bit is set in
    /// [`WebViewDelegate::embedder_control_flags`] is offered to
    /// [`WebViewDelegate::handle_embedder_control`] first; everything else, and anything the
    /// delegate declines, follows the ordinary
    /// [`WebViewDelegate::show_embedder_control`] path.
    ///
    /// `requester_origin` is the origin of the document that asked for the control, which
    /// may be a nested frame's. `None` only for a control with no tag, which is never
    /// offered to the delegate's handler.
    pub(crate) fn dispatch_embedder_control(
        &self,
        embedder_control: EmbedderControl,
        requester_origin: Option<&ImmutableOrigin>,
    ) {
        let delegate = self.delegate();

        let taken_by_embedder = embedder_control
            .tag()
            .is_some_and(|tag| delegate.embedder_control_flags() & tag.bit() != 0);
        // A page request has no user interface to show, so one that is not taken, or that
        // the embedder declines, is dropped here, which rejects the page's promise.
        let show = |embedder_control| {
            if !matches!(embedder_control, EmbedderControl::PageRequest(..)) {
                delegate.show_embedder_control(self.clone(), embedder_control);
            }
        };

        if !taken_by_embedder {
            show(embedder_control);
            return;
        }

        // The embedder returns the control to us if it declined to handle it.
        if let Some(embedder_control) =
            delegate.handle_embedder_control(self.clone(), embedder_control, requester_origin)
        {
            show(embedder_control);
        }
    }
}
