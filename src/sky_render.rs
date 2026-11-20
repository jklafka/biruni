//! Draws the sky view (see `sky.rs` for the camera and conventions): a
//! full-screen pass of `shaders/sky.frag` over the shared quad vertex
//! shader, then the stars as additive points (`shaders/stars.*`). Built on
//! first use and dropped with the WebGL context; the star buffer is rebuilt
//! from the CPU copy the renderer keeps.

use crate::render::{compile, link, VERT_SRC};
use crate::sky::{to_gl, Mat3};
use wasm_bindgen::JsValue;
use web_sys::{
    WebGl2RenderingContext as GL, WebGlBuffer, WebGlProgram, WebGlUniformLocation,
    WebGlVertexArrayObject,
};

const SKY_FRAG_SRC: &str = include_str!(concat!(env!("OUT_DIR"), "/shaders/sky.frag"));
const STARS_VERT_SRC: &str = include_str!(concat!(env!("OUT_DIR"), "/shaders/stars.vert"));
const STARS_FRAG_SRC: &str = include_str!(concat!(env!("OUT_DIR"), "/shaders/stars.frag"));

/// Everything one sky frame needs, in device pixels.
pub struct SkyFrame {
    pub width: u32,
    pub height: u32,
    pub half_short: f64,
    pub tan_quarter: f64,
    pub cam_to_hor: Mat3,
    pub hor_to_eq: Mat3,
    pub dpr: f64,
    /// Radians between grid lines, 0 for none.
    pub grid_step: f64,
    /// Equatorial -> camera, and the zenith in equatorial space.
    pub eq_to_cam: Mat3,
    pub zenith: [f64; 3],
    /// Faintest magnitude drawn, and how many stars (brightest first) that is.
    pub mag_limit: f32,
    pub star_count: usize,
}

struct StarProgram {
    program: WebGlProgram,
    eq_to_cam: Option<WebGlUniformLocation>,
    zenith: Option<WebGlUniformLocation>,
    res: Option<WebGlUniformLocation>,
    half_short: Option<WebGlUniformLocation>,
    tan_q: Option<WebGlUniformLocation>,
    dpr: Option<WebGlUniformLocation>,
    mag_limit: Option<WebGlUniformLocation>,
}

struct StarBuffer {
    vao: WebGlVertexArrayObject,
    _buffer: WebGlBuffer,
    len: usize,
}

pub struct SkyPass {
    program: WebGlProgram,
    bounds: Option<WebGlUniformLocation>,
    res: Option<WebGlUniformLocation>,
    half_short: Option<WebGlUniformLocation>,
    tan_q: Option<WebGlUniformLocation>,
    cam_to_hor: Option<WebGlUniformLocation>,
    hor_to_eq: Option<WebGlUniformLocation>,
    dpr: Option<WebGlUniformLocation>,
    grid_step: Option<WebGlUniformLocation>,
    stars: StarProgram,
    star_buffer: Option<StarBuffer>,
}

impl SkyPass {
    pub fn create(gl: &GL) -> Result<SkyPass, JsValue> {
        let vs = compile(gl, GL::VERTEX_SHADER, VERT_SRC)?;
        let fs = compile(gl, GL::FRAGMENT_SHADER, SKY_FRAG_SRC)?;
        let program = link(gl, &vs, &fs)?;
        let ku = |name: &str| gl.get_uniform_location(&program, name);
        let stars = {
            let vs = compile(gl, GL::VERTEX_SHADER, STARS_VERT_SRC)?;
            let fs = compile(gl, GL::FRAGMENT_SHADER, STARS_FRAG_SRC)?;
            let program = link(gl, &vs, &fs)?;
            let su = |name: &str| gl.get_uniform_location(&program, name);
            StarProgram {
                eq_to_cam: su("u_eqToCam"),
                zenith: su("u_zenith"),
                res: su("u_res"),
                half_short: su("u_halfShort"),
                tan_q: su("u_tanQ"),
                dpr: su("u_dpr"),
                mag_limit: su("u_magLimit"),
                program,
            }
        };
        Ok(SkyPass {
            stars,
            star_buffer: None,
            bounds: ku("u_bounds"),
            res: ku("u_res"),
            half_short: ku("u_halfShort"),
            tan_q: ku("u_tanQ"),
            cam_to_hor: ku("u_camToHor"),
            hor_to_eq: ku("u_horToEq"),
            dpr: ku("u_dpr"),
            grid_step: ku("u_gridStep"),
            program,
        })
    }

    /// Whether the star vertices are on the GPU.
    pub fn has_stars(&self) -> bool {
        self.star_buffer.is_some()
    }

