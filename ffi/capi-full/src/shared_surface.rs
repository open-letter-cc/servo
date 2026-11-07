// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! A rendering context that draws into a Direct3D 11 texture another process can
//! open, with no window involved.
//!
//! The texture is created by this crate on ANGLE's own D3D11 device with
//! `D3D11_RESOURCE_MISC_SHARED_NTHANDLE | D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX`,
//! wrapped as surfman's render target through `EGL_D3D_TEXTURE_ANGLE`, and exported
//! as an NT handle created with no name and placed only into the process the
//! embedder names. Nothing here inspects or decides anything about what is drawn.
//!
//! # Hand-off
//!
//! The texture's keyed mutex alternates between two keys:
//!
//! - The engine holds the mutex while it draws. `servo_rendering_context_present`
//!   releases it with key 1: a finished frame is ready.
//! - The consumer acquires key 1, reads or copies the texture, and releases with key 0.
//! - Before it draws again, the engine reclaims the mutex: with key 1 if the
//!   consumer never took the last frame (that frame is then dropped in favour of
//!   the new one), otherwise with key 0, waiting for the consumer to release it.
//!
//! So a consumer must release promptly: while it holds the mutex the engine cannot
//! draw the next frame.
//!
//! # Consumer textures
//!
//! For more than one consumer, or a consumer that must never stall the engine,
//! [`servo_rendering_context_shared_add_consumer`] gives a process a texture of its
//! own, with its own keyed mutex and its own handle. On every present the engine
//! copies the finished frame into each consumer texture on the GPU and releases it
//! with key 1, so the consumer runs exactly the protocol above on its own texture.
//! The engine never waits on a consumer texture: one the consumer still holds is
//! skipped for that frame, and one whose last frame was never taken is overwritten
//! with the newer frame.
//!
//! The primary texture keeps its blocking hand-off. An embedder that wants no
//! consumer to be able to stall drawing puts every consumer on a consumer texture
//! and never opens the primary; it still calls `servo_rendering_context_present`,
//! which is what publishes to the consumer textures.

use crate::rendering_context::RenderingContext;

/// What a consumer needs to open and read a shared surface, filled in by
/// [`servo_rendering_context_shared_info`].
///
/// Only ever reached through a pointer, so it may grow by appending fields without
/// an ABI version bump.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ServoSharedSurfaceInfo {
    /// The NT handle to the texture, valid only in the process named when the
    /// surface was created. It is never valid in the process that created it.
    pub handle: u64,
    /// The texture's width in pixels.
    pub width: u32,
    /// The texture's height in pixels.
    pub height: u32,
    /// The texture's `DXGI_FORMAT`. Always `DXGI_FORMAT_R8G8B8A8_UNORM` (28) for now,
    /// with premultiplied alpha.
    pub dxgi_format: u32,
    /// `SERVO_SHARED_SURFACE_FLAG_*` bits describing the texture's contents.
    pub flags: u32,
    /// The number of frames presented on this surface so far.
    pub frame: u64,
    /// The LUID of the adapter the texture lives on, as `HighPart << 32 | LowPart`.
    /// The consumer must open the handle on a device created on this adapter.
    pub adapter_luid: u64,
    /// Starts at 1 and increases each time the surface is recreated, which happens
    /// on every resize. A new generation comes with a new `handle`.
    pub generation: u64,
}

const _: () = assert!(
    size_of::<ServoSharedSurfaceInfo>() == 48,
    "ServoSharedSurfaceInfo must stay 48 bytes wide"
);

/// The texture's rows run bottom to top: row 0 is the bottom of the page. This is
/// OpenGL's orientation, which the engine draws in; a consumer drawing the texture
/// on a quad flips its vertical texture coordinate. Always set for now.
pub const SERVO_SHARED_SURFACE_FLAG_BOTTOM_UP: u32 = 1 << 0;

/// Returned by [`servo_rendering_context_shared_info`] on success.
pub const SERVO_SHARED_SURFACE_OK: i32 = 0;
/// Returned when `context` is not a shared-surface context or the out-pointer is
/// `NULL`.
pub const SERVO_SHARED_SURFACE_NOT_SHARED: i32 = -1;
/// Returned by [`servo_rendering_context_shared_add_consumer`] when the process
/// cannot be opened or its texture cannot be created or shared with it.
pub const SERVO_SHARED_SURFACE_CONSUMER_FAILED: i32 = -2;
/// Returned by [`servo_rendering_context_shared_add_consumer`] when the process
/// already has a consumer texture on this context.
pub const SERVO_SHARED_SURFACE_CONSUMER_EXISTS: i32 = -3;
/// Returned when the process has no consumer texture on this context.
pub const SERVO_SHARED_SURFACE_NO_CONSUMER: i32 = -4;

