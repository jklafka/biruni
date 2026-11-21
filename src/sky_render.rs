//! Draws the sky view (see `sky.rs` for the camera and conventions): a
//! full-screen pass of `shaders/sky.frag` over the shared quad vertex
//! shader, then the stars as additive points (`shaders/stars.*`). Built on
//! first use and dropped with the WebGL context; the star buffer is rebuilt
//! from the CPU copy the renderer keeps.

use crate::render::{compile, link, VERT_SRC};
use crate::sky::{to_gl, Mat3};
use wasm_bindgen::JsValue;
use web_sys::{
    WebGl2RenderingContext as GL, WebGlBuffer, WebGlProgram, WebGlTexture, WebGlUniformLocation,
    WebGlVertexArrayObject,
};

const SKY_FRAG_SRC: &str = include_str!(concat!(env!("OUT_DIR"), "/shaders/sky.frag"));
const STARS_VERT_SRC: &str = include_str!(concat!(env!("OUT_DIR"), "/shaders/stars.vert"));
const STARS_FRAG_SRC: &str = include_str!(concat!(env!("OUT_DIR"), "/shaders/stars.frag"));
const SURVEY_VERT_SRC: &str = include_str!(concat!(env!("OUT_DIR"), "/shaders/survey.vert"));
const SURVEY_FRAG_SRC: &str = include_str!(concat!(env!("OUT_DIR"), "/shaders/survey.frag"));

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
    /// 0 at night, 1 in full day.
    pub daylight: f64,
    /// How strongly the Milky Way image is added (when loaded).
    pub milky_way: f64,
    /// Equatorial -> camera, and the zenith in equatorial space.
    pub eq_to_cam: Mat3,
    pub zenith: [f64; 3],
    /// Faintest magnitude drawn, and how many stars (brightest first) that is.
    pub mag_limit: f32,
    pub star_count: usize,
    /// The deep-sky survey, when it is on.
    pub survey: Option<SurveyFrame>,
    /// A horizon and ground (the sky view); none in space (the solar view).
    pub ground: bool,
}

/// What of the survey to draw this frame.
pub struct SurveyFrame {
    /// 0..1, fading out by day.
    pub strength: f64,
    /// Atlas cells in view (`survey::allsky_indices`).
    pub allsky_indices: Vec<u16>,
    /// Streamed tiles to draw over the atlas, coarse first.
    pub tiles: Vec<crate::healpix::Tile>,
}

struct SurveyProgram {
    program: WebGlProgram,
    eq_to_cam: Option<WebGlUniformLocation>,
    res: Option<WebGlUniformLocation>,
    half_short: Option<WebGlUniformLocation>,
    tan_q: Option<WebGlUniformLocation>,
    tex: Option<WebGlUniformLocation>,
    zenith: Option<WebGlUniformLocation>,
    strength: Option<WebGlUniformLocation>,
    black: Option<WebGlUniformLocation>,
}

/// A mesh with its texture: the atlas (indices set per frame) or one tile.
struct SurveyMesh {
    tex: WebGlTexture,
    vao: WebGlVertexArrayObject,
    vbo: WebGlBuffer,
    ibo: WebGlBuffer,
    count: i32,
}

impl SurveyMesh {
    fn delete(&self, gl: &GL) {
        gl.delete_texture(Some(&self.tex));
        gl.delete_vertex_array(Some(&self.vao));
        gl.delete_buffer(Some(&self.vbo));
        gl.delete_buffer(Some(&self.ibo));
    }
}

/// Upload an image to a new texture on unit 0 (LINEAR, clamped; mipmapped
/// if asked).
fn image_texture(
    gl: &GL,
    bitmap: &web_sys::ImageBitmap,
    mips: bool,
) -> Result<WebGlTexture, JsValue> {
    let (w, h) = (bitmap.width(), bitmap.height());
    let tex = gl.create_texture().ok_or("failed to create texture")?;
    gl.active_texture(GL::TEXTURE0);
    gl.bind_texture(GL::TEXTURE_2D, Some(&tex));
    let levels = if mips {
        crate::imagery::mip_levels(w, h)
    } else {
        1
    };
    gl.tex_storage_2d(GL::TEXTURE_2D, levels, GL::RGBA8, w as i32, h as i32);
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
    let min = if mips {
        gl.generate_mipmap(GL::TEXTURE_2D);
        GL::LINEAR_MIPMAP_LINEAR
    } else {
        GL::LINEAR
    };
    gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MIN_FILTER, min as i32);
    gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MAG_FILTER, GL::LINEAR as i32);
    gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_WRAP_S, GL::CLAMP_TO_EDGE as i32);
    gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_WRAP_T, GL::CLAMP_TO_EDGE as i32);
    Ok(tex)
}

