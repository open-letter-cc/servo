// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/servo/servo.rs`.

#[allow(unused_imports)]
use super::*;

use std::cell::{Cell, Ref, RefCell, RefMut};
use std::cmp::max;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;
use accesskit::ActionRequest;
use crossbeam_channel::{Receiver, Sender, unbounded};
use env_logger::Builder as EnvLoggerBuilder;
use fonts::SystemFontService;
use gaol::sandbox::{ChildSandbox, ChildSandboxMethods};
use ipc_channel::ipc::{self, IpcSender};
use layout::LayoutFactoryImpl;
use layout_api::ScriptThreadFactory;
use log::{Log, Metadata, Record, debug, warn};
use media::{GlApi, NativeDisplay, WindowGLContext};
use net::embedder::NetToEmbedderMsg;
use net::image_cache::ImageCacheFactoryImpl;
use net::protocols::ProtocolRegistry;
use net::resource_thread::new_resource_threads;
use net_traits::FetchThread;
use net_traits::ResourceThreads;
use paint::{InitialPaintState, Paint};
use paint_api::{CrossProcessPaintApi, PaintMessage, PaintProxy};
use profile::{mem as profile_mem, time as profile_time};
use profile_traits::mem::MemoryReportResult;
use profile_traits::mem::{ProfilerMsg, Reporter};
use profile_traits::{mem, time};
use rustc_hash::FxHashMap;
use script::JSEngineSetup;
use script::ServiceWorkerManager;
use servo_background_hang_monitor::HangMonitorRegister;
use servo_base::generic_channel::{GenericCallback, GenericSender, RoutedReceiver, SendError};
use servo_base::id::{EMBEDDER_PIPELINE_NAMESPACE_ID, PipelineId, PipelineNamespace};
use servo_bluetooth::BluetoothThreadFactory;
use servo_bluetooth_traits::BluetoothRequest;
use servo_config::opts::{DiagnosticsLoggingOption, Opts};
use servo_config::prefs::{PrefValue, Preferences};
use servo_config::{opts, pref, prefs};
use servo_constellation::UnprivilegedContent;
use servo_constellation::content_process_sandbox_profile;
use servo_constellation::{
    Constellation, ConstellationToEmbedderMsg, FromEmbedderLogger, InitialConstellationState,
};
use servo_constellation::{FromScriptLogger, NewScriptEventLoopProcessInfo};
use servo_constellation_traits::EmbedderToConstellationMessage;
use servo_constellation_traits::ScriptToConstellationSender;
use servo_geometry::{
    DeviceIndependentIntRect, convert_rect_to_css_pixel, convert_size_to_css_pixel,
};
use servo_media::ServoMedia;
use servo_media::player::context::GlContext;
use servo_wakelock::DefaultWakeLockDelegate;
use storage::new_storage_threads;
use storage_traits::StorageThreads;
use style::global_style_data::StyleThreadPool;
use webxr::WebXrRegistry;
use crate::clipboard_delegate::StringRequest;
use crate::gamepad_delegate::{GamepadHapticEffectRequest, GamepadHapticEffectRequestType};
use crate::geolocation_delegate::{PositionRequest, PositionWatch};
use crate::javascript_evaluator::JavaScriptEvaluator;
use crate::network_manager::NetworkManager;
use crate::proxies::ConstellationProxy;
use crate::responders::ServoErrorChannel;
use crate::servo_delegate::{DefaultServoDelegate, ServoDelegate, ServoError};
use crate::site_data_manager::{CookieOperationResponse, SiteDataManager};
use crate::webview::{MINIMUM_WEBVIEW_SIZE, WebView, WebViewInner};
use crate::webview_delegate::{
    AllowOrDenyRequest, AuthenticationRequest, BluetoothDeviceSelectionRequest, EmbedderControl,
    FilePicker, FrameEvent, FrameEventKind, NavigationRequest, PermissionRequest,
    ProtocolHandlerRegistration, WebResourceLoad,
};

impl Servo {
    pub(crate) fn embedder_proxy(&self) -> &EmbedderProxy {
        &self.0.embedder_proxy
    }
}