/// Creates a rendering context that draws into an off-screen Direct3D 11 texture,
/// shared as an NT handle with the process `target_process_id` and with no other.
///
/// `width` and `height` are the surface size in physical pixels. Both must be
/// non-zero. They need not relate to any window; the surface can be larger than
/// whatever shows it.
///
/// The handle is created with no name, with read access only, and is placed in the
/// target process with `DuplicateHandle`; this process keeps no copy of it. Read
/// it with [`servo_rendering_context_shared_info`]. The target process owns that
/// handle and is responsible for closing it, including the ones superseded by a
/// resize.
///
/// Draw as with any other context: `servo_rendering_context_make_current`, then
/// `servo_webview_paint`, then `servo_rendering_context_present`, which publishes
/// the frame. See the module documentation for the keyed-mutex protocol the
/// consumer follows.
///
/// Returns a newly allocated `RenderingContext` handle, or `NULL` on failure,
/// including on targets other than Windows. The ownership of the returned handle is
/// transferred to the caller, who must free it with `servo_rendering_context_free`
/// or consume it by passing it to `servo_webview_builder_create`.
///
/// # Safety
///
/// The call is made from the thread that will drive the event loop.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_rendering_context_create_shared(
    width: u32,
    height: u32,
    target_process_id: u32,
) -> *mut RenderingContext {
    if width == 0 || height == 0 {
        log::error!("servo_rendering_context_create_shared: size must be non-zero");
        return std::ptr::null_mut();
    }
    // Nothing below may unwind into the caller's C frame; surfman asserts on some
    // failures.
    let created = std::panic::catch_unwind(|| platform::create(width, height, target_process_id));
    match created {
        Ok(Ok(inner)) => Box::into_raw(Box::new(RenderingContext::new(inner, false))),
        Ok(Err(error)) => {
            log::error!("servo_rendering_context_create_shared failed: {error}");
            std::ptr::null_mut()
        },
        Err(_) => {
            log::error!("servo_rendering_context_create_shared panicked");
            std::ptr::null_mut()
        },
    }
}

/// Fills `info` with what a consumer needs to open `context`'s current surface.
///
/// Call it after creating the context and again after every resize, since a
/// resize replaces the texture and its handle; `generation` tells the two apart.
///
/// Returns [`SERVO_SHARED_SURFACE_OK`], or [`SERVO_SHARED_SURFACE_NOT_SHARED`] if
/// `context` was not created by [`servo_rendering_context_create_shared`] or `info`
/// is `NULL`, in which case `info` is left untouched.
///
/// # Safety
///
/// - `context` is a non-null pointer to a `RenderingContext` that has not been freed.
/// - `info` is either `NULL` or valid for a write of a `ServoSharedSurfaceInfo`.
/// - The call is made from the thread that created `context`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_rendering_context_shared_info(
    context: *mut RenderingContext,
    info: *mut ServoSharedSurfaceInfo,
) -> i32 {
    assert!(!context.is_null(), "context pointer must not be null");
    if info.is_null() {
        return SERVO_SHARED_SURFACE_NOT_SHARED;
    }
    // SAFETY: the caller upholds the requirements documented above.
    let inner = unsafe { &(*context).inner };
    match platform::info(inner) {
        Some(found) => {
            // SAFETY: `info` is non-null and valid for writes, as documented above.
            unsafe { info.write(found) };
            SERVO_SHARED_SURFACE_OK
        },
        None => SERVO_SHARED_SURFACE_NOT_SHARED,
    }
}

/// Gives the process `target_process_id` a texture of its own that receives every
/// frame `context` presents, and fills `info` with what it needs to open it.
///
/// The texture has the primary's size and format and its own keyed mutex. Its
/// handle is created with no name and read access only, and is placed in that
/// process alone with `DuplicateHandle`; this process keeps no copy. The target
/// process owns the handle and closes it, including the ones a resize supersedes.
/// See the module documentation for how frames reach the texture; the first one
/// arrives with the next present.
///
/// Returns [`SERVO_SHARED_SURFACE_OK`], [`SERVO_SHARED_SURFACE_NOT_SHARED`],
/// [`SERVO_SHARED_SURFACE_CONSUMER_EXISTS`] if that process already has one, or
/// [`SERVO_SHARED_SURFACE_CONSUMER_FAILED`]. `info` is written only on success.
///
/// # Safety
///
/// - `context` is a non-null pointer to a `RenderingContext` that has not been freed.
/// - `info` is either `NULL` or valid for a write of a `ServoSharedSurfaceInfo`.
/// - The call is made from the thread that created `context`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_rendering_context_shared_add_consumer(
    context: *mut RenderingContext,
    target_process_id: u32,
    info: *mut ServoSharedSurfaceInfo,
) -> i32 {
    assert!(!context.is_null(), "context pointer must not be null");
    if info.is_null() {
        return SERVO_SHARED_SURFACE_NOT_SHARED;
    }
    // SAFETY: the caller upholds the requirements documented above.
    let inner = unsafe { &(*context).inner };
    let added = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        platform::add_consumer(inner, target_process_id)
    }));
    match added {
        Ok(Ok(found)) => {
            // SAFETY: `info` is non-null and valid for writes, as documented above.
            unsafe { info.write(found) };
            SERVO_SHARED_SURFACE_OK
        },
        Ok(Err(status)) => status,
        Err(_) => {
            log::error!("servo_rendering_context_shared_add_consumer panicked");
            SERVO_SHARED_SURFACE_CONSUMER_FAILED
        },
    }
}

