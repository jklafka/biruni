//! Draws the bodies of the solar system view (`shaders/body.frag`, one
//! screen-space quad per body over the shared quad vertex shader), after
//! the sky pass has drawn the stars and the Milky Way behind them. Part of
//! `SkyPass`, so it is built and dropped with it. What it gets per body is
//! camera-relative (`solar_scene::Drawn`).

use crate::render::{compile, link, VERT_SRC};
use crate::solar::Body;
use crate::solar_scene::{color, Drawn};
use std::collections::HashMap;
use wasm_bindgen::JsValue;
use web_sys::{WebGl2RenderingContext as GL, WebGlProgram, WebGlTexture, WebGlUniformLocation};

/// Surface maps kept on the GPU at once (each 2048x1024 RGBA with mipmaps
/// is about 11 MB; 2.8 MB on the small tier at 1024x512). The least
/// recently drawn one goes first; the app decodes it again when needed.
pub const MAX_MAPS: usize = 2;

const BODY_FRAG_SRC: &str = include_str!(concat!(env!("OUT_DIR"), "/shaders/body.frag"));

pub struct BodyProgram {
    program: WebGlProgram,
    bounds: Option<WebGlUniformLocation>,
    res: Option<WebGlUniformLocation>,
    half_short: Option<WebGlUniformLocation>,
    tan_q: Option<WebGlUniformLocation>,
    center: Option<WebGlUniformLocation>,
    sun: Option<WebGlUniformLocation>,
    pole: Option<WebGlUniformLocation>,
    color: Option<WebGlUniformLocation>,
    emissive: Option<WebGlUniformLocation>,
    bands: Option<WebGlUniformLocation>,
    rings: Option<WebGlUniformLocation>,
    dot: Option<WebGlUniformLocation>,
    dpr: Option<WebGlUniformLocation>,
    meridian: Option<WebGlUniformLocation>,
    map: Option<WebGlUniformLocation>,
    map_on: Option<WebGlUniformLocation>,
    /// Uploaded surface maps and the frame each was last drawn in.
    maps: HashMap<Body, (WebGlTexture, u64)>,
    frame: u64,
    /// A 1x1 texture for the sampler of bodies without a map.
    blank: WebGlTexture,
}

/// Screen state for drawing the bodies, device pixels.
pub struct BodyFrame {
    pub width: u32,
    pub height: u32,
    pub half_short: f64,
    pub tan_quarter: f64,
    pub dpr: f64,
}