/// A VAO over interleaved direction (3 floats) and texture coordinates (2),
/// with an index buffer.
fn survey_mesh(
    gl: &GL,
    tex: WebGlTexture,
    verts: &[f32],
    indices: &[u16],
) -> Result<SurveyMesh, JsValue> {
    let vao = gl.create_vertex_array().ok_or("failed to create VAO")?;
    let vbo = gl.create_buffer().ok_or("failed to create buffer")?;
    let ibo = gl.create_buffer().ok_or("failed to create buffer")?;
    gl.bind_vertex_array(Some(&vao));
    gl.bind_buffer(GL::ARRAY_BUFFER, Some(&vbo));
    // SAFETY: the views are used only by these calls, before the slices can move.
    unsafe {
        let v = js_sys::Float32Array::view(verts);
        gl.buffer_data_with_array_buffer_view(GL::ARRAY_BUFFER, &v, GL::STATIC_DRAW);
    }
    gl.enable_vertex_attrib_array(0);
    gl.vertex_attrib_pointer_with_i32(0, 3, GL::FLOAT, false, 20, 0);
    gl.enable_vertex_attrib_array(1);
    gl.vertex_attrib_pointer_with_i32(1, 2, GL::FLOAT, false, 20, 12);
    gl.bind_buffer(GL::ELEMENT_ARRAY_BUFFER, Some(&ibo));
    unsafe {
        let i = js_sys::Uint16Array::view(indices);
        gl.buffer_data_with_array_buffer_view(GL::ELEMENT_ARRAY_BUFFER, &i, GL::DYNAMIC_DRAW);
    }
    gl.bind_vertex_array(None);
    Ok(SurveyMesh {
        tex,
        vao,
        vbo,
        ibo,
        count: indices.len() as i32,
    })
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
    ground: Option<WebGlUniformLocation>,
}