    /// Upload star vertices (`stars::vertices`: [`crate::stars::STRIDE`]
    /// floats each).
    pub fn upload_stars(&mut self, gl: &GL, data: &[f32]) -> Result<(), JsValue> {
        use crate::stars::STRIDE;
        let vao = gl
            .create_vertex_array()
            .ok_or("failed to create star VAO")?;
        let buffer = gl.create_buffer().ok_or("failed to create star buffer")?;
        gl.bind_vertex_array(Some(&vao));
        gl.bind_buffer(GL::ARRAY_BUFFER, Some(&buffer));
        // SAFETY: the view is only used for this call, before `data` can move.
        unsafe {
            let view = js_sys::Float32Array::view(data);
            gl.buffer_data_with_array_buffer_view(GL::ARRAY_BUFFER, &view, GL::STATIC_DRAW);
        }
        let stride = (STRIDE * 4) as i32;
        for (loc, size, offset) in [(0, 3, 0), (1, 1, 12), (2, 3, 16)] {
            gl.enable_vertex_attrib_array(loc);
            gl.vertex_attrib_pointer_with_i32(loc, size, GL::FLOAT, false, stride, offset);
        }
        gl.bind_vertex_array(None);
        self.star_buffer = Some(StarBuffer {
            vao,
            _buffer: buffer,
            len: data.len() / STRIDE,
        });
        Ok(())
    }

    pub fn draw(&self, gl: &GL, f: &SkyFrame) {
        gl.bind_framebuffer(GL::FRAMEBUFFER, None);
        gl.bind_vertex_array(None);
        gl.viewport(0, 0, f.width as i32, f.height as i32);
        gl.disable(GL::BLEND);
        gl.use_program(Some(&self.program));
        gl.uniform4f(self.bounds.as_ref(), -1.0, -1.0, 1.0, 1.0);
        gl.uniform2f(self.res.as_ref(), f.width as f32, f.height as f32);
        gl.uniform1f(self.half_short.as_ref(), f.half_short as f32);
        gl.uniform1f(self.tan_q.as_ref(), f.tan_quarter as f32);
        gl.uniform_matrix3fv_with_f32_array(self.cam_to_hor.as_ref(), false, &to_gl(&f.cam_to_hor));
        gl.uniform_matrix3fv_with_f32_array(self.hor_to_eq.as_ref(), false, &to_gl(&f.hor_to_eq));
        gl.uniform1f(self.dpr.as_ref(), f.dpr as f32);
        gl.uniform1f(self.grid_step.as_ref(), f.grid_step as f32);
        gl.draw_arrays(GL::TRIANGLE_STRIP, 0, 4);

        let Some(buf) = &self.star_buffer else {
            return;
        };
        let count = f.star_count.min(buf.len);
        if count == 0 {
            return;
        }
        let sp = &self.stars;
        gl.use_program(Some(&sp.program));
        gl.uniform_matrix3fv_with_f32_array(sp.eq_to_cam.as_ref(), false, &to_gl(&f.eq_to_cam));
        let z = f.zenith;
        gl.uniform3f(sp.zenith.as_ref(), z[0] as f32, z[1] as f32, z[2] as f32);
        gl.uniform2f(sp.res.as_ref(), f.width as f32, f.height as f32);
        gl.uniform1f(sp.half_short.as_ref(), f.half_short as f32);
        gl.uniform1f(sp.tan_q.as_ref(), f.tan_quarter as f32);
        gl.uniform1f(sp.dpr.as_ref(), f.dpr as f32);
        gl.uniform1f(sp.mag_limit.as_ref(), f.mag_limit);
        gl.enable(GL::BLEND);
        gl.blend_func(GL::ONE, GL::ONE);
        gl.bind_vertex_array(Some(&buf.vao));
        gl.draw_arrays(GL::POINTS, 0, count as i32);
        gl.bind_vertex_array(None);
        gl.disable(GL::BLEND);
    }
}

#[cfg(test)]
mod tests {
    fn uniforms(src: &str) -> Vec<&str> {
        src.lines()
            .filter(|l| l.starts_with("uniform "))
            .filter_map(|l| l.split(';').next()?.split_whitespace().last())
            .collect()
    }

    #[test]
    fn every_uniform_is_declared_and_looked_up() {
        let rust = include_str!("sky_render.rs");
        for (src, prefix, min) in [
            (include_str!("shaders/sky.frag"), "ku", 7),
            (include_str!("shaders/stars.vert"), "su", 7),
        ] {
            let names = uniforms(src);
            assert!(names.len() >= min, "{names:?}");
            for name in names {
                assert!(
                    rust.contains(&format!("{prefix}(\"{name}\")")),
                    "{name} not looked up"
                );
            }
        }
        assert!(rust.contains("ku(\"u_bounds\")"));
    }

    /// The vertex layout in `upload_stars` matches the shader's locations
    /// and `stars::vertices`.
    #[test]
    fn star_attributes_match_the_layout() {
        let src = include_str!("shaders/stars.vert");
        for decl in [
            "layout(location = 0) in vec3 a_dir;",
            "layout(location = 1) in float a_mag;",
            "layout(location = 2) in vec3 a_color;",
        ] {
            assert!(src.contains(decl), "{decl}");
        }
        assert_eq!(crate::stars::STRIDE, 3 + 1 + 3);
    }
}
