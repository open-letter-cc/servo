// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/constellation/constellation.rs`.

#[allow(unused_imports)]
use super::*;

use std::borrow::ToOwned;
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::marker::PhantomData;
use std::mem::replace;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::{process, thread};
use accesskit::{ActionRequest, TreeId};
use background_hang_monitor_api::{
    BackgroundHangMonitorControlMsg, BackgroundHangMonitorRegister, HangAlert,
};
use content_security_policy::sandboxing_directive::SandboxingFlagSet;
use crossbeam_channel::{Receiver, Select, Sender, unbounded};
use devtools_traits::{
    ChromeToDevtoolsControlMsg, DevtoolsControlMsg, DevtoolsPageInfo, NavigationState,
    ScriptToDevtoolsControlMsg,
};
use embedder_traits::resources::{self, Resource};
use embedder_traits::user_contents::{UserContentManagerId, UserContents};
use embedder_traits::{
    AnimationState, EmbedderControlId, EmbedderControlResponse, EmbedderProxy, FocusSequenceNumber,
    GenericEmbedderProxy, InputEvent, InputEventAndId, InputEventOutcome, JSValue,
    JavaScriptEvaluationError, JavaScriptEvaluationId, KeyboardEvent, MediaSessionActionType,
    MediaSessionEvent, MediaSessionPlaybackState, MouseButtonAction, MouseButtonEvent,
    NewWebViewDetails, PaintHitTestResult, Theme, ViewportDetails, WakeLockDelegate, WakeLockType,
    WebDriverCommandMsg, WebDriverLoadStatus, WebDriverScriptCommand,
};
use euclid::default::Size2D as UntypedSize2D;
use fonts::SystemFontServiceProxy;
use ipc_channel::router::ROUTER;
use keyboard_types::{Key, KeyState, Modifiers, NamedKey};
use layout_api::{LayoutFactory, ScriptThreadFactory};
use log::{debug, error, info, trace, warn};
use media::WindowGLContext;
use net::image_cache::ImageCacheFactoryImpl;
use net_traits::pub_domains::registered_domain_name;
use net_traits::{self, AsyncRuntime, FetchThread, ResourceThreads};
use paint_api::{
    PaintMessage, PaintProxy, PinchZoomInfos, PipelineExitSource, SendableFrameTree,
    WebRenderExternalImageIdManager,
};
use profile_traits::mem::ProfilerMsg;
use profile_traits::{mem, time};
use rand::rngs::SmallRng;
use rand::seq::IndexedRandom;
use rand::{RngExt, SeedableRng, make_rng};
use rustc_hash::{FxHashMap, FxHashSet};
use script_traits::{
    ConstellationInputEvent, DiscardBrowsingContext, DocumentActivity, MouseButtons,
    NewPipelineInfo, ScriptThreadMessage, UpdatePipelineIdReason,
};
use servo_background_hang_monitor::HangMonitorRegister;
use servo_base::generic_channel;
use servo_base::generic_channel::{
    GenericCallback, GenericSend, GenericSender, RoutedReceiver, SendError,
};
use servo_base::id::{
    BrowsingContextId, CONSTELLATION_PIPELINE_NAMESPACE_ID, FIRST_CONTENT_PIPELINE_NAMESPACE_ID,
    HistoryStateId, MessagePortId, MessagePortRouterId, PainterId, PipelineId, PipelineNamespace,
    PipelineNamespaceId, PipelineNamespaceRequest, ScriptEventLoopId, ServiceWorkerId, WebViewId,
    WorkerId,
};
use servo_base::threadboost::{BoostAffinity, ThreadPriority};
use servo_bluetooth_traits::BluetoothRequest;
use servo_canvas::canvas_paint_thread::CanvasPaintThread;
use servo_canvas_traits::ConstellationCanvasMsg;
use servo_canvas_traits::canvas::{CanvasId, CanvasMsg};
use servo_config::{opts, pref};
use servo_constellation_traits::{
    AuxiliaryWebViewCreationRequest, AuxiliaryWebViewCreationResponse, ConstellationInterest,
    EmbedderToConstellationMessage, HistoryTraversalSource, IFrameLoadInfo, IFrameLoadInfoWithData,
    IFrameSizeMsg, LoadData, LogEntry, MessagePortMsg, NavigationHistoryBehavior, PaintMetricEvent,
    PortMessageTask, PortTransferInfo, RemoteFocusOperation, SWManagerSenders,
    ScreenshotReadinessResponse, ScriptToConstellationMessage, ScrollStateUpdate,
    ServiceWorkerAlgorithm, ServiceWorkerManagerFactory, ServiceWorkerManagerMsg, ServiceWorkerMsg,
    SessionHistoryTraversalRequest, StructuredSerializedData, TargetSnapshotParams,
    TraversalDirection, UserContentManagerAction, WindowSizeType, WorkerAnimationFrameTick,
};
use servo_url::{Host, ImmutableOrigin, ServoUrl};
use storage_traits::StorageThreads;
use storage_traits::cache_storage::CacheStorageThreadMessage;
use storage_traits::client_storage::ClientStorageThreadMessage;
use storage_traits::indexeddb::{IndexedDBThreadMsg, SyncOperation};
use storage_traits::webstorage_thread::{WebStorageThreadMsg, WebStorageType};
use webgpu::canvas_context::WebGpuExternalImageMap;
use webgpu_traits::{WebGPU, WebGPURequest};
use super::super::embedder::ConstellationToEmbedderMsg;
use crate::broadcastchannel::BroadcastChannels;
use crate::browsingcontext::{
    AllBrowsingContextsIterator, BrowsingContext, FullyActiveBrowsingContextsIterator,
    NewBrowsingContextInfo,
};
use crate::constellation_webview::{ConstellationWebView, OngoingHistoryTraversalRequest};
use crate::event_loop::EventLoop;
use crate::pipeline::Pipeline;
use crate::serviceworker::ServiceWorkerUnprivilegedContent;
use crate::session_history::{NeedsToReload, SessionHistoryChange, SessionHistoryDiff};

