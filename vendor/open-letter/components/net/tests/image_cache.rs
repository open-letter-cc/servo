// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `components/net/tests/image_cache.rs`.

#[allow(unused_imports)]
use super::*;

/// `rasterize_vector_image_now` returns pixels on the calling thread, without a WebRender
/// key: key requests are deliberately left unanswered once the image has loaded.
#[test]
pub(super) fn test_svg_rasterization_now() {
    let (cache, key_receiver) = create_test_image_cache();
    let url = ServoUrl::parse("http://example.com/image.svg").unwrap();
    let origin = mock_origin();

    let id = match cache.get_cached_image_status(url.clone(), &origin, None) {
        ImageCacheResult::ReadyForRequest(id) => id,
        _ => panic!("Expected ReadyForRequest"),
    };

    cache.notify_pending_response(
        id,
        FetchResponseMsg::ProcessResponse(
            create_request_id(),
            Ok(create_test_metadata(Some(mime::IMAGE_SVG))),
        ),
    );
    cache.notify_pending_response(
        id,
        FetchResponseMsg::ProcessResponseChunk(create_request_id(), svg_image_bytes().into()),
    );
    cache.notify_pending_response(
        id,
        FetchResponseMsg::ProcessResponseEOF(create_request_id(), Ok(()), create_timing()),
    );

    let vec_img = loop {
        handle_pending_key_requests(&cache, &key_receiver);
        std::thread::sleep(std::time::Duration::from_millis(10));
        let result = cache.get_cached_image_status(url.clone(), &origin, None);
        let ImageCacheResult::Available(ImageOrMetadataAvailable::ImageAvailable { image, .. }) =
            result
        else {
            continue;
        };

        let net_traits::image_cache::Image::Vector(vec_img) = image else {
            panic!("Expected vector image");
        };
        break vec_img;
    };

    let size = webrender_api::units::DeviceIntSize::new(100, 100);
    let raster = cache
        .rasterize_vector_image_now(vec_img.id, size)
        .expect("Expected pixels without waiting for a key");
    assert_eq!((raster.metadata.width, raster.metadata.height), (100, 100));
    assert!(raster.id.is_none());

    // The centre of the fixture's red circle.
    let offset = (50 * 100 + 50) * 4;
    assert_eq!(&raster.bytes[offset..offset + 4], &[255, 0, 0, 255]);

    let empty = webrender_api::units::DeviceIntSize::new(100, 0);
    assert!(cache.rasterize_vector_image_now(vec_img.id, empty).is_none());
}
