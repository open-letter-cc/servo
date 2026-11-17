// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/script/runtime/script_runtime.rs`.

#[allow(unused_imports)]
use super::*;

use std::cell::{Cell, LazyCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString};
use std::io::{Write, stdout};
use std::ops::{Deref, DerefMut};
use std::os::raw::c_void;
use std::ptr::NonNull;
use std::rc::Weak;
use std::time::Instant;
use std::{os, ptr};
use background_hang_monitor_api::ScriptHangAnnotation;
use js::context::JSContext;
use js::conversions::jsstr_to_string;
use js::gc::StackGCVector;
use js::glue::{
    DispatchablePointer, RUST_js_GetErrorMessage, RegisterScriptEnvironmentPreparer,
    RunScriptEnvironmentPreparerClosure, SetBuildId, StreamConsumerConsumeChunk,
    StreamConsumerNoteResponseURLs, StreamConsumerStreamEnd, StreamConsumerStreamError,
};
use js::jsapi::{
    AsmJSOption, BuildIdCharVector, CompilationType, Dispatchable_MaybeShuttingDown, GCDescription,
    GCOptions, GCProgress, GCReason, Handle as RawHandle, HandleObject, HandleString,
    HandleValue as RawHandleValue, Heap, JSContext as RawJSContext, JSGCParamKey, JSGCStatus,
    JSJitCompilerOption, JSObject, JSSecurityCallbacks, JSString, JSTracer, MimeType,
    MutableHandleString, PromiseRejectionHandlingState, RuntimeCode,
    ScriptEnvironmentPreparer_Closure, SetProcessBuildIdOp, StreamConsumer as JSStreamConsumer,
};
use js::jsval::{JSVal, UndefinedValue};
use js::panic::wrap_panic;
use js::realm::CurrentRealm;
use js::rust::wrappers2::{
    CollectServoSizes, ContextOptionsRef, DispatchableRun, InitConsumeStreamCallback,
    JS_AddExtraGCRootsTracer, JS_GetPromiseResult, JS_InitDestroyPrincipalsCallback,
    JS_InitReadPrincipalsCallback, JS_NewStringCopyUTF8N, JS_SetGCCallback, JS_SetGCParameter,
    JS_SetGlobalJitCompilerOption, JS_SetNativeStackQuota, JS_SetOffthreadIonCompilationEnabled,
    JS_SetSecurityCallbacks, SetDOMCallbacks, SetGCSliceCallback, SetPreserveWrapperCallbacks,
    SetPromiseRejectionTrackerCallback, SetUpEventLoopDispatch,
};
use js::rust::{
    Handle, HandleObject as RustHandleObject, HandleValue, IntoHandle, ParentRuntime,
    Runtime as RustRuntime, Trace,
};
use malloc_size_of::MallocSizeOfOps;
use malloc_size_of_derive::MallocSizeOf;
use profile_traits::mem::{Report, ReportKind};
use profile_traits::path;
use profile_traits::time::ProfilerCategory;
use script_bindings::reflector::DomObject;
use script_bindings::script_runtime::{mark_runtime_dead, runtime_is_alive, temp_cx};
use script_bindings::settings_stack::run_a_script;
use servo_config::opts::{self, DiagnosticsLoggingOption};
use servo_config::pref;
use servo_url::ServoUrl;
use style::thread_state::{self, ThreadState};
use crate::dom::bindings::codegen::Bindings::ResponseBinding::Response_Binding::ResponseMethods;
use crate::dom::bindings::codegen::Bindings::ResponseBinding::ResponseType as DOMResponseType;
use crate::dom::bindings::codegen::UnionTypes::TrustedScriptOrString;
use crate::dom::bindings::conversions::{
    get_dom_class, private_from_object, root_from_handleobject, root_from_object,
};
use crate::dom::bindings::error::{Error, report_pending_exception, throw_dom_exception};
use crate::dom::bindings::inheritance::Castable;
use crate::dom::bindings::refcounted::{
    LivePromiseReferences, Trusted, TrustedPromise, trace_refcounted_objects,
};
use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::bindings::root::trace_roots;
use crate::dom::bindings::str::DOMString;
use crate::dom::bindings::utils::DOM_CALLBACKS;
use crate::dom::bindings::{principals, settings_stack};
use crate::dom::console::{Console, stringify_handle_value};
use crate::dom::csp::CspReporting;
use crate::dom::event::{Event, EventBubbles, EventCancelable};
use crate::dom::eventtarget::EventTarget;
use crate::dom::globalscope::GlobalScope;
use crate::dom::promise::Promise;
use crate::dom::promiserejectionevent::PromiseRejectionEvent;
use crate::dom::response::Response;
use crate::dom::trustedtypes::trustedscript::TrustedScript;
use crate::dom::window::Window;
use crate::engine::handle::current_js_engine_handle;
use crate::messaging::{CommonScriptMsg, ScriptEventLoopSender};
use crate::realms::enter_auto_realm;
use crate::runtime::job_queue::{JobQueue, job_queue_clear};
use crate::scripting::script_module::EnsureModuleHooksInitialized;
use crate::tasks::task_source::TaskSourceName;
use crate::{DomTypeHolder, ScriptThread};

/// The kinds of thread that run JavaScript, each with its own stack size preference.
#[derive(Clone, Copy, Debug)]
pub(crate) enum JSThreadKind {
    /// A script thread, which runs the documents of an event loop.
    Script,
    /// A worker or worklet thread.
    Worker,
}

/// The smallest stack, in KiB, that a JavaScript thread is given.
pub(super) const MIN_JS_THREAD_STACK_SIZE_KB: i64 = 1024;

/// Stack kept beyond the untrusted-script limit for trusted and system code, as mozjs
/// keeps by default.
pub(super) const SYSTEM_CODE_STACK_BUFFER: usize = 10 * 1024;

pub(super) const TRUSTED_SCRIPT_STACK_BUFFER: usize = 8 * 12800;

impl JSThreadKind {
    /// The stack size, in bytes, to spawn a thread of this kind with, from preferences.
    pub(crate) fn stack_size(self) -> usize {
        let kilobytes = match self {
            JSThreadKind::Script => pref!(thread_script_stack_size_kb),
            JSThreadKind::Worker => pref!(thread_worker_stack_size_kb),
        };
        kilobytes.max(MIN_JS_THREAD_STACK_SIZE_KB) as usize * 1024
    }
}

/// Record, on a newly spawned JavaScript thread, the stack size it was spawned with, so
/// that the runtime created on it limits script to part of that stack. Call this first
/// thing on the thread, before creating its [`Runtime`].
pub(crate) fn set_js_thread_stack_size(stack_size: usize) {
    JS_THREAD_STACK_SIZE.set(Some(stack_size));
}

/// The native stack quota for script on a thread with a stack of `stack_size` bytes: half
/// of it. The other half is kept for the engine frames that run on the same thread below
/// or between script frames: layout when it runs on the script thread, garbage
/// collection, and native functions called from script.
pub(super) fn native_stack_quota(stack_size: usize) -> usize {
    stack_size / 2
}
