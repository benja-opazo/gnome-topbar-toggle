use gtk::prelude::*;
use std::sync::{Arc, Mutex};

struct AnimData {
    start_y: f64,
    target_y: f64,
    start_time: std::time::Instant,
}

/// Smooth-scrolls a `ScrolledWindow` in response to wheel/touchpad events.
///
/// Chaining is supported: if a scroll event arrives while an animation is still
/// running, the new target is computed from the in-flight target rather than the
/// current scroll position, so rapid scrolling accumulates naturally.
#[derive(Clone)]
pub(crate) struct SmoothScroller {
    adj: gtk::Adjustment,
    active: Arc<Mutex<Option<glib::SourceId>>>,
    data: Arc<Mutex<AnimData>>,
}

impl SmoothScroller {
    pub(crate) fn new(adj: gtk::Adjustment) -> Self {
        Self {
            adj,
            active: Arc::new(Mutex::new(None)),
            data: Arc::new(Mutex::new(AnimData {
                start_y: 0.0,
                target_y: 0.0,
                start_time: std::time::Instant::now(),
            })),
        }
    }

    /// Scroll by `delta` units (positive = down). Chains onto the current
    /// in-flight target if an animation is already running.
    pub(crate) fn scroll_by(&self, delta: f64) {
        let mut active = self.active.lock().unwrap();
        let mut data = self.data.lock().unwrap();

        let current_val = self.adj.value();
        let base_y = if active.is_some() {
            data.target_y
        } else {
            current_val
        };
        let new_target = (base_y + delta * 160.0)
            .clamp(self.adj.lower(), self.adj.upper() - self.adj.page_size());

        *data = AnimData {
            start_y: current_val,
            target_y: new_target,
            start_time: std::time::Instant::now(),
        };

        if active.is_none() {
            let adj = self.adj.clone();
            let active_inner = Arc::clone(&self.active);
            let data_inner = Arc::clone(&self.data);

            let source_id =
                glib::timeout_add_local(std::time::Duration::from_millis(8), move || {
                    let (start_y, target_y, start_time) = {
                        let d = data_inner.lock().unwrap();
                        (d.start_y, d.target_y, d.start_time)
                    };
                    let t = (start_time.elapsed().as_millis() as f64 / 150.0).min(1.0);
                    let progress = 1.0 - (1.0 - t).powi(3); // cubic ease-out
                    adj.set_value(start_y + (target_y - start_y) * progress);

                    if t < 1.0 {
                        glib::ControlFlow::Continue
                    } else {
                        *active_inner.lock().unwrap() = None;
                        glib::ControlFlow::Break
                    }
                });
            *active = Some(source_id);
        }
    }

    /// Connects this scroller to the scroll events of a `ScrolledWindow`.
    pub(crate) fn attach(&self, scrolled: &gtk::ScrolledWindow) {
        let scroller = self.clone();
        scrolled.connect_scroll_event(move |_, event| {
            let (_, dy) = event.scroll_deltas().unwrap_or((0.0, 0.0));
            if dy == 0.0 {
                return glib::Propagation::Proceed;
            }
            scroller.scroll_by(dy);
            glib::Propagation::Stop
        });
    }
}

/// Animates a scroll to bring `target` into view within `container`, using a
/// quintic ease-out over 250 ms. The scroll position is read after a short
/// layout delay so that `translate_coordinates` returns a valid value.
pub(crate) fn scroll_to_widget(adj: gtk::Adjustment, target: gtk::Box, container: gtk::Box) {
    glib::timeout_add_local(std::time::Duration::from_millis(10), move || {
        if let Some((_, target_y)) = target.translate_coordinates(&container, 0, 0) {
            let start_y = adj.value();
            let end_y = target_y as f64;
            let distance = end_y - start_y;

            if distance.abs() < 1.0 {
                return glib::ControlFlow::Break;
            }

            let start_time = std::time::Instant::now();
            let adj_inner = adj.clone();

            glib::timeout_add_local(std::time::Duration::from_millis(10), move || {
                let t = start_time.elapsed().as_millis() as f64 / 250.0;
                if t >= 1.0 {
                    adj_inner.set_value(end_y);
                    return glib::ControlFlow::Break;
                }
                let progress = 1.0 - (1.0 - t).powi(5); // quintic ease-out
                adj_inner.set_value(start_y + distance * progress);
                glib::ControlFlow::Continue
            });
        }
        glib::ControlFlow::Break
    });
}
