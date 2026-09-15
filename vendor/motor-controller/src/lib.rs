// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org.

//! WebView abstraction seam: an embedding application talks to `WebView`,
//! never to a rendering engine directly. Today the only implementation is
//! `StubWebView`, which tracks the current route and enforces the
//! privileged/unprivileged origin boundary, but renders nothing. Swapping
//! in a real Servo backend later means adding `impl WebView for
//! ServoWebView` here; the host's compositor/input wiring doesn't change.
//!
//! This crate is self-contained and has no non-optional dependencies. The
//! route type is not fixed: an embedder defines its own and implements
//! [`Route`] for it, so a host's routing taxonomy stays entirely in the
//! host's own code. [`NavRoute`] is provided for embedders that don't need
//! one of their own.
...

pub trait Route {
    /// Whether this route may only be loaded by a privileged (chrome)
    /// view. An unprivileged view refuses to load any route for which this
    /// returns `true`.
    ///
    /// This is the privilege gate. Implementations should derive it from
    /// the route itself, not from caller-supplied state.
    fn is_privileged(&self) -> bool;

    /// The route rendered as a navigable string, for logging and for
    /// handing to a backend.
    fn to_nav_string(&self) -> String;
}

/// A view that can be navigated to routes of type `R`.
///
/// Generic over `R` rather than using an associated type, so that
/// `dyn WebView<R>` remains object-safe for hosts that store views behind
/// a trait object.
pub trait WebView<R: Route> {
    /// Navigate this view to `route`. Implementations enforce their own
    /// privilege boundary: an unprivileged (content-origin) view must
    /// refuse any route where [`Route::is_privileged`] is true.
    fn load(&mut self, route: R);

    /// The route currently loaded, if any.
    fn current_route(&self) -> Option<&R>;
}

/// A ready-made [`Route`] for embedders that do not have a route type of
/// their own.
///
/// This is a *carrier*, not an authority: it reports whichever privilege
/// its constructor was given. That is fine for a host whose privileged
/// surfaces are known at the call site, but a host with a real routing
/// taxonomy should implement [`Route`] on its own type and derive
/// `is_privileged` from the route's structure instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavRoute {
    nav: String,
    privileged: bool,
}

impl NavRoute {
    /// A route loadable by any view.
    pub fn unprivileged(nav: impl Into<String>) -> Self {
        Self {
            nav: nav.into(),
            privileged: false,
        }
    }

    /// A route loadable only by a privileged (chrome) view.
    pub fn privileged(nav: impl Into<String>) -> Self {
        Self {
            nav: nav.into(),
            privileged: true,
        }
    }

    /// The navigable string this route was built from.
    pub fn nav(&self) -> &str {
        &self.nav
    }
}

impl Route for NavRoute {
    fn is_privileged(&self) -> bool {
        self.privileged
    }

    fn to_nav_string(&self) -> String {
        self.nav.clone()
    }
}

/// Placeholder backend used until a real Servo WebView is wired up. State
/// is tracked faithfully enough for a host's compositor/input wiring to be
/// developed and tested against; nothing is actually rendered.
pub struct StubWebView<R> {
    origin_is_privileged: bool,
    current: Option<R>,
}

impl<R> StubWebView<R> {
    /// `origin_is_privileged` is fixed at construction: a chrome view and a
    /// content view are never the same WebView switching modes.
    pub fn new(origin_is_privileged: bool) -> Self {
        Self {
            origin_is_privileged,
            current: None,
        }
    }
}

impl<R: Route> WebView<R> for StubWebView<R> {
    fn load(&mut self, route: R) {
        if route.is_privileged() && !self.origin_is_privileged {
            eprintln!(
                "[motor-controller] blocked privileged nav from unprivileged view: {}",
                route.to_nav_string()
            );
            return;
        }
        println!("[motor-controller] (stub) load {}", route.to_nav_string());
        self.current = Some(route);
    }

    fn current_route(&self) -> Option<&R> {
        self.current.as_ref()
    }
}