/// Fills `info` for the consumer texture of process `target_process_id`.
///
/// Call it after every resize, which gives each consumer a new texture and handle
/// at the new size; `generation` tells them apart.
///
/// Returns [`SERVO_SHARED_SURFACE_OK`], [`SERVO_SHARED_SURFACE_NOT_SHARED`], or
/// [`SERVO_SHARED_SURFACE_NO_CONSUMER`]. `info` is written only on success.
///
/// # Safety
///
/// As for [`servo_rendering_context_shared_add_consumer`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_rendering_context_shared_consumer_info(
    context: *mut RenderingContext,
    target_process_id: u32,
    info: *mut ServoSharedSurfaceInfo,
) -> i32 {
    assert!(!context.is_null(), "context pointer must not be null");
    if info.is_null() {
        return SERVO_SHARED_SURFACE_NOT_SHARED;
    }
    // SAFETY: the caller upholds the requirements documented above.
    let inner = unsafe { &(*context).inner };
    match platform::consumer_info(inner, target_process_id) {
        Ok(found) => {
            // SAFETY: `info` is non-null and valid for writes, as documented above.
            unsafe { info.write(found) };
            SERVO_SHARED_SURFACE_OK
        },
        Err(status) => status,
    }
}

/// Stops sending frames to the consumer texture of process `target_process_id` and
/// releases this process's reference to it. The handle in the target process stays
/// valid until that process closes it; the texture simply stops changing.
///
/// Returns [`SERVO_SHARED_SURFACE_OK`], [`SERVO_SHARED_SURFACE_NOT_SHARED`], or
/// [`SERVO_SHARED_SURFACE_NO_CONSUMER`].
///
/// # Safety
///
/// - `context` is a non-null pointer to a `RenderingContext` that has not been freed.
/// - The call is made from the thread that created `context`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_rendering_context_shared_remove_consumer(
    context: *mut RenderingContext,
    target_process_id: u32,
) -> i32 {
    assert!(!context.is_null(), "context pointer must not be null");
    // SAFETY: the caller upholds the requirements documented above.
    let inner = unsafe { &(*context).inner };
    match platform::remove_consumer(inner, target_process_id) {
        Ok(()) => SERVO_SHARED_SURFACE_OK,
        Err(status) => status,
    }
}

#[cfg(all(test, target_os = "windows"))]
#[path = "shared_surface_tests.rs"]
mod tests;

#[cfg(not(target_os = "windows"))]
mod platform {
    use std::rc::Rc;

    use super::ServoSharedSurfaceInfo;

    pub(super) fn create(
        _: u32,
        _: u32,
        _: u32,
    ) -> Result<Rc<dyn servo_api::RenderingContext>, String> {
        Err("shared surfaces are only implemented on Windows".into())
    }

    pub(super) fn info(_: &Rc<dyn servo_api::RenderingContext>) -> Option<ServoSharedSurfaceInfo> {
        None
    }

    pub(super) fn add_consumer(
        _: &Rc<dyn servo_api::RenderingContext>,
        _: u32,
    ) -> Result<ServoSharedSurfaceInfo, i32> {
        Err(super::SERVO_SHARED_SURFACE_NOT_SHARED)
    }

    pub(super) fn consumer_info(
        _: &Rc<dyn servo_api::RenderingContext>,
        _: u32,
    ) -> Result<ServoSharedSurfaceInfo, i32> {
        Err(super::SERVO_SHARED_SURFACE_NOT_SHARED)
    }