impl<STF, SWF> Constellation<STF, SWF>
where
    STF: ScriptThreadFactory,
    SWF: ServiceWorkerManagerFactory,
{
    /// A content process's lifeline closed, so the process has exited. Reap it, then
    /// look at the pipelines its event loops were still serving.
    ///
    /// A pipeline that was never asked to close is one the process took down with it.
    /// That is handled exactly like a panic reported by the event loop itself: the
    /// embedder is told, every pipeline on that event loop is closed without waiting
    /// for an acknowledgement the process can no longer send, and the affected
    /// top-level documents are replaced by crash pages.
    ///
    /// A pipeline that *was* asked to close is in the middle of a normal exit, and its
    /// `PipelineExited` acknowledgement may still be queued behind this lifeline
    /// closure or may never arrive. It is removed here either way so the pipeline table
    /// drains, which is the condition for shutdown to complete.
    ///
    /// The lifeline is the only prompt signal of a content-process exit. The script
    /// channel is shared by every content process, so one dying never disconnects it,
    /// and a send to the dead event loop fails only when something next sends.
    #[cfg(feature = "multiprocess")]
    #[servo_tracing::instrument(skip_all)]
    pub(super) fn handle_process_exited(&mut self, index: usize) {
        let process_id = self.process_manager.pid_at(index);
        self.process_manager.remove(index);

        let mut crashed_event_loops = HashSet::new();
        let mut closing_pipelines = Vec::new();
        for pipeline in self.pipelines.values() {
            if pipeline.event_loop.process_id() != Some(process_id) {
                continue;
            }
            let is_active = self
                .browsing_contexts
                .get(&pipeline.browsing_context_id)
                .is_some_and(|browsing_context| browsing_context.pipelines.contains(&pipeline.id));
            let is_pending = self.webviews.get(&pipeline.webview_id).is_some_and(|webview| {
                webview
                    .pending_changes
                    .iter()
                    .any(|change| change.new_pipeline_id == pipeline.id)
            });
            if is_active || is_pending {
                crashed_event_loops.insert(pipeline.event_loop.id());
            } else {
                closing_pipelines.push(pipeline.id);
            }
        }

        for event_loop_id in crashed_event_loops {
            warn!("Content process {process_id} exited while serving {event_loop_id:?}");
            self.handle_panic(
                Some(event_loop_id),
                format!("Content process {process_id} exited"),
                None,
            );
        }
        for pipeline_id in closing_pipelines {
            self.handle_pipeline_exited(pipeline_id, PipelineExitSource::all());
        }
    }

    // Close and return the browsing context with the given id (and its children), if it exists.
    /// The parent of `browsing_context`: `Some(None)` for a top-level one, and `None`
    /// when its parent pipeline is gone, so that a nested frame is never reported as
    /// top-level.
    pub(super) fn parent_browsing_context_id(
        &self,
        browsing_context: &BrowsingContext,
    ) -> Option<Option<BrowsingContextId>> {
        match browsing_context.parent_pipeline_id {
            None => Some(None),
            Some(parent_pipeline_id) => match self.pipelines.get(&parent_pipeline_id) {
                Some(parent_pipeline) => Some(Some(parent_pipeline.browsing_context_id)),
                None => {
                    warn!("{}: Parent pipeline {parent_pipeline_id} is gone", browsing_context.id);
                    None
                },
            },
        }
    }

    /// Tells the embedder that `pipeline_id`'s document is now the active one in its
    /// frame, `browsing_context_id`.
    pub(super) fn notify_frame_navigated(
        &self,
        browsing_context_id: BrowsingContextId,
        pipeline_id: PipelineId,
    ) {
        let Some(browsing_context) = self.browsing_contexts.get(&browsing_context_id) else {
            return;
        };
        let Some(pipeline) = self.pipelines.get(&pipeline_id) else {
            return;
        };
        let Some(parent) = self.parent_browsing_context_id(browsing_context) else {
            return;
        };
        self.constellation_to_embedder_proxy
            .send(ConstellationToEmbedderMsg::FrameNavigated(
                browsing_context.webview_id,
                browsing_context_id,
                parent,
                pipeline.url.clone(),
                pipeline.event_loop.process_id(),
            ));
    }
}
