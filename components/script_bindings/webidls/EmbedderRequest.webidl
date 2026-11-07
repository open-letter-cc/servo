// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

// This interface is internal to Servo. It is not a web standard and is exposed only
// when the embedder turns on `dom_embedder_request_enabled`.
//
// `send()` delivers a copy of `data` to the embedder as a page-request embedder control
// and resolves with the embedder's answer, which may be empty. It rejects if the
// embedder declines, does not take page requests, or drops the request unanswered.
// Servo never parses either side's bytes.
//
// WebAssembly reaches it as an ordinary JavaScript import; there is no
// WebAssembly-specific path.

[Exposed=Window, Pref="dom_embedder_request_enabled"]
namespace EmbedderRequest {
  Promise<Uint8Array> send(BufferSource data);
};