struct StarBuffer {
    vao: WebGlVertexArrayObject,
    buffer: WebGlBuffer,
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
    daylight: Option<WebGlUniformLocation>,
    milky_way_tex: Option<WebGlUniformLocation>,
    milky_way_on: Option<WebGlUniformLocation>,
    milky_way_texel: Option<WebGlUniformLocation>,
    ground: Option<WebGlUniformLocation>,
    /// The solar system view's bodies, drawn over this pass.
    pub(crate) bodies: crate::solar_render::BodyProgram,
    /// The Milky Way texture and its width, once uploaded.
    milky_way: Option<(WebGlTexture, u32)>,
    stars: StarProgram,
    star_buffer: Option<StarBuffer>,
    survey: SurveyProgram,
    allsky: Option<SurveyMesh>,
    survey_tiles: std::collections::HashMap<crate::healpix::Tile, SurveyMesh>,
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
                ground: su("u_ground"),
                program,
            }
        };
        let survey = {
            let vs = compile(gl, GL::VERTEX_SHADER, SURVEY_VERT_SRC)?;
            let fs = compile(gl, GL::FRAGMENT_SHADER, SURVEY_FRAG_SRC)?;
            let program = link(gl, &vs, &fs)?;
            let vu = |name: &str| gl.get_uniform_location(&program, name);
            SurveyProgram {
                eq_to_cam: vu("u_eqToCam"),
                res: vu("u_res"),
                half_short: vu("u_halfShort"),
                tan_q: vu("u_tanQ"),
                tex: vu("u_tex"),
                zenith: vu("u_zenith"),
                strength: vu("u_strength"),
                black: vu("u_black"),
                program,
            }
        };
        Ok(SkyPass {
            stars,
            star_buffer: None,
            survey,
            allsky: None,
            survey_tiles: std::collections::HashMap::new(),
            bounds: ku("u_bounds"),
            res: ku("u_res"),
            half_short: ku("u_halfShort"),
            tan_q: ku("u_tanQ"),
            cam_to_hor: ku("u_camToHor"),
            hor_to_eq: ku("u_horToEq"),
            dpr: ku("u_dpr"),
            grid_step: ku("u_gridStep"),
            daylight: ku("u_daylight"),
            milky_way_tex: ku("u_milkyWay"),
            milky_way_on: ku("u_milkyWayOn"),
            milky_way_texel: ku("u_milkyWayTexel"),
            ground: ku("u_ground"),
            milky_way: None,
            bodies: crate::solar_render::BodyProgram::create(gl)?,
            program,
        })
    }

    pub fn has_allsky(&self) -> bool {
        self.allsky.is_some()
    }

    /// Upload the survey's Allsky atlas (mipmapped) and its mesh.
    pub fn upload_allsky(&mut self, gl: &GL, bitmap: &web_sys::ImageBitmap) -> Result<(), JsValue> {
        use crate::survey::{allsky_mesh, ALLSKY_COLUMNS};
        let cell = bitmap.width() as f64 / ALLSKY_COLUMNS as f64;
        let rows = (bitmap.height() as f64 / cell).round().max(1.0) as u64;
        let tex = image_texture(gl, bitmap, true)?;
        let mesh = survey_mesh(gl, tex, &allsky_mesh(cell, rows), &[])?;
        if let Some(old) = self.allsky.replace(mesh) {
            old.delete(gl);
        }
        Ok(())
    }

    /// Upload one streamed survey tile and its mesh.
    pub fn upload_survey_tile(
        &mut self,
        gl: &GL,
        tile: crate::healpix::Tile,
        bitmap: &web_sys::ImageBitmap,
    ) -> Result<(), JsValue> {
        use crate::survey::{mesh, tile_uv, TILE_MESH};
        let tex = image_texture(gl, bitmap, false)?;
        let (mut v, mut i) = (Vec::new(), Vec::new());
        mesh(tile, TILE_MESH, 0, tile_uv, &mut v, &mut i);
        let m = survey_mesh(gl, tex, &v, &i)?;
        if let Some(old) = self.survey_tiles.insert(tile, m) {
            old.delete(gl);
        }
        Ok(())
    }

    pub fn drop_survey_tile(&mut self, gl: &GL, tile: crate::healpix::Tile) {
        if let Some(m) = self.survey_tiles.remove(&tile) {
            m.delete(gl);
        }
    }

    fn draw_survey(&self, gl: &GL, f: &SkyFrame, s: &SurveyFrame) {
        let Some(atlas) = &self.allsky else {
            return;
        };
        let p = &self.survey;
        gl.use_program(Some(&p.program));
        gl.uniform_matrix3fv_with_f32_array(p.eq_to_cam.as_ref(), false, &to_gl(&f.eq_to_cam));
        gl.uniform2f(p.res.as_ref(), f.width as f32, f.height as f32);
        gl.uniform1f(p.half_short.as_ref(), f.half_short as f32);
        gl.uniform1f(p.tan_q.as_ref(), f.tan_quarter as f32);
        let z = f.zenith;
        gl.uniform3f(p.zenith.as_ref(), z[0] as f32, z[1] as f32, z[2] as f32);
        gl.uniform1f(p.strength.as_ref(), s.strength as f32);
        gl.uniform1f(p.black.as_ref(), crate::survey::BLACK);
        gl.uniform1i(p.tex.as_ref(), 0);
        gl.active_texture(GL::TEXTURE0);
        // Each part of the sky is added once, from the finest layer there:
        // tiles finest first, then the atlas, each only where the stencil
        // is still 0, and each fragment drawn sets it.
        gl.enable(GL::BLEND);
        gl.blend_func(GL::ONE, GL::ONE);
        gl.enable(GL::STENCIL_TEST);
        gl.clear_stencil(0);
        gl.clear(GL::STENCIL_BUFFER_BIT);
        gl.stencil_func(GL::EQUAL, 0, 0xff);
        gl.stencil_op(GL::KEEP, GL::KEEP, GL::INCR);
        for t in s.tiles.iter().rev() {
            if let Some(m) = self.survey_tiles.get(t) {
                gl.bind_vertex_array(Some(&m.vao));
                gl.bind_texture(GL::TEXTURE_2D, Some(&m.tex));
                gl.draw_elements_with_i32(GL::TRIANGLES, m.count, GL::UNSIGNED_SHORT, 0);
            }
        }
        if !s.allsky_indices.is_empty() {
            gl.bind_vertex_array(Some(&atlas.vao));
            gl.bind_buffer(GL::ELEMENT_ARRAY_BUFFER, Some(&atlas.ibo));
            // SAFETY: the view is used only by this call.
            unsafe {
                let i = js_sys::Uint16Array::view(&s.allsky_indices);
                gl.buffer_data_with_array_buffer_view(
                    GL::ELEMENT_ARRAY_BUFFER,
                    &i,
                    GL::DYNAMIC_DRAW,
                );
            }
            gl.bind_texture(GL::TEXTURE_2D, Some(&atlas.tex));
            gl.draw_elements_with_i32(
                GL::TRIANGLES,
                s.allsky_indices.len() as i32,
                GL::UNSIGNED_SHORT,
                0,
            );
        }
        gl.disable(GL::STENCIL_TEST);
        gl.bind_vertex_array(None);
        gl.disable(GL::BLEND);
    }

    /// Whether the Milky Way image is on the GPU.
    pub fn has_milky_way(&self) -> bool {
        self.milky_way.is_some()
    }

    /// Upload the Milky Way image (RGBA8, mipmapped, wrapping in RA) to
    /// texture unit 7, which no other pass uses.
    pub fn upload_milky_way(
        &mut self,
        gl: &GL,
        bitmap: &web_sys::ImageBitmap,
    ) -> Result<(), JsValue> {
        let (w, h) = (bitmap.width(), bitmap.height());
        let tex = gl.create_texture().ok_or("failed to create texture")?;
        gl.active_texture(GL::TEXTURE7);
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
        let uploaded = gl.tex_sub_image_2d_with_u32_and_u32_and_image_bitmap(
            GL::TEXTURE_2D,
            0,
            0,
            0,
            GL::RGBA,
            GL::UNSIGNED_BYTE,
            bitmap,
        );
        if let Err(e) = uploaded {
            gl.delete_texture(Some(&tex));
            gl.active_texture(GL::TEXTURE0);
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
        gl.active_texture(GL::TEXTURE0);
        self.milky_way = Some((tex, w));
        Ok(())
    }

    /// Drop the star buffer, so new vertices can be uploaded.
    pub fn clear_stars(&mut self, gl: &GL) {
        if let Some(b) = self.star_buffer.take() {
            gl.delete_vertex_array(Some(&b.vao));
            gl.delete_buffer(Some(&b.buffer));
        }
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
            buffer,
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
        gl.uniform1f(self.daylight.as_ref(), f.daylight as f32);
        let ground = if f.ground { 1.0 } else { 0.0 };
        gl.uniform1f(self.ground.as_ref(), ground);
        match &self.milky_way {
            Some((tex, w)) => {
                gl.active_texture(GL::TEXTURE7);
                gl.bind_texture(GL::TEXTURE_2D, Some(tex));
                gl.active_texture(GL::TEXTURE0);
                gl.uniform1f(self.milky_way_on.as_ref(), f.milky_way as f32);
                let texel = std::f64::consts::TAU / *w as f64;
                gl.uniform1f(self.milky_way_texel.as_ref(), texel as f32);
            }
            None => gl.uniform1f(self.milky_way_on.as_ref(), 0.0),
        }
        // The sampler always points at unit 7 (nothing else is bound there
        // before the image arrives, and the strength is then 0).
        gl.uniform1i(self.milky_way_tex.as_ref(), 7);
        gl.draw_arrays(GL::TRIANGLE_STRIP, 0, 4);
        if let Some(s) = &f.survey {
            self.draw_survey(gl, f, s);
        }

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
        gl.uniform1f(sp.ground.as_ref(), ground);
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
            (include_str!("shaders/survey.vert"), "vu", 4),
            (include_str!("shaders/survey.frag"), "vu", 4),
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