    pub(super) fn remove_consumer(
        _: &Rc<dyn servo_api::RenderingContext>,
        _: u32,
    ) -> Result<(), i32> {
        Err(super::SERVO_SHARED_SURFACE_NOT_SHARED)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::ptr;
    use std::rc::{Rc, Weak};
    use std::sync::Arc;

    use euclid::default::Size2D;
    use gleam::gl::{self, Gl};
    use servo_api::{DeviceIntRect, RgbaImage};
    use surfman::{
        Connection, Context, ContextAttributeFlags, ContextAttributes, Device, GLApi, Surface,
        SurfaceTexture,
    };
    use winapi::shared::dxgi::{DXGI_ADAPTER_DESC, IDXGIAdapter, IDXGIDevice, IDXGIKeyedMutex};
    use winapi::shared::dxgi1_2::{DXGI_SHARED_RESOURCE_READ, IDXGIResource1};
    use winapi::shared::dxgiformat::DXGI_FORMAT_R8G8B8A8_UNORM;
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::shared::winerror::{S_OK, SUCCEEDED};
    use winapi::um::d3d11::{
        D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
        D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX, D3D11_RESOURCE_MISC_SHARED_NTHANDLE,
        D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Device, ID3D11DeviceContext,
        ID3D11Resource, ID3D11Texture2D,
    };
    use winapi::um::handleapi::{CloseHandle, DuplicateHandle};
    use winapi::um::processthreadsapi::{GetCurrentProcess, OpenProcess};
    use winapi::um::winbase::INFINITE;
    use winapi::um::winnt::{
        DUPLICATE_CLOSE_SOURCE, DUPLICATE_SAME_ACCESS, HANDLE, PROCESS_DUP_HANDLE,
    };
    use wio::com::ComPtr;

    use super::ServoSharedSurfaceInfo;

    /// The key the engine releases a finished frame with, and the consumer acquires.
    const KEY_FRAME_READY: u64 = 1;
    /// The key the consumer releases with once it has finished reading.
    const KEY_CONSUMED: u64 = 0;

    /// One texture, its keyed mutex, and its handle in the target process.
    struct SharedTexture {
        texture: ComPtr<ID3D11Texture2D>,
        keyed_mutex: ComPtr<IDXGIKeyedMutex>,
        remote_handle: u64,
        size: Size2D<i32>,
    }

    pub(super) struct SharedSurfaceRenderingContext {
        gleam_gl: Rc<dyn Gl>,
        glow_gl: Arc<glow::Context>,
        device: RefCell<Device>,
        context: RefCell<Context>,
        d3d11_device: ComPtr<ID3D11Device>,
        target_process: HANDLE,
        adapter_luid: u64,
        texture: RefCell<SharedTexture>,
        /// Textures of their own for further processes, each sent a copy of every
        /// frame. See the module documentation.
        consumers: RefCell<Vec<ConsumerTexture>>,
        /// Whether this process currently holds the texture's keyed mutex.
        held: Cell<bool>,
        frame: Cell<u64>,
        generation: Cell<u64>,
    }

    thread_local! {
        /// See `NATIVE_CONTEXTS` in `native.rs`: the erased `Rc` has no way back to
        /// the concrete type, so keep a side table from its address.
        static SHARED_CONTEXTS: RefCell<HashMap<usize, Weak<SharedSurfaceRenderingContext>>> =
            RefCell::new(HashMap::new());
    }

    fn erased_key(inner: &Rc<dyn servo_api::RenderingContext>) -> usize {
        Rc::as_ptr(inner) as *const () as usize
    }

    /// A further process's own texture, and the process handle kept to reissue it
    /// on resize.
    struct ConsumerTexture {
        process_id: u32,
        process: HANDLE,
        texture: SharedTexture,
    }

    impl Drop for ConsumerTexture {
        fn drop(&mut self) {
            // SAFETY: opened in `add_consumer` and owned by this value.
            unsafe { CloseHandle(self.process) };
        }
    }

    fn find(
        inner: &Rc<dyn servo_api::RenderingContext>,
    ) -> Option<Rc<SharedSurfaceRenderingContext>> {
        let key = erased_key(inner);
        SHARED_CONTEXTS.with(|contexts| {
            let mut contexts = contexts.borrow_mut();
            let found = contexts.get(&key).and_then(Weak::upgrade);
            if found.is_none() {
                contexts.remove(&key);
            }
            found
        })
    }

    pub(super) fn info(
        inner: &Rc<dyn servo_api::RenderingContext>,
    ) -> Option<ServoSharedSurfaceInfo> {
        let context = find(inner)?;
        let texture = context.texture.borrow();
        Some(context.describe(&texture))
    }

    pub(super) fn add_consumer(
        inner: &Rc<dyn servo_api::RenderingContext>,
        process_id: u32,
    ) -> Result<ServoSharedSurfaceInfo, i32> {
        let context = find(inner).ok_or(super::SERVO_SHARED_SURFACE_NOT_SHARED)?;
        if context
            .consumers
            .borrow()
            .iter()
            .any(|consumer| consumer.process_id == process_id)
        {
            return Err(super::SERVO_SHARED_SURFACE_CONSUMER_EXISTS);
        }
        // SAFETY: plain Win32 call; the handle is closed by `ConsumerTexture`.
        let process = unsafe { OpenProcess(PROCESS_DUP_HANDLE, 0, process_id) };
        if process.is_null() {
            log::error!("shared surface: cannot open process {process_id} to share with");
            return Err(super::SERVO_SHARED_SURFACE_CONSUMER_FAILED);
        }
        let size = context.texture.borrow().size;
        let texture = match create_texture(&context.d3d11_device, process, size) {
            Ok((texture, _)) => texture,
            Err(error) => {
                log::error!("shared surface: consumer texture: {error}");
                // SAFETY: opened above and not stored anywhere on failure.
                unsafe { CloseHandle(process) };
                return Err(super::SERVO_SHARED_SURFACE_CONSUMER_FAILED);
            },
        };
        let info = context.describe(&texture);
        context.consumers.borrow_mut().push(ConsumerTexture {
            process_id,
            process,
            texture,
        });
        Ok(info)
    }

    pub(super) fn consumer_info(
        inner: &Rc<dyn servo_api::RenderingContext>,
        process_id: u32,
    ) -> Result<ServoSharedSurfaceInfo, i32> {
        let context = find(inner).ok_or(super::SERVO_SHARED_SURFACE_NOT_SHARED)?;
        let consumers = context.consumers.borrow();
        let consumer = consumers
            .iter()
            .find(|consumer| consumer.process_id == process_id)
            .ok_or(super::SERVO_SHARED_SURFACE_NO_CONSUMER)?;
        Ok(context.describe(&consumer.texture))
    }

    pub(super) fn remove_consumer(
        inner: &Rc<dyn servo_api::RenderingContext>,
        process_id: u32,
    ) -> Result<(), i32> {
        let context = find(inner).ok_or(super::SERVO_SHARED_SURFACE_NOT_SHARED)?;
        let mut consumers = context.consumers.borrow_mut();
        let index = consumers
            .iter()
            .position(|consumer| consumer.process_id == process_id)
            .ok_or(super::SERVO_SHARED_SURFACE_NO_CONSUMER)?;
        consumers.remove(index);
        Ok(())
    }

    pub(super) fn create(
        width: u32,
        height: u32,
        target_process_id: u32,
    ) -> Result<Rc<dyn servo_api::RenderingContext>, String> {
        // SAFETY: plain Win32 call; the handle is closed in `Drop`.
        let target_process = unsafe { OpenProcess(PROCESS_DUP_HANDLE, 0, target_process_id) };
        if target_process.is_null() {
            return Err(format!(
                "cannot open process {target_process_id} to share with"
            ));
        }

        let result = create_with_process(width, height, target_process);
        if result.is_err() {
            // SAFETY: opened above and not stored anywhere on failure.
            unsafe { CloseHandle(target_process) };
        }
        result
    }

    fn create_with_process(
        width: u32,
        height: u32,
        target_process: HANDLE,
    ) -> Result<Rc<dyn servo_api::RenderingContext>, String> {
        let connection = Connection::new().map_err(|error| format!("{error:?}"))?;
        let adapter = connection
            .create_hardware_adapter()
            .map_err(|error| format!("{error:?}"))?;
        let mut device = connection
            .create_device(&adapter)
            .map_err(|error| format!("{error:?}"))?;

        let flags = ContextAttributeFlags::ALPHA |
            ContextAttributeFlags::DEPTH |
            ContextAttributeFlags::STENCIL;
        let version = surfman::GLVersion { major: 3, minor: 0 };
        let descriptor = device
            .create_context_descriptor(&ContextAttributes { flags, version })
            .map_err(|error| format!("{error:?}"))?;
        let mut context = device
            .create_context(&descriptor, None)
            .map_err(|error| format!("{error:?}"))?;
        debug_assert!(matches!(connection.gl_api(), GLApi::GLES));

        // SAFETY: the loaders only resolve symbols from the context just created.
        let gleam_gl =
            unsafe { gl::GlesFns::load_with(|name| device.get_proc_address(&context, name)) };
        // SAFETY: as above.
        let glow_gl = unsafe {
            glow::Context::from_loader_function(|name| device.get_proc_address(&context, name))
        };

        // SAFETY: `native_device` adds a reference to the D3D11 device, which the
        // `ComPtr` takes over and releases.
        let d3d11_device = unsafe { ComPtr::from_raw(device.native_device().d3d11_device) };
        let adapter_luid = adapter_luid(&d3d11_device)?;

        let size = Size2D::new(width as i32, height as i32);
        let (texture, d3d_texture) = create_texture(&d3d11_device, target_process, size)?;
        bind_texture(&mut device, &mut context, d3d_texture, size)?;

        let shared = Rc::new(SharedSurfaceRenderingContext {
            gleam_gl,
            glow_gl: Arc::new(glow_gl),
            device: RefCell::new(device),
            context: RefCell::new(context),
            d3d11_device,
            target_process,
            adapter_luid,
            texture: RefCell::new(texture),
            consumers: RefCell::new(Vec::new()),
            held: Cell::new(false),
            frame: Cell::new(0),
            generation: Cell::new(1),
        });
        shared.take_ownership_after_bind();

        let erased: Rc<dyn servo_api::RenderingContext> = shared.clone();
        let key = erased_key(&erased);
        SHARED_CONTEXTS.with(|contexts| {
            contexts.borrow_mut().insert(key, Rc::downgrade(&shared));
        });
        Ok(erased)
    }

    fn adapter_luid(d3d11_device: &ComPtr<ID3D11Device>) -> Result<u64, String> {
        let dxgi_device = d3d11_device
            .cast::<IDXGIDevice>()
            .map_err(|hr| format!("IDXGIDevice: {hr:#x}"))?;
        let mut adapter: *mut IDXGIAdapter = ptr::null_mut();
        // SAFETY: `adapter` receives a new reference on success, taken over below.
        let hr = unsafe { dxgi_device.GetAdapter(&mut adapter) };
        if !SUCCEEDED(hr) || adapter.is_null() {
            return Err(format!("GetAdapter: {hr:#x}"));
        }
        // SAFETY: a new reference returned by `GetAdapter`.
        let adapter = unsafe { ComPtr::from_raw(adapter) };
        // SAFETY: plain-data out-parameter.
        let mut desc: DXGI_ADAPTER_DESC = unsafe { std::mem::zeroed() };
        // SAFETY: `desc` is valid for writes.
        let hr = unsafe { adapter.GetDesc(&mut desc) };
        if !SUCCEEDED(hr) {
            return Err(format!("GetDesc: {hr:#x}"));
        }
        Ok(((desc.AdapterLuid.HighPart as u32 as u64) << 32) | desc.AdapterLuid.LowPart as u64)
    }

    /// Creates the texture and places an unnamed, read-only NT handle to it in
    /// `target_process`, keeping no handle in this process.
    fn create_texture(
        d3d11_device: &ComPtr<ID3D11Device>,
        target_process: HANDLE,
        size: Size2D<i32>,
    ) -> Result<(SharedTexture, ComPtr<ID3D11Texture2D>), String> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: size.width as u32,
            Height: size.height as u32,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE,
            CPUAccessFlags: 0,
            MiscFlags: D3D11_RESOURCE_MISC_SHARED_NTHANDLE | D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX,
        };
        let mut texture: *mut ID3D11Texture2D = ptr::null_mut();
        // SAFETY: `desc` is fully initialized; `texture` receives a new reference.
        let hr = unsafe { d3d11_device.CreateTexture2D(&desc, ptr::null(), &mut texture) };
        if !SUCCEEDED(hr) || texture.is_null() {
            return Err(format!("CreateTexture2D: {hr:#x}"));
        }
        // SAFETY: a new reference returned by `CreateTexture2D`.
        let texture = unsafe { ComPtr::from_raw(texture) };

