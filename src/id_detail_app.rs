//! The app's part of the detail country IDs (`id_detail.rs`): after the
//! view changes, make or drop the window for whatever fills the political
//! layer (today's countries or a year's polities). Deep time needs none:
//! its land mask and IDs come from the same raster, so they agree.

use crate::camera::Camera;
use crate::id_detail::{self, Filler, Plan};
use crate::App;

/// While the view moves, rebuild at most this often.
const MOVING_INTERVAL_MS: f64 = 150.0;

#[derive(Default)]
pub struct DetailState {
    /// The view and world raster last planned for, as (lat, lon, alt,
    /// bearing, width, height, projection id, world raster generation);
    /// planning again only when it changes.
    last: Option<[f64; 8]>,
    last_build_ms: f64,
}

fn key(c: &Camera, generation: u32) -> [f64; 8] {
    [
        c.lat,
        c.lon,
        c.alt,
        c.bearing,
        c.width,
        c.height,
        f64::from(c.proj.shader_index()),
        f64::from(generation),
    ]
}

impl App {
    /// Called every Earth frame before drawing.
    pub(crate) fn id_detail_frame(&mut self, moving: bool, now_ms: f64) {
        self.renderer.set_fills_always(self.paleo.on);
        if self.paleo.on {
            self.renderer.clear_id_detail();
            return;
        }
        let k = key(&self.camera, self.renderer.ids_generation());
        if self.detail.last == Some(k) {
            return;
        }
        if moving && now_ms - self.detail.last_build_ms < MOVING_INTERVAL_MS {
            return;
        }
        let world = self.renderer.ids_width();
        let plan = id_detail::plan(&self.camera, world, self.renderer.id_detail());
        self.detail.last = Some(k);
        let win = match plan {
            Plan::Keep => return,
            Plan::None => {
                self.renderer.clear_id_detail();
                return;
            }
            Plan::Make(w) => w,
        };
        let mut f = Filler::new(win);
        if self.history.on {
            if !self.history_fill_detail(&mut f) {
                return;
            }
        } else {
            let Some(c) = self.countries.as_ref().filter(|_| self.ids.is_some()) else {
                return;
            };
            c.fill_detail(&mut f);
        }
        if let Err(e) = self.renderer.upload_id_detail(win, &f.out) {
            web_sys::console::error_1(&e);
        }
        self.detail.last_build_ms = now_ms;
        self.redraw.mark_render();
    }
}
