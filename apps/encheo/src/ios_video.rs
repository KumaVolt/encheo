//! iOS: mpv draws (Vulkan through MoltenVK) into a Metal layer of its own, in a view placed behind
//! Slint's view. Slint has no OpenGL on iOS, so mpv can't draw into Slint's frame as it does
//! elsewhere; instead Slint's view is see-through and the player screen leaves the video visible.

use objc2::{MainThreadMarker, MainThreadOnly};
use objc2::rc::Retained;
use objc2_quartz_core::CAMetalLayer;
use objc2_ui_kit::{UIColor, UIView, UIViewAutoresizing};
use std::cell::RefCell;

struct Video {
    view: Retained<UIView>,
    layer: Retained<CAMetalLayer>,
}

thread_local! {
    static VIDEO: RefCell<Option<Video>> = const { RefCell::new(None) };
}

fn slint_view(window: &slint::Window) -> Option<Retained<UIView>> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use slint::winit_030::WinitWindowAccessor;
    window.with_winit_window(|w| match w.window_handle().ok()?.as_raw() {
        // SAFETY: winit's view, alive as long as the window.
        RawWindowHandle::UiKit(h) => unsafe { Retained::retain(h.ui_view.as_ptr().cast::<UIView>()) },
        _ => None,
    })?
}

/// Put the video view behind the UI (once) and return mpv's `wid`: the Metal layer's address.
pub fn attach(window: &slint::Window) -> Option<i64> {
    if let Some(wid) = VIDEO.with(|v| v.borrow().as_ref().map(|v| Retained::as_ptr(&v.layer) as i64)) {
        return Some(wid);
    }
    let mtm = MainThreadMarker::new()?;
    let ui = slint_view(window)?;
    let parent = ui.superview()?;
    let frame = ui.frame();
    let view = UIView::initWithFrame(UIView::alloc(mtm), frame);
    view.setAutoresizingMask(UIViewAutoresizing::FlexibleWidth | UIViewAutoresizing::FlexibleHeight);
    view.setBackgroundColor(Some(&UIColor::blackColor()));
    let layer = CAMetalLayer::new();
    layer.setContentsScale(ui.contentScaleFactor());
    layer.setFrame(view.bounds());
    view.layer().addSublayer(&layer);
    parent.insertSubview_belowSubview(&view, &ui);
    // Let the video show through Slint's view (and the Metal layer Slint draws into).
    ui.setOpaque(false);
    ui.setBackgroundColor(Some(&UIColor::clearColor()));
    ui.layer().setOpaque(false);
    if let Some(sublayers) = unsafe { ui.layer().sublayers() } {
        for l in sublayers.iter() {
            l.setOpaque(false);
        }
    }
    let wid = Retained::as_ptr(&layer) as i64;
    VIDEO.with(|v| *v.borrow_mut() = Some(Video { view, layer }));
    Some(wid)
}

pub fn attached() -> bool {
    VIDEO.with(|v| v.borrow().is_some())
}

/// Keep the Metal layer the size of its view (rotation, iPad multitasking).
pub fn follow_size() {
    VIDEO.with(|v| {
        if let Some(v) = v.borrow().as_ref() {
            let bounds = v.view.bounds();
            let f = v.layer.frame();
            if f.size.width != bounds.size.width || f.size.height != bounds.size.height {
                v.layer.setFrame(bounds);
            }
        }
    });
}