/// Real libservo-backed view. Compiled only under the `servo-backend`
/// feature; a host still only ever names the [`WebView`] trait, so toggling
/// the feature does not change the host's `ApplicationHandler` shape.
///
/// SECURITY - this is deliberately not yet constructible with a real engine
/// attached. On `StubWebView` the `origin_is_privileged` flag is honest
/// bookkeeping because nothing renders. Behind a real engine it is not
/// self-enforcing, and three boundaries have to be established by the
/// embedder before a `Servo` instance may be attached here:
///
/// 1. Disk-cache containment. libservo's net stack keeps its own HTTP
///    cache, cookie jar and localStorage. If an embedder serves content
///    that is decrypted or otherwise privileged in memory, a default servo
///    profile directory will write that plaintext to disk outside the
///    embedder's custody. The profile/cache dir must be pinned to a
///    location the embedder governs, or disabled outright, at construction.
/// 2. Origin isolation. Two views sharing one constellation share cookie
///    jars, storage and JS realms. A privileged/unprivileged split has to
///    map onto separate servo `WebView`s with distinct origins (ideally
///    separate constellations), or the boundary regresses from "not yet
///    implemented" to actively false - which is worse, because the flag
///    makes it look enforced.
/// 3. IPC separation. libservo brings its own `ipc-channel` fabric between
///    constellation and content processes. That is engine-internal and must
///    never be used to carry an embedder's privilege tokens.
///
/// A fourth item is unresolved in general: network egress. Once a real
/// engine renders arbitrary markup, that markup can originate outbound
/// requests through servo's fetch stack. Embedders that assume a closed
/// egress set need a fetch-interception or allowlist decision first.
#[cfg(feature = "servo-backend")]
pub struct ServoWebView<R> {
    origin_is_privileged: bool,
    current: Option<R>,
}

#[cfg(feature = "servo-backend")]
impl<R> ServoWebView<R> {
    /// Same fixed-at-construction privilege flag as [`StubWebView`].
    pub fn new(origin_is_privileged: bool) -> Self {
        Self {
            origin_is_privileged,
            current: None,
        }
    }

    /// Platform hooks for attaching a real `Servo` instance to a native
    /// surface. Kept as `cfg` blocks from the start so the portable core
    /// never accumulates target-specific linkage: Windows/Linux today,
    /// macOS to follow.
    #[allow(dead_code)]
    fn platform_surface_hint() -> &'static str {
        #[cfg(target_os = "windows")]
        {
            // ANGLE/WGL: servo's `no-wgl` feature swaps surfman onto ANGLE.
            "windows/angle"
        }
        #[cfg(target_os = "linux")]
        {
            // Wayland-first, per the host compositor.
            "linux/wayland"
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            "unsupported"
        }
    }
}

#[cfg(feature = "servo-backend")]
impl<R: Route> WebView<R> for ServoWebView<R> {
    fn load(&mut self, route: R) {
        // Identical privilege gate to StubWebView. This check is necessary
        // but NOT sufficient once an engine is attached - see the three
        // boundaries in this type's doc comment.
        if route.is_privileged() && !self.origin_is_privileged {
            eprintln!(
                "[motor-controller] blocked privileged nav from unprivileged view: {}",
                route.to_nav_string()
            );
            return;
        }
        self.current = Some(route);
    }

    fn current_route(&self) -> Option<&R> {
        self.current.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_view_refuses_privileged_navigation() {
        let mut view = StubWebView::new(false);
        view.load(NavRoute::privileged("chrome/settings"));
        assert!(view.current_route().is_none());
    }

    #[test]
    fn chrome_view_accepts_privileged_navigation() {
        let mut view = StubWebView::new(true);
        let route = NavRoute::privileged("chrome/settings");
        view.load(route.clone());
        assert_eq!(view.current_route(), Some(&route));
    }

    #[test]
    fn content_view_accepts_unprivileged_navigation() {
        let mut view = StubWebView::new(false);
        let route = NavRoute::unprivileged("https://example.com/");
        view.load(route.clone());
        assert_eq!(view.current_route(), Some(&route));
    }

    /// A custom `Route` impl needs nothing from this crate but the trait -
    /// this is the shape an embedder with its own taxonomy uses.
    #[test]
    fn custom_route_type_drives_the_gate() {
        struct MyRoute(&'static str);
        impl Route for MyRoute {
            fn is_privileged(&self) -> bool {
                self.0.starts_with("chrome/")
            }
            fn to_nav_string(&self) -> String {
                self.0.to_string()
            }
        }

        let mut content = StubWebView::new(false);
        content.load(MyRoute("chrome/settings"));
        assert!(content.current_route().is_none());

        content.load(MyRoute("content/index"));
        assert!(content.current_route().is_some());
    }
}
