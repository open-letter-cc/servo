/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use embedder_traits::{
    InputEventOutcome, JSValue, JavaScriptEvaluationError, JavaScriptEvaluationId,
    MediaSessionEvent, NewWebViewDetails, TraversalId,
};
use servo_base::generic_channel::GenericSender;
use servo_base::id::{BrowsingContextId, PipelineId, WebViewId};
use servo_url::ServoUrl;

/// Messages sent from the `Constellation` to the embedder.
pub enum ConstellationToEmbedderMsg {
    /// Informs the embedder that the constellation has completed shutdown.
    /// Required because the constellation can have pending calls to make
    /// (e.g. SetFrameTree) at the time that we send it an ExitMsg.
    ShutdownComplete,
    /// A history traversal operation completed.
    HistoryTraversalComplete(WebViewId, TraversalId),
    /// Notifies the embedder about media session events
    /// (i.e. when there is metadata for the active media session, playback state changes...).
    MediaSessionEvent(WebViewId, MediaSessionEvent),
    /// A pipeline panicked. First string is the reason, second one is the backtrace.
    Panic(WebViewId, String, Option<String>),
    /// Inform the embedding layer that a particular `InputEvent` was handled by Servo
    /// and the embedder can continue processing it, if necessary.
    InputEventsHandled(WebViewId, Vec<InputEventOutcome>),
    /// A `WebView` was destroyed.
    WebViewClosed(WebViewId),
    /// Inform the embedding layer that a JavaScript evaluation has
    /// finished with the given result.
    FinishJavaScriptEvaluation(
        JavaScriptEvaluationId,
        Result<JSValue, JavaScriptEvaluationError>,
    ),
    /// Whether or not to allow script to open a new tab/browser
    AllowOpeningWebView(WebViewId, GenericSender<Option<NewWebViewDetails>>),
    /// Whether or not to allow a pipeline to load a url.
    AllowNavigationRequest(WebViewId, PipelineId, ServoUrl),
    /// The history state has changed.
    HistoryChanged(WebViewId, Vec<ServoUrl>, usize),
    /// A pipeline exited, whether cleanly or because its content process died.
    PipelineExited(PipelineId),
    /// A new document became active in a frame: the frame's browsing context, its
    /// parent's (`None` for a top-level frame), the document's URL, and the
    /// operating-system id of the content process the document runs in (`None` when it
    /// runs in this process).
    FrameNavigated(
        WebViewId,
        BrowsingContextId,
        Option<BrowsingContextId>,
        ServoUrl,
        Option<u32>,
    ),
    /// A frame was removed: its browsing context and its parent's. A frame's descendants
    /// are always reported removed before the frame itself.
    FrameRemoved(WebViewId, BrowsingContextId, Option<BrowsingContextId>),
}