        let keyed_mutex = texture
            .cast::<IDXGIKeyedMutex>()
            .map_err(|hr| format!("IDXGIKeyedMutex: {hr:#x}"))?;
        let resource = texture
            .cast::<IDXGIResource1>()
            .map_err(|hr| format!("IDXGIResource1: {hr:#x}"))?;

        let mut local_handle: HANDLE = ptr::null_mut();
        // SAFETY: no security attributes and no name, so the handle is reachable
        // only through `local_handle`.
        let hr = unsafe {
            resource.CreateSharedHandle(
                ptr::null(),
                DXGI_SHARED_RESOURCE_READ,
                ptr::null(),
                &mut local_handle,
            )
        };
        if !SUCCEEDED(hr) || local_handle.is_null() {
            return Err(format!("CreateSharedHandle: {hr:#x}"));
        }

        let mut remote_handle: HANDLE = ptr::null_mut();
        // SAFETY: `DUPLICATE_CLOSE_SOURCE` closes `local_handle` whether or not the
        // duplication succeeds, so this process never keeps a handle to the texture.
        let ok = unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                local_handle,
                target_process,
                &mut remote_handle,
                0,
                0,
                DUPLICATE_SAME_ACCESS | DUPLICATE_CLOSE_SOURCE,
            )
        };
        if ok == 0 {
            return Err("DuplicateHandle into the target process failed".into());
        }

        Ok((
            SharedTexture {
                texture: texture.clone(),
                keyed_mutex,
                remote_handle: remote_handle as usize as u64,
                size,
            },
            texture,
        ))
    }

    fn bind_texture(
        device: &mut Device,
        context: &mut Context,
        texture: ComPtr<ID3D11Texture2D>,
        size: Size2D<i32>,
    ) -> Result<(), String> {
        // SAFETY: the texture was created on this device's own D3D11 device, and the
        // surface is only used on this thread.
        let surface = unsafe { device.create_surface_from_texture(context, &size, texture) }
            .map_err(|error| format!("{error:?}"))?;
        device
            .bind_surface_to_context(context, surface)
            .map_err(|(error, mut surface)| {
                let _ = device.destroy_surface(context, &mut surface);
                format!("{error:?}")
            })?;
        let _ = device.make_context_current(context);
        Ok(())
    }

    impl SharedSurfaceRenderingContext {
        fn describe(&self, texture: &SharedTexture) -> ServoSharedSurfaceInfo {
            ServoSharedSurfaceInfo {
                handle: texture.remote_handle,
                width: texture.size.width as u32,
                height: texture.size.height as u32,
                dxgi_format: DXGI_FORMAT_R8G8B8A8_UNORM,
                flags: super::SERVO_SHARED_SURFACE_FLAG_BOTTOM_UP,
                frame: self.frame.get(),
                adapter_luid: self.adapter_luid,
                generation: self.generation.get(),
            }
        }

        /// Copies the finished frame into every consumer texture that is free,
        /// never waiting on one. Called while this process holds the primary.
        fn publish_to_consumers(&self) {
            let consumers = self.consumers.borrow();
            if consumers.is_empty() {
                return;
            }
            let mut immediate: *mut ID3D11DeviceContext = ptr::null_mut();
            // SAFETY: `immediate` receives a new reference, taken over below.
            unsafe { self.d3d11_device.GetImmediateContext(&mut immediate) };
            // SAFETY: a new reference returned by `GetImmediateContext`.
            let immediate = unsafe { ComPtr::from_raw(immediate) };
            let source = self.texture.borrow();
            for consumer in consumers.iter() {
                let target = &consumer.texture;
                // SAFETY: live keyed mutexes and textures on this device. A texture
                // the consumer released is at key 0; one whose last frame it never
                // took is at key 1 and is overwritten. One it holds is skipped.
                unsafe {
                    if target.keyed_mutex.AcquireSync(KEY_CONSUMED, 0) != S_OK &&
                        target.keyed_mutex.AcquireSync(KEY_FRAME_READY, 0) != S_OK
                    {
                        continue;
                    }
                    immediate.CopyResource(
                        target.texture.as_raw() as *mut ID3D11Resource,
                        source.texture.as_raw() as *mut ID3D11Resource,
                    );
                    let hr = target.keyed_mutex.ReleaseSync(KEY_FRAME_READY);
                    if hr != S_OK {
                        log::error!("shared surface: consumer ReleaseSync failed: {hr:#x}");
                    }
                }
            }
        }

        /// After surfman binds a surface it may or may not hold the keyed mutex,
        /// depending on whether ANGLE reports it. Either way, make this process the
        /// holder: acquiring a mutex this device already owns fails harmlessly.
        fn take_ownership_after_bind(&self) {
            let texture = self.texture.borrow();
            // SAFETY: a live keyed mutex on this device.
            let _ = unsafe { texture.keyed_mutex.AcquireSync(KEY_CONSUMED, 0) };
            self.held.set(true);
        }

        /// Make sure this process holds the keyed mutex before touching the texture.
        fn ensure_held(&self) {
            if self.held.get() {
                return;
            }
            let texture = self.texture.borrow();
            // SAFETY: a live keyed mutex on this device.
            unsafe {
                // A frame the consumer never took is reclaimed at once and dropped.
                if texture.keyed_mutex.AcquireSync(KEY_FRAME_READY, 0) != S_OK {
                    // Otherwise wait for the consumer to finish with it.
                    let hr = texture.keyed_mutex.AcquireSync(KEY_CONSUMED, INFINITE);
                    if hr != S_OK {
                        log::error!("shared surface: AcquireSync failed: {hr:#x}");
                    }
                }
            }
            self.held.set(true);
        }

        fn framebuffer_id(&self) -> u32 {
            let device = self.device.borrow();
            let context = self.context.borrow();
            device
                .context_surface_info(&context)
                .ok()
                .flatten()
                .and_then(|info| info.framebuffer_object)
                .map_or(0, |framebuffer| framebuffer.0.get())
        }

        fn replace_texture(&self, size: Size2D<i32>) -> Result<(), String> {
            // surfman releases the mutex with key 0 on unbind and asserts that it can.
            self.ensure_held();
            let device = &mut *self.device.borrow_mut();
            let context = &mut *self.context.borrow_mut();
            if let Ok(Some(mut old)) = device.unbind_surface_from_context(context) {
                let _ = device.destroy_surface(context, &mut old);
            }
            self.held.set(false);

            let (texture, d3d_texture) =
                create_texture(&self.d3d11_device, self.target_process, size)?;
            *self.texture.borrow_mut() = texture;
            bind_texture(device, context, d3d_texture, size)?;
            self.generation.set(self.generation.get() + 1);

            // Each consumer gets a new texture at the new size. One that cannot be
            // reissued is dropped, and its process then has no consumer here.
            self.consumers.borrow_mut().retain_mut(|consumer| {
                match create_texture(&self.d3d11_device, consumer.process, size) {
                    Ok((texture, _)) => {
                        consumer.texture = texture;
                        true
                    },
                    Err(error) => {
                        log::error!(
                            "shared surface: dropping consumer {} on resize: {error}",
                            consumer.process_id
                        );
                        false
                    },
                }
            });
            Ok(())
        }
    }

    impl Drop for SharedSurfaceRenderingContext {
        fn drop(&mut self) {
            self.ensure_held();
            let device = &mut *self.device.borrow_mut();
            let context = &mut *self.context.borrow_mut();
            if let Ok(Some(mut surface)) = device.unbind_surface_from_context(context) {
                let _ = device.destroy_surface(context, &mut surface);
            }
            let _ = device.destroy_context(context);
            // SAFETY: opened in `create` and owned by this value.
            unsafe { CloseHandle(self.target_process) };
        }
    }

    impl servo_api::RenderingContext for SharedSurfaceRenderingContext {
        fn prepare_for_rendering(&self) {
            self.ensure_held();
            self.gleam_gl
                .bind_framebuffer(gl::FRAMEBUFFER, self.framebuffer_id());
        }

        fn read_to_image(&self, source_rectangle: DeviceIntRect) -> Option<RgbaImage> {
            self.ensure_held();
            let gl = &self.gleam_gl;
            gl.bind_framebuffer(gl::FRAMEBUFFER, self.framebuffer_id());
            gl.bind_vertex_array(0);
            let mut pixels = gl.read_pixels(
                source_rectangle.min.x,
                source_rectangle.min.y,
                source_rectangle.width(),
                source_rectangle.height(),
                gl::RGBA,
                gl::UNSIGNED_BYTE,
            );
            // GL rows run bottom to top.
            let width = source_rectangle.width() as usize;
            let height = source_rectangle.height() as usize;
            let stride = width * 4;
            let original = pixels.clone();
            for y in 0..height {
                let source = (height - y - 1) * stride;
                pixels[y * stride..(y + 1) * stride]
                    .copy_from_slice(&original[source..source + stride]);
            }
            RgbaImage::from_raw(width as u32, height as u32, pixels)
        }

        fn size(&self) -> dpi::PhysicalSize<u32> {
            let size = self.texture.borrow().size;
            dpi::PhysicalSize::new(size.width as u32, size.height as u32)
        }

        fn resize(&self, size: dpi::PhysicalSize<u32>) {
            if size.width == 0 || size.height == 0 {
                log::error!("shared surface: refusing to resize to {size:?}");
                return;
            }
            let size = Size2D::new(size.width as i32, size.height as i32);
            if size == self.texture.borrow().size {
                return;
            }
            if let Err(error) = self.replace_texture(size) {
                log::error!("shared surface: resize failed: {error}");
            }
        }

        fn present(&self) {
            if !self.held.get() {
                return;
            }
            // ANGLE draws through this device's immediate context, so after the
            // flush the copies below are ordered after the frame.
            self.gleam_gl.flush();
            self.publish_to_consumers();
            let texture = self.texture.borrow();
            // SAFETY: a live keyed mutex this process holds.
            let hr = unsafe { texture.keyed_mutex.ReleaseSync(KEY_FRAME_READY) };
            if hr != S_OK {
                log::error!("shared surface: ReleaseSync failed: {hr:#x}");
                return;
            }
            self.held.set(false);
            self.frame.set(self.frame.get() + 1);
        }

        fn make_current(&self) -> Result<(), surfman::Error> {
            let device = self.device.borrow();
            let context = self.context.borrow();
            device.make_context_current(&context)
        }

        fn gleam_gl_api(&self) -> Rc<dyn Gl> {
            self.gleam_gl.clone()
        }

        fn glow_gl_api(&self) -> Arc<glow::Context> {
            self.glow_gl.clone()
        }

        fn create_texture(
            &self,
            surface: Surface,
        ) -> Option<(SurfaceTexture, u32, euclid::default::Size2D<i32>)> {
            let device = self.device.borrow();
            let context = &mut *self.context.borrow_mut();
            let size = device.surface_info(&surface).size;
            let surface_texture = device.create_surface_texture(context, surface).ok()?;
            let gl_texture = device
                .surface_texture_object(&surface_texture)
                .map_or(0, |texture| texture.0.get());
            Some((surface_texture, gl_texture, size))
        }

        fn destroy_texture(&self, surface_texture: SurfaceTexture) -> Option<Surface> {
            let device = self.device.borrow();
            let context = &mut *self.context.borrow_mut();
            device
                .destroy_surface_texture(context, surface_texture)
                .map_err(|(error, _)| error)
                .ok()
        }

        fn connection(&self) -> Option<Connection> {
            Some(self.device.borrow().connection())
        }
    }
}
