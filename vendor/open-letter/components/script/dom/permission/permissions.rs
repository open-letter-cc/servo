// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/script/dom/permission/permissions.rs`.

#[allow(unused_imports)]
use super::*;

use dom_struct::dom_struct;
use embedder_traits::{self, AllowOrDeny, EmbedderMsg, PermissionFeature, WakeLockType};
use js::context::JSContext;
use js::conversions::ConversionResult;
use js::jsapi::JSObject;
use js::jsval::{ObjectValue, UndefinedValue};
use js::realm::CurrentRealm;
use script_bindings::inheritance::Castable;
use script_bindings::reflector::{Reflector, reflect_dom_object};
use servo_base::generic_channel;
use servo_base::generic_channel::GenericCallback;
use servo_config::pref;
use crate::conversions::Convert;
use crate::dom::RootedPromise;
use crate::dom::bindings::codegen::Bindings::PermissionStatusBinding::{
    PermissionDescriptor, PermissionName, PermissionState, PermissionStatusMethods,
};
use crate::dom::bindings::codegen::Bindings::PermissionsBinding::PermissionsMethods;
use crate::dom::bindings::codegen::Bindings::WindowBinding::Window_Binding::WindowMethods;
use crate::dom::bindings::error::Error;
use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::bindings::root::DomRoot;
use crate::dom::bluetooth::Bluetooth;
use crate::dom::bluetoothpermissionresult::BluetoothPermissionResult;
use crate::dom::globalscope::GlobalScope;
use crate::dom::permissionstatus::PermissionStatus;
use crate::dom::promise::Promise;
use crate::dom::window::Window;

/// The browsing context of the frame `global_scope` belongs to, reported to the embedder
/// with a permission request so that it can tell frames apart. `None` for a global with
/// no frame of its own, such as a worker.
pub(crate) fn requesting_browsing_context(
    global_scope: &GlobalScope,
) -> Option<servo_base::id::BrowsingContextId> {
    global_scope
        .downcast::<Window>()?
        .undiscarded_window_proxy()
        .map(|window_proxy| window_proxy.browsing_context_id())
}
