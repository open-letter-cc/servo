// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/net/image_cache.rs`.

#[allow(unused_imports)]
use super::*;

use std::cell::{Cell, OnceCell};
use std::cmp::min;
use std::collections::hash_map::Entry::{Occupied, Vacant};
use std::collections::{HashMap, HashSet, VecDeque};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::{mem, thread_local};
use imsz::imsz_from_reader;
use log::{debug, error, warn};
use malloc_size_of::{MallocConditionalSizeOf, MallocSizeOf as MallocSizeOfTrait, MallocSizeOfOps};
use malloc_size_of_derive::MallocSizeOf;
use mime::Mime;
use net_traits::image_cache::{
    FontResolver, Image, ImageCache, ImageCacheFactory, ImageCacheResponseCallback,
    ImageCacheResponseMessage, ImageCacheResult, ImageLoadListener, ImageOrMetadataAvailable,
    ImageResponse, PendingImageId, RasterizationCompleteResponse, RasterizationResult, VectorImage,
};
use net_traits::request::CorsSettings;
use net_traits::{FetchResponseMsg, Metadata, NetworkError};
use paint_api::{CrossProcessPaintApi, ImageUpdate, SerializableImageData};
use parking_lot::Mutex;
use pixels::{CorsStatus, ImageFrame, ImageMetadata, PixelFormat, RasterImage, load_from_memory};
use profile_traits::mem::{Report, ReportKind};
use profile_traits::path;
use resvg::tiny_skia;
use resvg::usvg::{self, fontdb};
use rustc_hash::{FxHashMap, FxHashSet};
use servo_base::cross_process_instant::CrossProcessInstant;
use servo_base::id::{PipelineId, WebViewId};
use servo_base::threadpool::ThreadPool;
use servo_url::{ImmutableOrigin, ServoUrl};
use uuid::Uuid;
use webrender_api::ImageKey as WebRenderImageKey;
use webrender_api::units::DeviceIntSize;
use crate::image_cache::KeyCacheState::PipelineClosed;

/// The size of the pixmap that rasterizing `svg_tree` at `requested_size` allocates:
/// clamped to [`MAX_SVG_PIXMAP_DIMENSION`] on each axis, or the clamped natural size if
/// the requested one is empty or not representable. `None` if the natural size has a
/// zero dimension, or the result is empty or too wide for `tiny_skia::Pixmap`.
pub(super) fn svg_pixmap_size(
    svg_tree: &usvg::Tree,
    requested_size: DeviceIntSize,
) -> Option<tiny_skia::IntSize> {
    let natural_size = svg_tree.size().to_int_size();
    let natural_tinyskia_size = {
        let width = natural_size.width().min(MAX_SVG_PIXMAP_DIMENSION);
        let height = natural_size.height().min(MAX_SVG_PIXMAP_DIMENSION);
        tiny_skia::IntSize::from_wh(width, height)
    };
    let pixmap_size = {
        let width = requested_size
            .width
            .try_into()
            .unwrap_or(0)
            .min(MAX_SVG_PIXMAP_DIMENSION);
        let height = requested_size
            .height
            .try_into()
            .unwrap_or(0)
            .min(MAX_SVG_PIXMAP_DIMENSION);
        tiny_skia::IntSize::from_wh(width, height).or(natural_tinyskia_size)
    };
    let Some(pixmap_size) = pixmap_size else {
        debug!(
            "Asked for requested size {:?} which has zero size, and natural size {:?} has zero dimension. Not returning image",
            requested_size, natural_size
        );
        return None;
    };

    if natural_tinyskia_size.is_none() {
        debug!("SVG natural size has zero dimension. Not returning image");
        return None;
    }

    // Requirements from tiny_skia::Pixmap::new
    if pixmap_size.width() > (i32::MAX / 4).try_into().unwrap() {
        debug!(
            "Asked for requested size {:?} which is too wide. Not returning image",
            requested_size
        );
        return None;
    }
    Some(pixmap_size)
}

/// Rasterizes `vector_image` into a pixmap of `pixmap_size`, on the calling thread.
///
/// Some SVG documents, primarily ones created by fuzzers, can cause resvg to fail
/// assertions and panic. We catch any panics in `resvg::render` here so that we don't
/// crash the whole engine for such cases, and return `Err`.
pub(super) fn render_svg(
    vector_image: &VectorImageData,
    pixmap_size: tiny_skia::IntSize,
) -> std::thread::Result<RasterImage> {
    let natural_size = vector_image.svg_tree.size().to_int_size();
    let transform = tiny_skia::Transform::from_scale(
        pixmap_size.width() as f32 / natural_size.width() as f32,
        pixmap_size.height() as f32 / natural_size.height() as f32,
    );
    let mut pixmap = tiny_skia::Pixmap::new(pixmap_size.width(), pixmap_size.height()).unwrap();

    // We also need to set `SUPPRESS_ABORT_IN_PANIC_HOOK` for the duration of the call
    // because servoshell registers a custom hook that intercepts the panic and crashes
    // the process when run in hard_fail mode.
    //
    // `AssertUnwindSafe` should be safe here since callers do not use the `vector_image`
    // again after a panic. This assumes `resvg::render` doesn't use internal global state
    // that could become invalid after the panic.
    SUPPRESS_ABORT_IN_PANIC_HOOK.set(true);
    let resvg_result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        resvg::render(&vector_image.svg_tree, transform, &mut pixmap.as_mut());

        let bytes = pixmap.take();
        let frame = ImageFrame {
            delay: None,
            byte_range: 0..bytes.len(),
            width: pixmap_size.width(),
            height: pixmap_size.height(),
        };

        RasterImage {
            metadata: ImageMetadata {
                width: pixmap_size.width(),
                height: pixmap_size.height(),
            },
            format: PixelFormat::RGBA8,
            frames: vec![frame],
            bytes: Arc::new(bytes),
            id: None,
            cors_status: vector_image.cors_status,
            load_time: None,
            is_opaque: false,
            loop_count: None,
        }
    }));
    SUPPRESS_ABORT_IN_PANIC_HOOK.set(false);
    resvg_result
}

impl ImageCacheImpl {

    /// Body of [`WebViewDelegate::rasterize_vector_image_now`] for this delegate.
    pub(super) fn open_letter_rasterize_vector_image_now(
        &self,
        image_id: PendingImageId,
        requested_size: DeviceIntSize,
    ) -> Option<RasterImage> {
        // `svg_pixmap_size` falls back to the natural size for an empty request; a caller
        // asking for pixels synchronously asked for this size and no other.
        if requested_size.is_empty() {
            return None;
        }
        let vector_image = {
            let store = self.store.lock();
            if let Some(result) = store
                .rasterized_vector_images
                .get(&(image_id, requested_size))
                .and_then(|task| task.result.clone())
            {
                return Some(result);
            }
            store.vector_images.get(&image_id).cloned()?
        };

        // Rendered outside the lock, so that a large image does not hold up the cache's
        // other users. Nothing is stored: the result has no WebRender key, and giving it one
        // would mean waiting for the script thread, which is the thread calling this.
        let pixmap_size = svg_pixmap_size(&vector_image.svg_tree, requested_size)?;
        match render_svg(&vector_image, pixmap_size) {
            Ok(rasterized_image) => Some(rasterized_image),
            Err(_) => {
                warn!("resvg panicked while rasterizing SVG image {image_id:?} at {requested_size:?}");
                // As in `rasterize_vector_image`: never rasterize this image again.
                self.store.lock().vector_images.remove(&image_id);
                None
            },
        }
    }
}
