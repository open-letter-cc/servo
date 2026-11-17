// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/layout/layout_impl.rs`.

#[allow(unused_imports)]
use super::*;

/// The most threads the style system's per-thread data supports in one pool.
pub(super) const MAX_LAYOUT_POOL_THREADS: usize = 6;

/// A pool to style and lay out in parallel on, used in place of the style system's
/// pool when the `thread_layout_stack_size_kb` preference asks for larger stacks than
/// that pool's threads have. Parallel layout recurses with the depth of the page, and
/// unlike styling it has no depth limiter, so a deeply nested page can exhaust the style
/// threads' stacks. Built from the preferences when first used.
pub(super) static LAYOUT_THREAD_POOL: LazyLock<Option<rayon::ThreadPool>> = LazyLock::new(|| {
    let stack_size_kb = pref!(thread_layout_stack_size_kb).max(0) as usize;
    // Smaller stacks than the style system's would defeat its depth limiter.
    if stack_size_kb <= style::parallel::STYLE_THREAD_STACK_SIZE_KB {
        return None;
    }
    // The same number of threads as the style system's pool.
    let threads = pref!(layout_threads);
    let threads = if threads >= 0 {
        threads as usize
    } else {
        std::thread::available_parallelism().map_or(1, |cores| (cores.get() * 3 / 4).max(1))
    };
    let threads = threads.min(MAX_LAYOUT_POOL_THREADS);
    if threads <= 1 {
        return None;
    }
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|index| format!("Layout#{index}"))
        .stack_size(stack_size_kb * 1024)
        .start_handler(|_| style::thread_state::initialize_layout_worker_thread())
        .build()
        .ok()
});
