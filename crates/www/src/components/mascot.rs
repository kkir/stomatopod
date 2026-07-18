//! Hero mascot with a light scroll parallax + idle float.

use dioxus::prelude::*;

const MASCOT_ID: &str = "hero-mascot";

/// Teal/purple mantis shrimp for the home hero. On the hydrating client it
/// swims gently upward and tips as the page scrolls; CSS handles idle bob.
#[component]
pub fn HeroMascot() -> Element {
    #[cfg(target_arch = "wasm32")]
    let parallax = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(None::<MascotParallax>)));
    let mounted = use_hook(|| std::rc::Rc::new(std::cell::Cell::new(true)));

    use_effect({
        let mounted = mounted.clone();
        #[cfg(target_arch = "wasm32")]
        let parallax = parallax.clone();
        move || {
            mounted.set(true);
            #[cfg(target_arch = "wasm32")]
            {
                *parallax.borrow_mut() = install_mascot_parallax(mounted.clone());
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = &mounted;
            }
        }
    });

    use_drop({
        let mounted = mounted.clone();
        #[cfg(target_arch = "wasm32")]
        let parallax = parallax.clone();
        move || {
            mounted.set(false);
            #[cfg(target_arch = "wasm32")]
            {
                *parallax.borrow_mut() = None;
            }
        }
    });

    rsx! {
        // Outer layer receives scroll-driven translate/rotate from JS.
        // Inner img keeps the CSS bob so the two transforms do not fight.
        div {
            id: "{MASCOT_ID}",
            class: "mascot-parallax will-change-transform",
            img {
                src: asset!("/assets/mascot.png"),
                alt: "Stomatopod mascot - a teal and purple mantis shrimp",
                width: "320",
                height: "320",
                class: "mascot-bob w-44 sm:w-56 lg:w-72 h-auto drop-shadow-[0_18px_40px_rgba(45,212,191,0.22)] select-none pointer-events-none",
                decoding: "async",
                draggable: "false",
            }
        }
    }
}

/// Scroll listener + element handle; removes the listener on drop.
#[cfg(target_arch = "wasm32")]
struct MascotParallax {
    window: web_sys::Window,
    _on_scroll: wasm_bindgen::closure::Closure<dyn FnMut()>,
}

#[cfg(target_arch = "wasm32")]
impl Drop for MascotParallax {
    fn drop(&mut self) {
        use wasm_bindgen::JsCast;
        let _ = self.window.remove_event_listener_with_callback(
            "scroll",
            self._on_scroll.as_ref().unchecked_ref(),
        );
        // Leave transform at rest so a remount starts clean.
        if let Some(document) = self.window.document() {
            if let Some(el) = document.get_element_by_id(MASCOT_ID) {
                if let Ok(html) = el.dyn_into::<web_sys::HtmlElement>() {
                    let _ = html.style().remove_property("transform");
                }
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn install_mascot_parallax(mounted: std::rc::Rc<std::cell::Cell<bool>>) -> Option<MascotParallax> {
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;

    let window = web_sys::window()?;
    let document = window.document()?;
    let el = document
        .get_element_by_id(MASCOT_ID)?
        .dyn_into::<web_sys::HtmlElement>()
        .ok()?;

    // Honor reduced motion: keep the static pose only.
    if prefers_reduced_motion(&window) {
        return None;
    }

    // Apply once so a mid-page refresh is already posed correctly.
    apply_parallax(&window, &el);

    let window_cb = window.clone();
    let el_cb = el.clone();
    let on_scroll = Closure::wrap(Box::new(move || {
        if !mounted.get() {
            return;
        }
        apply_parallax(&window_cb, &el_cb);
    }) as Box<dyn FnMut()>);

    window
        .add_event_listener_with_callback("scroll", on_scroll.as_ref().unchecked_ref())
        .ok()?;

    Some(MascotParallax {
        window,
        _on_scroll: on_scroll,
    })
}

#[cfg(target_arch = "wasm32")]
fn prefers_reduced_motion(window: &web_sys::Window) -> bool {
    window
        .match_media("(prefers-reduced-motion: reduce)")
        .ok()
        .flatten()
        .map(|mq| mq.matches())
        .unwrap_or(false)
}

/// Maps the first ~half viewport of scroll into a small swim-up + tip.
#[cfg(target_arch = "wasm32")]
fn apply_parallax(window: &web_sys::Window, el: &web_sys::HtmlElement) {
    let y = window.scroll_y().unwrap_or(0.0);
    // Ease through the hero: full pose by ~480px of scroll.
    let t = (y / 480.0).clamp(0.0, 1.0);
    // Smoothstep for a soft start/stop (cute, not mechanical).
    let e = t * t * (3.0 - 2.0 * t);
    let ty = -e * 64.0;
    // Drift sideways a little so it feels like a swim, not a lift.
    let tx = e * 14.0;
    let rot = -e * 9.0;
    let scale = 1.0 - e * 0.05;
    let _ = el.style().set_property(
        "transform",
        &format!("translate3d({tx:.2}px, {ty:.2}px, 0) rotate({rot:.2}deg) scale({scale:.4})"),
    );
}
