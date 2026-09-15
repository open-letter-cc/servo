# motor-controller

A generic, reusable **WebView abstraction seam** that lets any Rust desktop
application embed [Servo](https://servo.org) behind a strongly typed,
engine-agnostic interface.

The name follows Servo's own motor metaphor: this crate is the controller an
application drives, not the engine itself.

## What it is for

An application that embeds a browser engine usually ends up coupled to it —
engine types leak into the compositor, the input router, and the application's
own navigation model. This crate is the seam that prevents that. Your
application talks to the `WebView` trait; it never names a rendering engine
directly.

That buys two things:

- **The engine is swappable.** `StubWebView` renders nothing but tracks state
  faithfully, so an application's windowing, compositor and input wiring can be
  built and tested with no engine present. Attaching a real backend later is a
  change of type at the construction site, and nothing else.
- **Your routing model stays yours.** The seam is generic over the route type.
  It never inspects a route's internals, so an application's routing taxonomy
  never has to be expressed in this crate.

## The `Route` boundary

`WebView` is generic over any type implementing `Route`:

```rust
pub trait Route {
    fn is_privileged(&self) -> bool;
    fn to_nav_string(&self) -> String;
}

pub trait WebView<R: Route> {
    fn load(&mut self, route: R);
    fn current_route(&self) -> Option<&R>;
}
```

Two methods are the entire contract. Implement `Route` on your own type and it
works with every view in this crate:

```rust
use motor_controller::{Route, StubWebView, WebView};

struct AppRoute(String);

impl Route for AppRoute {
    fn is_privileged(&self) -> bool {
        self.0.starts_with("chrome/")
    }
    fn to_nav_string(&self) -> String {
        self.0.clone()
    }
}

let mut content_view = StubWebView::new(false); // unprivileged origin
content_view.load(AppRoute("chrome/settings".into()));
assert!(content_view.current_route().is_none()); // refused
```

`WebView` is generic over `R` rather than using an associated type, so
`dyn WebView<R>` stays object-safe for applications that store views behind a
trait object.

### The privilege gate

A view's origin privilege is fixed at construction — a chrome view and a content
view are never the same `WebView` switching modes. An unprivileged view refuses
any route whose `is_privileged()` returns `true`.

Derive `is_privileged` from the route's own structure, not from caller-supplied
state. A bundled `NavRoute` is provided for applications that don't need a route
type of their own, but note that it is a *carrier*, not an authority: it reports
whichever privilege its constructor was given.

> **Note:** this check is meaningful bookkeeping while nothing renders. Behind a
> real engine it is necessary but **not sufficient** — see the boundaries
> documented on `ServoWebView` regarding disk-cache containment, origin
> isolation and engine-internal IPC before attaching a live `Servo` instance.

## Features

| Feature | Default | Effect |
|---|---|---|
| *(none)* | ✅ | `StubWebView` only. No LLVM, no SpiderMonkey, no engine build. |
| `servo-backend` | ❌ | Pulls in real `libservo` and compiles `ServoWebView`. |

The default build is dependency-free and checks in seconds, which keeps the
baseline green for contributors who are not working on the renderer. Everything
the feature enables is behind `#[cfg(feature = "servo-backend")]`, so nothing
outside this crate changes shape when it is toggled.

Building with `servo-backend` requires a full Servo toolchain, including
SpiderMonkey. On Windows that additionally means MozTools or MozillaBuild — see
[servo/mozjs](https://github.com/servo/mozjs?tab=readme-ov-file#windows).
`--no-default-features` does not avoid this: `script` is a non-optional
dependency of `components/servo`, so SpiderMonkey is required at every feature
combination.

## Building

```sh
cargo check -p motor-controller    # default: stub only, no engine toolchain
cargo test  -p motor-controller    # unit tests for the privilege gate
```

## License

Licensed under the Mozilla Public License 2.0 ([MPL-2.0](https://www.mozilla.org/MPL/2.0/)),
matching Servo's own licensing.