impl BodyProgram {
    pub fn create(gl: &GL) -> Result<BodyProgram, JsValue> {
        let vs = compile(gl, GL::VERTEX_SHADER, VERT_SRC)?;
        let fs = compile(gl, GL::FRAGMENT_SHADER, BODY_FRAG_SRC)?;
        let program = link(gl, &vs, &fs)?;
        let bu = |name: &str| gl.get_uniform_location(&program, name);
        Ok(BodyProgram {
            bounds: bu("u_bounds"),
            res: bu("u_res"),
            half_short: bu("u_halfShort"),
            tan_q: bu("u_tanQ"),
            center: bu("u_center"),
            sun: bu("u_sun"),
            pole: bu("u_pole"),
            color: bu("u_color"),
            emissive: bu("u_emissive"),
            bands: bu("u_bands"),
            rings: bu("u_rings"),
            dot: bu("u_dot"),
            dpr: bu("u_dpr"),
            meridian: bu("u_meridian"),
            map: bu("u_map"),
            map_on: bu("u_mapOn"),
            maps: HashMap::new(),
            frame: 0,
            blank: {
                let t = gl.create_texture().ok_or("failed to create texture")?;
                gl.active_texture(GL::TEXTURE0);
                gl.bind_texture(GL::TEXTURE_2D, Some(&t));
                gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
                    GL::TEXTURE_2D,
                    0,
                    GL::RGBA8 as i32,
                    1,
                    1,
                    0,
                    GL::RGBA,
                    GL::UNSIGNED_BYTE,
                    Some(&[0, 0, 0, 255]),
                )?;
                gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MIN_FILTER, GL::NEAREST as i32);
                gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MAG_FILTER, GL::NEAREST as i32);
                t
            },
            program,
        })
    }

    pub fn has_map(&self, body: Body) -> bool {
        self.maps.contains_key(&body)
    }

    /// Upload a body's surface map (mipmapped, repeating in longitude),
    /// dropping the least recently drawn map beyond [`MAX_MAPS`].
    pub fn upload_map(
        &mut self,
        gl: &GL,
        body: Body,
        bitmap: &web_sys::ImageBitmap,
    ) -> Result<(), JsValue> {
        let (w, h) = (bitmap.width(), bitmap.height());
        let tex = gl.create_texture().ok_or("failed to create texture")?;
        gl.active_texture(GL::TEXTURE0);
        gl.bind_texture(GL::TEXTURE_2D, Some(&tex));
        gl.tex_storage_2d(
            GL::TEXTURE_2D,
            crate::imagery::mip_levels(w, h),
            GL::RGBA8,
            w as i32,
            h as i32,
        );
        gl.pixel_storei(GL::UNPACK_FLIP_Y_WEBGL, 0);
        gl.pixel_storei(GL::UNPACK_PREMULTIPLY_ALPHA_WEBGL, 0);
        if let Err(e) = gl.tex_sub_image_2d_with_u32_and_u32_and_image_bitmap(
            GL::TEXTURE_2D,
            0,
            0,
            0,
            GL::RGBA,
            GL::UNSIGNED_BYTE,
            bitmap,
        ) {
            gl.delete_texture(Some(&tex));
            return Err(e);
        }
        gl.generate_mipmap(GL::TEXTURE_2D);
        gl.tex_parameteri(
            GL::TEXTURE_2D,
            GL::TEXTURE_MIN_FILTER,
            GL::LINEAR_MIPMAP_LINEAR as i32,
        );
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MAG_FILTER, GL::LINEAR as i32);
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_WRAP_S, GL::REPEAT as i32);
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_WRAP_T, GL::CLAMP_TO_EDGE as i32);
        if let Some((old, _)) = self.maps.insert(body, (tex, self.frame)) {
            gl.delete_texture(Some(&old));
        }
        while self.maps.len() > MAX_MAPS {
            let oldest = self
                .maps
                .iter()
                .filter(|(b, _)| **b != body)
                .min_by_key(|(_, (_, used))| *used)
                .map(|(b, _)| *b);
            match oldest.and_then(|b| self.maps.remove(&b)) {
                Some((t, _)) => gl.delete_texture(Some(&t)),
                None => break,
            }
        }
        Ok(())
    }

    /// Draw `bodies` (farthest first) over what is on screen.
    pub fn draw(&mut self, gl: &GL, f: &BodyFrame, bodies: &[Drawn]) {
        if bodies.is_empty() {
            return;
        }
        self.frame += 1;
        gl.bind_framebuffer(GL::FRAMEBUFFER, None);
        gl.viewport(0, 0, f.width as i32, f.height as i32);
        gl.use_program(Some(&self.program));
        gl.bind_vertex_array(None);
        gl.uniform2f(self.res.as_ref(), f.width as f32, f.height as f32);
        gl.uniform1f(self.half_short.as_ref(), f.half_short as f32);
        gl.uniform1f(self.tan_q.as_ref(), f.tan_quarter as f32);
        gl.uniform1f(self.dpr.as_ref(), f.dpr as f32);
        gl.enable(GL::BLEND);
        gl.blend_func(GL::ONE, GL::ONE_MINUS_SRC_ALPHA);
        let (w, h) = (f.width as f64, f.height as f64);
        for b in bodies {
            let dpr = f.dpr;
            // Quad around everything the body touches, in NDC, a few pixels
            // wider for the antialiased edge and the dot.
            let reach = (b.reach.r.max(crate::solar_scene::DOT_RADIUS) + 2.0) * dpr;
            let (cx, cy) = (b.reach.x * dpr, h - b.reach.y * dpr);
            let ndc = |v: f64, size: f64| (v / size * 2.0 - 1.0).clamp(-1.0, 1.0) as f32;
            gl.uniform4f(
                self.bounds.as_ref(),
                ndc(cx - reach, w),
                ndc(cy - reach, h),
                ndc(cx + reach, w),
                ndc(cy + reach, h),
            );
            let v3 = |loc: &Option<WebGlUniformLocation>, v: [f32; 3]| {
                gl.uniform3f(loc.as_ref(), v[0], v[1], v[2]);
            };
            v3(&self.center, b.center);
            v3(&self.sun, b.sun);
            v3(&self.pole, b.pole);
            v3(&self.color, color(b.body));
            let sun = b.body == crate::solar::Body::Sun;
            gl.uniform1f(self.emissive.as_ref(), if sun { 1.0 } else { 0.0 });
            let bands = match b.body {
                crate::solar::Body::Jupiter => 0.09,
                crate::solar::Body::Saturn => 0.05,
                _ => 0.0,
            };
            gl.uniform1f(self.bands.as_ref(), bands);
            let (ri, ro) = b.rings.unwrap_or((0.0, 0.0));
            gl.uniform2f(self.rings.as_ref(), ri, ro);
            v3(&self.meridian, b.meridian);
            gl.uniform1i(self.map.as_ref(), 0);
            gl.active_texture(GL::TEXTURE0);
            let frame = self.frame;
            match self.maps.get_mut(&b.body) {
                Some((tex, used)) => {
                    *used = frame;
                    gl.bind_texture(GL::TEXTURE_2D, Some(tex));
                    gl.uniform1f(self.map_on.as_ref(), 1.0);
                }
                None => {
                    gl.bind_texture(GL::TEXTURE_2D, Some(&self.blank));
                    gl.uniform1f(self.map_on.as_ref(), 0.0);
                }
            }
            gl.uniform3f(
                self.dot.as_ref(),
                (b.disk.x * dpr) as f32,
                (h - b.disk.y * dpr) as f32,
                (b.disk.r * dpr) as f32,
            );
            gl.draw_arrays(GL::TRIANGLE_STRIP, 0, 4);
        }
        gl.disable(GL::BLEND);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_uniform_is_declared_and_looked_up() {
        let rust = include_str!("solar_render.rs");
        let src = include_str!("shaders/body.frag");
        let names: Vec<&str> = src
            .lines()
            .filter(|l| l.starts_with("uniform "))
            .filter_map(|l| l.split(';').next()?.split_whitespace().last())
            .collect();
        assert!(names.len() >= 12, "{names:?}");
        for name in names {
            assert!(rust.contains(&format!("bu(\"{name}\")")), "{name}");
        }
        assert!(rust.contains("bu(\"u_bounds\")"));
    }
}
