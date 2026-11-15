//! WebGL2 renderer: one full-screen triangle and a ray-tracing fragment shader.

use crate::camera::{Camera, TAN_HALF_FOV};
use crate::geo::Layers;
use crate::lines::{lod_for, LineLevels, LineSet};
use crate::projection::{self, Blend, Projection, FLAT_ZOOM};
use crate::quality::scaled_size;
use crate::view::Layer;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    HtmlCanvasElement, WebGl2RenderingContext as GL, WebGlBuffer, WebGlFramebuffer, WebGlProgram,
    WebGlShader, WebGlTexture, WebGlUniformLocation, WebGlVertexArrayObject,
};

const VERT_SRC: &str = include_str!(concat!(env!("OUT_DIR"), "/shaders/globe.vert"));
const FRAG_SRC: &str = include_str!(concat!(env!("OUT_DIR"), "/shaders/globe.frag"));
const SPACE_FRAG_SRC: &str = include_str!(concat!(env!("OUT_DIR"), "/shaders/space.frag"));
const BLIT_FRAG_SRC: &str = include_str!(concat!(env!("OUT_DIR"), "/shaders/blit.frag"));
const MORPH_VERT_SRC: &str = include_str!(concat!(env!("OUT_DIR"), "/shaders/morph.vert"));
const LINE_VERT_SRC: &str = include_str!(concat!(env!("OUT_DIR"), "/shaders/line.vert"));
const LINE_FRAG_SRC: &str = include_str!(concat!(env!("OUT_DIR"), "/shaders/line.frag"));

/// Line widths in CSS pixels (multiplied by the device pixel ratio).
const COAST_WIDTH: f32 = 1.0;
const BORDER_WIDTH: f32 = 1.25;

// EXT_texture_filter_anisotropic
const TEXTURE_MAX_ANISOTROPY_EXT: u32 = 0x84FE;
const MAX_TEXTURE_MAX_ANISOTROPY_EXT: u32 = 0x84FF;

/// What the texture-size choice depends on. Gathered from the browser by
/// [`Renderer::detect_device`]; kept plain so the policy is unit testable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeviceInfo {
    /// `gl.MAX_TEXTURE_SIZE`.
    pub max_texture_size: u32,
    /// `navigator.deviceMemory` in GB (Chrome only, capped at 8).
    pub device_memory_gb: Option<f64>,
    /// Primary pointer is coarse (a touch screen: phones and tablets).
    pub coarse_pointer: bool,
}

/// Forces a tier regardless of the device (`?tex=low` / `?tex=high`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierOverride {
    Low,
    High,
}

/// Sizes of the two GL textures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureBudget {
    /// Map texture width (height is half). R8 with a full mip chain.
    pub map_width: u32,
    /// Country-ID raster width (height is half). R8, no mipmaps.
    pub ids_width: usize,
    /// Satellite imagery width (height is half). RGBA8 with a full mip
    /// chain, allocated only when the Imagery layer is first selected.
    pub imagery_width: u32,
}

/// Devices with at most this much RAM take the small tier.
const LOW_MEMORY_GB: f64 = 4.0;

/// Pick texture sizes. Capable desktops keep 8192 (crisp coastlines at the
/// closest zoom). Touch devices, low-memory devices and GPUs limited to 4096
/// get a 4096 map (about 11 MB with mips) and a 2048 ID raster (2 MB): at
/// phone resolutions a 4096-wide map still has more texels than the globe
/// has screen pixels at the default zoom. Both are clamped to the GPU limit.
pub fn choose_budget(d: &DeviceInfo, forced: Option<TierOverride>) -> TextureBudget {
    let low = match forced {
        Some(TierOverride::Low) => true,
        Some(TierOverride::High) => false,
        None => {
            d.coarse_pointer
                || d.max_texture_size < 8192
                || d.device_memory_gb.is_some_and(|m| m <= LOW_MEMORY_GB)
        }
    };
    let (map, ids, imagery) = if low {
        (4096, 2048, 2048)
    } else {
        (8192, 4096, 4096)
    };
    // Largest power of two the GPU supports (at least 1024).
    let cap = if d.max_texture_size >= 1024 {
        1 << (31 - d.max_texture_size.leading_zeros())
    } else {
        1024
    };
    TextureBudget {
        map_width: map.min(cap),
        ids_width: (ids as u32).min(cap) as usize,
        imagery_width: imagery.min(cap),
    }
}

/// GPU bytes for the satellite imagery (RGBA8, full mip chain), which exists
/// only once the Imagery layer has been opened: 44.7 MB at 4096 wide, 11.2 MB
/// at 2048 wide.
pub fn imagery_bytes(b: &TextureBudget) -> u64 {
    let (mut w, mut h) = (b.imagery_width as u64, b.imagery_width as u64 / 2);
    let mut total = 0;
    loop {
        total += w * h * 4;
        if w == 1 && h == 1 {
            break;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    total
}

/// GPU bytes for the map texture (R8, full mip chain) and the ID raster
/// (R8). The mip chain adds a third of the base level.
pub fn budget_bytes(b: &TextureBudget) -> (u64, u64) {
    let (w, h) = (b.map_width as u64, b.map_width as u64 / 2);
    let mut map = 0;
    let (mut lw, mut lh) = (w, h);
    loop {
        map += lw * lh;
        if lw == 1 && lh == 1 {
            break;
        }
        lw = (lw / 2).max(1);
        lh = (lh / 2).max(1);
    }
    let ids = (b.ids_width as u64) * (b.ids_width as u64 / 2);
    (map, ids)
}

/// Parse the value of the `tex` query parameter.
pub fn parse_tier(search: &str) -> Option<TierOverride> {
    search
        .trim_start_matches('?')
        .split('&')
        .find_map(|kv| match kv {
            "tex=low" => Some(TierOverride::Low),
            "tex=high" => Some(TierOverride::High),
            _ => None,
        })
}

/// True if the `debug=ctxloss` query flag is present. It enables the hidden
/// context-loss key (see [`Renderer::debug_toggle_context_loss`]).
pub fn parse_debug_ctxloss(search: &str) -> bool {
    search
        .trim_start_matches('?')
        .split('&')
        .any(|kv| kv == "debug=ctxloss")
}

struct Uniforms {
    res: Option<WebGlUniformLocation>,
    dist: Option<WebGlUniformLocation>,
    tan_half_fov: Option<WebGlUniformLocation>,
    rot: Option<WebGlUniformLocation>,
    map: Option<WebGlUniformLocation>,
    lines: Option<WebGlUniformLocation>,
    has_map: Option<WebGlUniformLocation>,
    grid_step: Option<WebGlUniformLocation>,
    dpr: Option<WebGlUniformLocation>,
    layer: Option<WebGlUniformLocation>,
    imagery: Option<WebGlUniformLocation>,
    has_imagery: Option<WebGlUniformLocation>,
    ids: Option<WebGlUniformLocation>,
    highlight: Option<WebGlUniformLocation>,
    id_color: Option<WebGlUniformLocation>,
    political: Option<WebGlUniformLocation>,
    sun: Option<WebGlUniformLocation>,
    sun_on: Option<WebGlUniformLocation>,
    bounds: Option<WebGlUniformLocation>,
    scale: Option<WebGlUniformLocation>,
    mode: Option<WebGlUniformLocation>,
    proj: Option<WebGlUniformLocation>,
    flat_px: Option<WebGlUniformLocation>,
    cy: Option<WebGlUniformLocation>,
    lon0: Option<WebGlUniformLocation>,
    globe: Option<WebGlUniformLocation>,
    // Only in morph.vert.
    proj_a: Option<WebGlUniformLocation>,
    proj_b: Option<WebGlUniformLocation>,
    blend_mix: Option<WebGlUniformLocation>,
    cy_ab: Option<WebGlUniformLocation>,
}

impl Uniforms {
    /// Locations in `program`; those it does not declare are `None`.
    fn lookup(gl: &GL, program: &WebGlProgram) -> Self {
        let u = |name: &str| gl.get_uniform_location(program, name);
        Uniforms {
            res: u("u_res"),
            dist: u("u_dist"),
            tan_half_fov: u("u_tanHalfFov"),
            rot: u("u_rot"),
            map: u("u_map"),
            lines: u("u_lines"),
            has_map: u("u_hasMap"),
            grid_step: u("u_gridStep"),
            dpr: u("u_dpr"),
            layer: u("u_layer"),
            imagery: u("u_imagery"),
            has_imagery: u("u_hasImagery"),
            ids: u("u_ids"),
            highlight: u("u_highlight"),
            id_color: u("u_idColor"),
            political: u("u_political"),
            sun: u("u_sun"),
            sun_on: u("u_sunOn"),
            bounds: u("u_bounds"),
            scale: u("u_scale"),
            mode: u("u_mode"),
            proj: u("u_proj"),
            flat_px: u("u_flatPx"),
            cy: u("u_cy"),
            lon0: u("u_lon0"),
            globe: u("u_globe"),
            proj_a: u("u_projA"),
            proj_b: u("u_projB"),
            blend_mix: u("u_mix"),
            cy_ab: u("u_cyAB"),
        }
    }
}

/// The globe <-> flat morph mesh: a 1 degree lat/lon grid.
struct MorphMesh {
    vao: WebGlVertexArrayObject,
    indices: i32,
    program: WebGlProgram,
    uniforms: Uniforms,
}

/// Fills the screen with space and stars; the globe quad is drawn over it.
struct SpacePass {
    program: WebGlProgram,
    bounds: Option<WebGlUniformLocation>,
    dpr: Option<WebGlUniformLocation>,
    scale: Option<WebGlUniformLocation>,
}

/// Upscales the reduced-resolution globe render onto the canvas.
struct BlitPass {
    program: WebGlProgram,
    bounds: Option<WebGlUniformLocation>,
    src: Option<WebGlUniformLocation>,
    res: Option<WebGlUniformLocation>,
}

/// RGBA8 render target for the reduced-resolution globe pass.
struct ColorTarget {
    fbo: WebGlFramebuffer,
    tex: WebGlTexture,
    w: i32,
    h: i32,
}

/// NDC rectangle covering the whole screen.
const FULL_SCREEN: [f32; 4] = [-1.0, -1.0, 1.0, 1.0];
/// Space color, same as the base of `space()` in the shaders.
const SPACE_COLOR: [f32; 3] = [0.006, 0.010, 0.024];

struct LineUniforms {
    res: Option<WebGlUniformLocation>,
    dist: Option<WebGlUniformLocation>,
    tan_half_fov: Option<WebGlUniformLocation>,
    rot: Option<WebGlUniformLocation>,
    width: Option<WebGlUniformLocation>,
    strength: Option<WebGlUniformLocation>,
    lon0: Option<WebGlUniformLocation>,
    proj_a: Option<WebGlUniformLocation>,
    proj_b: Option<WebGlUniformLocation>,
    blend_mix: Option<WebGlUniformLocation>,
    cy_ab: Option<WebGlUniformLocation>,
    flat_px: Option<WebGlUniformLocation>,
    shift: Option<WebGlUniformLocation>,
    clip: Option<WebGlUniformLocation>,
}

/// Program and shared quad corners for the line pass.
struct LinePass {
    program: WebGlProgram,
    uniforms: LineUniforms,
    /// Four corners of the instanced quad (triangle strip).
    corners: WebGlBuffer,
}

impl LinePass {
    fn create(gl: &GL) -> Result<Self, JsValue> {
        let vs = compile(gl, GL::VERTEX_SHADER, LINE_VERT_SRC)?;
        let fs = compile(gl, GL::FRAGMENT_SHADER, LINE_FRAG_SRC)?;
        let program = link(gl, &vs, &fs)?;
        let lu = |name: &str| gl.get_uniform_location(&program, name);
        let uniforms = LineUniforms {
            res: lu("u_res"),
            dist: lu("u_dist"),
            tan_half_fov: lu("u_tanHalfFov"),
            rot: lu("u_rot"),
            width: lu("u_width"),
            strength: lu("u_strength"),
            lon0: lu("u_lon0"),
            proj_a: lu("u_projA"),
            proj_b: lu("u_projB"),
            blend_mix: lu("u_mix"),
            cy_ab: lu("u_cyAB"),
            flat_px: lu("u_flatPx"),
            shift: lu("u_shift"),
            clip: lu("u_clip"),
        };
        let corners = gl.create_buffer().ok_or("failed to create buffer")?;
        gl.bind_buffer(GL::ARRAY_BUFFER, Some(&corners));
        let quad = js_sys::Float32Array::from(&[0.0f32, -1.0, 0.0, 1.0, 1.0, -1.0, 1.0, 1.0][..]);
        gl.buffer_data_with_array_buffer_view(GL::ARRAY_BUFFER, &quad, GL::STATIC_DRAW);
        Ok(Self {
            program,
            uniforms,
            corners,
        })
    }
}

/// One level of detail of one line layer, uploaded.
struct LineBuf {
    vao: WebGlVertexArrayObject,
    buf: WebGlBuffer,
    segments: i32,
}

/// Uploaded line geometry, one buffer per level of detail.
struct LineGpu {
    coast: Vec<LineBuf>,
    borders: Vec<LineBuf>,
}

/// Screen-sized RG8 render target for the line pass (R coast, G borders).
struct LineTarget {
    fbo: WebGlFramebuffer,
    tex: WebGlTexture,
    w: i32,
    h: i32,
}

fn upload_levels(gl: &GL, pass: &LinePass, levels: &LineLevels) -> Result<Vec<LineBuf>, JsValue> {
    let mut out = Vec::with_capacity(levels.levels.len());
    for data in &levels.levels {
        let vao = gl.create_vertex_array().ok_or("failed to create VAO")?;
        let buf = gl.create_buffer().ok_or("failed to create buffer")?;
        gl.bind_vertex_array(Some(&vao));
        gl.bind_buffer(GL::ARRAY_BUFFER, Some(&pass.corners));
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_with_i32(0, 2, GL::FLOAT, false, 0, 0);
        gl.bind_buffer(GL::ARRAY_BUFFER, Some(&buf));
        let arr = js_sys::Float32Array::from(data.as_slice());
        gl.buffer_data_with_array_buffer_view(GL::ARRAY_BUFFER, &arr, GL::STATIC_DRAW);
        for (loc, offset) in [(1u32, 0), (2, 12)] {
            gl.enable_vertex_attrib_array(loc);
            gl.vertex_attrib_pointer_with_i32(loc, 3, GL::FLOAT, false, 24, offset);
            gl.vertex_attrib_divisor(loc, 1);
        }
        out.push(LineBuf {
            vao,
            buf,
            segments: (data.len() / 6) as i32,
        });
    }
    gl.bind_vertex_array(None);
    Ok(out)
}

/// GL objects that die with the context and must be rebuilt after a restore.
struct Resources {
    line_pass: LinePass,
    space: SpacePass,
    blit: BlitPass,
    program: WebGlProgram,
    frag: WebGlShader,
    vao: WebGlVertexArrayObject,
    uniforms: Uniforms,
    map: WebGlTexture,
    /// Country-ID raster (R8, one byte per texel, 0 = no country).
    ids: WebGlTexture,
    /// Satellite imagery: a 1x1 placeholder until the layer's file is decoded.
    imagery: WebGlTexture,
}

impl Resources {
    fn create(gl: &GL) -> Result<Self, JsValue> {
        let vs = compile(gl, GL::VERTEX_SHADER, VERT_SRC)?;
        let fs = compile(gl, GL::FRAGMENT_SHADER, FRAG_SRC)?;
        let program = link(gl, &vs, &fs)?;
        let uniforms = Uniforms::lookup(gl, &program);
        let space = {
            let fs = compile(gl, GL::FRAGMENT_SHADER, SPACE_FRAG_SRC)?;
            let program = link(gl, &vs, &fs)?;
            let su = |name: &str| gl.get_uniform_location(&program, name);
            SpacePass {
                bounds: su("u_bounds"),
                dpr: su("u_dpr"),
                scale: su("u_scale"),
                program,
            }
        };
        let blit = {
            let fs = compile(gl, GL::FRAGMENT_SHADER, BLIT_FRAG_SRC)?;
            let program = link(gl, &vs, &fs)?;
            let bu = |name: &str| gl.get_uniform_location(&program, name);
            BlitPass {
                bounds: bu("u_bounds"),
                src: bu("u_src"),
                res: bu("u_res"),
                program,
            }
        };
        let vao = gl.create_vertex_array().ok_or("failed to create VAO")?;
        let line_pass = LinePass::create(gl)?;

        // 1×1 placeholder until the vector data has been rasterized.
        let map = gl.create_texture().ok_or("failed to create texture")?;
        gl.bind_texture(GL::TEXTURE_2D, Some(&map));
        gl.pixel_storei(GL::UNPACK_ALIGNMENT, 1);
        gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
            GL::TEXTURE_2D,
            0,
            GL::R8 as i32,
            1,
            1,
            0,
            GL::RED,
            GL::UNSIGNED_BYTE,
            Some(&[0]),
        )?;
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MIN_FILTER, GL::NEAREST as i32);
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MAG_FILTER, GL::NEAREST as i32);

        let ids = gl.create_texture().ok_or("failed to create texture")?;
        gl.bind_texture(GL::TEXTURE_2D, Some(&ids));
        gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
            GL::TEXTURE_2D,
            0,
            GL::R8 as i32,
            1,
            1,
            0,
            GL::RED,
            GL::UNSIGNED_BYTE,
            Some(&[0]),
        )?;
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MIN_FILTER, GL::NEAREST as i32);
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MAG_FILTER, GL::NEAREST as i32);

        let imagery = gl.create_texture().ok_or("failed to create texture")?;
        gl.bind_texture(GL::TEXTURE_2D, Some(&imagery));
        gl.pixel_storei(GL::UNPACK_ALIGNMENT, 1);
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

        Ok(Self {
            line_pass,
            space,
            blit,
            program,
            frag: fs,
            vao,
            uniforms,
            map,
            ids,
            imagery,
        })
    }
}

/// Tracks whether the WebGL context is usable. Browser-free so it can be
/// unit tested natively.
///
/// A context can be lost at any time (GPU reset, backgrounded mobile tab).
/// While lost no GL call may be made. Map data may finish loading while the
/// context is lost; it is then uploaded once the context is restored.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ContextState {
    lost: bool,
    /// The decoded map is held in memory and ready to be rasterized.
    map_loaded: bool,
    /// The country-ID raster is held in memory and ready to be uploaded.
    ids_loaded: bool,
}

impl ContextState {
    pub fn is_lost(&self) -> bool {
        self.lost
    }

    pub fn on_lost(&mut self) {
        self.lost = true;
    }

    /// Returns true if the map must be uploaded again after rebuilding.
    pub fn on_restored(&mut self) -> bool {
        self.lost = false;
        self.map_loaded
    }

    /// Record that the map data is available. Returns true if it can be
    /// uploaded right now, false if that must wait for a restore.
    pub fn on_map_loaded(&mut self) -> bool {
        self.map_loaded = true;
        !self.lost
    }

    /// Like [`Self::on_map_loaded`], for the country-ID raster.
    pub fn on_ids_loaded(&mut self) -> bool {
        self.ids_loaded = true;
        !self.lost
    }

    /// True if the ID raster must be uploaded again after a restore.
    pub fn ids_need_upload(&self) -> bool {
        self.ids_loaded
    }
}

/// Values of the `u_mode` uniform in `globe.frag`.
const MODE_GLOBE: i32 = 0;
const MODE_FLAT: i32 = 1;
const MODE_MORPH: i32 = 2;
const MODE_SPACE: i32 = 3;

/// Which shader path draws this blend: exact per-pixel globe or flat map
/// when settled, the mesh while morphing.
fn blend_mode(blend: &Blend) -> i32 {
    if !blend.is_settled() {
        MODE_MORPH
    } else if blend.shown().is_flat() {
        MODE_FLAT
    } else {
        MODE_GLOBE
    }
}

/// Per-frame projection state shared by the quad, mesh and line passes.
struct FlatFrame {
    /// Central meridian, radians.
    lon0: f32,
    proj_a: i32,
    proj_b: i32,
    /// 0 = all A, 1 = all B.
    mix: f32,
    /// Plane y of the screen-center row, for A and B.
    cy: [f32; 2],
    /// Framebuffer pixels per plane unit.
    flat_px: f32,
}

impl FlatFrame {
    fn new(camera: &Camera, blend: &Blend, w: u32, h: u32) -> Self {
        let (a, b, mix) = if blend.is_settled() {
            let p = blend.shown();
            (p, p, 0.0)
        } else {
            (blend.a, blend.b, blend.t)
        };
        let cy = |p: Projection| {
            if p.is_flat() {
                projection::y_of_lat(p, camera.lat) as f32
            } else {
                0.0
            }
        };
        FlatFrame {
            lon0: camera.lon as f32,
            proj_a: a.shader_index(),
            proj_b: b.shader_index(),
            mix: mix as f32,
            cy: [cy(a), cy(b)],
            flat_px: camera.flat_scale(0.5 * w.min(h) as f64) as f32,
        }
    }

    /// Longitude offsets (radians) of the copies of the world the line pass
    /// draws, and whether to clip each to one world.
    fn line_copies(&self, blend: &Blend, w: u32) -> (Vec<f32>, bool) {
        if blend_mode(blend) != MODE_FLAT {
            return (vec![0.0], false);
        }
        let half_w = 0.5 * w as f64 / (self.flat_px as f64).max(1e-6);
        let (k, clip) = projection::line_wraps(blend.shown(), half_w);
        let shifts = (-k..=k).map(|i| i as f32 * std::f32::consts::TAU).collect();
        (shifts, clip)
    }

    /// Altitude to pick the line level of detail by: a flat map is drawn
    /// smaller than the globe at the same altitude, so it needs the finer
    /// levels sooner (and Mercator more so, as it stretches away from the
    /// equator).
    fn lod_alt(&self, camera: &Camera, blend: &Blend) -> f64 {
        let flat = 1.0 - blend.globe_weight();
        let mut alt = camera.alt / (1.0 - (1.0 - FLAT_ZOOM) * flat);
        if blend.is_settled() && blend.shown() == Projection::Mercator {
            alt *= 0.6;
        }
        alt
    }
}

impl MorphMesh {
    /// Build the mesh and its program (sharing the fragment shader `fs`).
    fn create(gl: &GL, fs: &WebGlShader) -> Result<Self, JsValue> {
        let vs = compile(gl, GL::VERTEX_SHADER, MORPH_VERT_SRC)?;
        let program = link(gl, &vs, fs)?;
        let uniforms = Uniforms::lookup(gl, &program);
        let (verts, indices) = morph_grid();
        let vao = gl.create_vertex_array().ok_or("failed to create VAO")?;
        let vbo = gl.create_buffer().ok_or("failed to create buffer")?;
        let ibo = gl.create_buffer().ok_or("failed to create buffer")?;
        gl.bind_vertex_array(Some(&vao));
        gl.bind_buffer(GL::ARRAY_BUFFER, Some(&vbo));
        let v = js_sys::Float32Array::from(verts.as_slice());
        gl.buffer_data_with_array_buffer_view(GL::ARRAY_BUFFER, &v, GL::STATIC_DRAW);
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_with_i32(0, 2, GL::FLOAT, false, 0, 0);
        gl.bind_buffer(GL::ELEMENT_ARRAY_BUFFER, Some(&ibo));
        let i = js_sys::Uint16Array::from(indices.as_slice());
        gl.buffer_data_with_array_buffer_view(GL::ELEMENT_ARRAY_BUFFER, &i, GL::STATIC_DRAW);
        gl.bind_vertex_array(None);
        Ok(MorphMesh {
            vao,
            indices: indices.len() as i32,
            program,
            uniforms,
        })
    }
}

/// Vertices (latitude, longitude relative to the central meridian, radians)
/// and triangle indices of a 1 degree grid from pole to pole and -180 to 180.
/// 361 x 181 = 65 341 vertices, which still fits 16-bit indices.
fn morph_grid() -> (Vec<f32>, Vec<u16>) {
    const COLS: usize = 361;
    const ROWS: usize = 181;
    let mut verts = Vec::with_capacity(COLS * ROWS * 2);
    for r in 0..ROWS {
        let lat = (r as f64 - 90.0).to_radians();
        for c in 0..COLS {
            verts.push(lat as f32);
            verts.push((c as f64 - 180.0).to_radians() as f32);
        }
    }
    let mut indices = Vec::with_capacity((COLS - 1) * (ROWS - 1) * 6);
    for r in 0..ROWS - 1 {
        for c in 0..COLS - 1 {
            let i = (r * COLS + c) as u16;
            let (a, b, d, e) = (i, i + 1, i + COLS as u16, i + COLS as u16 + 1);
            indices.extend_from_slice(&[a, b, d, b, e, d]);
        }
    }
    (verts, indices)
}

pub struct Renderer {
    gl: GL,
    line_pass: LinePass,
    /// Uploaded line geometry (none until the map data has loaded).
    lines: Option<LineGpu>,
    /// Line-pass render target, resized to the canvas on demand.
    target: Option<LineTarget>,
    /// Same for reduced-resolution frames, kept apart so switching between
    /// the two sizes never reallocates.
    target_lo: Option<LineTarget>,
    /// Reduced-resolution globe render, upscaled by the blit pass.
    color_lo: Option<ColorTarget>,
    space: SpacePass,
    blit: BlitPass,
    program: WebGlProgram,
    /// The shared fragment shader, kept to link the morph program on demand.
    frag: WebGlShader,
    /// Morph mesh, built the first time a morph is drawn.
    morph: Option<MorphMesh>,
    vao: WebGlVertexArrayObject,
    uniforms: Uniforms,
    map: WebGlTexture,
    has_map: bool,
    /// Satellite imagery texture (placeholder until uploaded) and whether
    /// the real image is in it.
    imagery: WebGlTexture,
    has_imagery: bool,
    layer: Layer,
    /// World-space direction to the sun, or `None` when the terminator is off.
    sun: Option<[f32; 3]>,
    ctx: ContextState,
    /// Country-ID raster texture (placeholder until uploaded).
    ids: WebGlTexture,
    /// ID of the highlighted country, 0 for none.
    highlight: u8,
    /// Political fill color index per country ID, once the country data has
    /// loaded. Kept on the CPU so it survives context loss. `None` draws plain
    /// land, as before the data arrives.
    country_colors: Option<[f32; 256]>,
    /// Texture sizes, chosen once from the device and kept across context loss.
    budget: TextureBudget,
    /// With `?debug=ctxloss`, the `WEBGL_lose_context` extension for the debug
    /// key. It must be fetched while the context is alive: `getExtension`
    /// returns null once it is lost, and the object stays valid for
    /// `restoreContext`.
    lose_ext: Option<web_sys::WebglLoseContext>,
}

impl Renderer {
    pub fn new(canvas: &HtmlCanvasElement) -> Result<Self, JsValue> {
        let opts = js_sys::Object::new();
        js_sys::Reflect::set(&opts, &"antialias".into(), &false.into())?;
        js_sys::Reflect::set(&opts, &"alpha".into(), &false.into())?;
        js_sys::Reflect::set(&opts, &"depth".into(), &false.into())?;
        let gl: GL = canvas
            .get_context_with_context_options("webgl2", &opts)?
            .ok_or("WebGL2 is not supported by this browser")?
            .dyn_into()?;
        let r = Resources::create(&gl)?;
        let forced = web_sys::window()
            .and_then(|w| w.location().search().ok())
            .and_then(|q| parse_tier(&q));
        let budget = choose_budget(&Self::detect_device(&gl), forced);
        let debug_ctxloss = web_sys::window()
            .and_then(|w| w.location().search().ok())
            .is_some_and(|q| parse_debug_ctxloss(&q));
        // Browsers return a plain object with no global constructor, so
        // `dyn_into` (an `instanceof` check) would always fail.
        let lose_ext = debug_ctxloss
            .then(|| gl.get_extension("WEBGL_lose_context").ok().flatten())
            .flatten()
            .map(|o| o.unchecked_into::<web_sys::WebglLoseContext>());
        Ok(Self {
            gl,
            line_pass: r.line_pass,
            lines: None,
            target: None,
            target_lo: None,
            color_lo: None,
            space: r.space,
            blit: r.blit,
            program: r.program,
            frag: r.frag,
            morph: None,
            vao: r.vao,
            uniforms: r.uniforms,
            map: r.map,
            has_map: false,
            imagery: r.imagery,
            has_imagery: false,
            layer: Layer::default(),
            sun: None,
            ctx: ContextState::default(),
            ids: r.ids,
            highlight: 0,
            country_colors: None,
            budget,
            lose_ext,
        })
    }

    /// Debug aid for `?debug=ctxloss`: lose the context with
    /// `WEBGL_lose_context`, or restore it if it is already lost. Returns
    /// false (and does nothing) without the flag or the extension.
    pub fn debug_toggle_context_loss(&self) -> bool {
        let Some(ext) = &self.lose_ext else {
            return false;
        };
        if self.gl.is_context_lost() {
            ext.restore_context();
        } else {
            ext.lose_context();
        }
        true
    }

    /// Width of the country-ID raster this device uses. Rasterize the IDs at
    /// this width before calling [`Self::upload_country_ids`].
    pub fn ids_width(&self) -> usize {
        self.budget.ids_width
    }

    /// Width of the map texture this device uses. Pass it to
    /// `Layers::load` so the land mask is rasterized at this size.
    pub fn map_width(&self) -> u32 {
        self.budget.map_width
    }

    /// Width of the satellite imagery this device uses (see [`choose_budget`]).
    pub fn imagery_width(&self) -> u32 {
        self.budget.imagery_width
    }

    /// Whether the satellite imagery is in GPU memory right now.
    pub fn has_imagery(&self) -> bool {
        self.has_imagery
    }

    pub fn is_lost(&self) -> bool {
        self.ctx.is_lost()
    }

    /// Upload the decoded satellite image as an RGBA8 texture with mipmaps,
    /// sized from the bitmap. While the context is lost nothing is done (the
    /// caller decodes and uploads again after the restore).
    pub fn upload_imagery(&mut self, bitmap: &web_sys::ImageBitmap) -> Result<(), JsValue> {
        if self.gl.is_context_lost() || self.ctx.is_lost() {
            return Ok(());
        }
        let gl = &self.gl;
        let (w, h) = (bitmap.width(), bitmap.height());
        let max = gl
            .get_parameter(GL::MAX_TEXTURE_SIZE)
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(4096.0) as u32;
        if w == 0 || h == 0 || w > max {
            return Err(format!("satellite image {w}x{h} does not fit this GPU").into());
        }
        let tex = gl.create_texture().ok_or("failed to create texture")?;
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
        self.set_anisotropy();
        self.gl.delete_texture(Some(&self.imagery));
        self.imagery = tex;
        self.has_imagery = true;
        web_sys::console::log_1(
            &format!(
                "satellite texture {w}x{h}: {:.1} MB",
                imagery_bytes(&TextureBudget {
                    imagery_width: w,
                    ..self.budget
                }) as f64
                    / 1048576.0
            )
            .into(),
        );
        Ok(())
    }

    /// Anisotropic filtering on the bound texture, up to 8x where available.
    fn set_anisotropy(&self) {
        let gl = &self.gl;
        if gl
            .get_extension("EXT_texture_filter_anisotropic")
            .ok()
            .flatten()
            .is_some()
        {
            let max = gl
                .get_parameter(MAX_TEXTURE_MAX_ANISOTROPY_EXT)
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0);
            gl.tex_parameterf(
                GL::TEXTURE_2D,
                TEXTURE_MAX_ANISOTROPY_EXT,
                max.min(8.0) as f32,
            );
        }
    }

    /// The context was lost: stop issuing GL calls until it is restored.
    pub fn context_lost(&mut self) {
        self.ctx.on_lost();
    }

    /// The context came back: rebuild every GL object and, if the map data
    /// is already in memory, upload it again without refetching. The layer
    /// and camera live outside GL, so the view is unchanged.
    pub fn context_restored(
        &mut self,
        layers: Option<&Layers>,
        ids: Option<&[u8]>,
    ) -> Result<(), JsValue> {
        let upload = self.ctx.on_restored();
        let r = Resources::create(&self.gl)?;
        self.program = r.program;
        self.line_pass = r.line_pass;
        self.space = r.space;
        self.blit = r.blit;
        self.frag = r.frag;
        self.morph = None; // rebuilt on demand
        self.vao = r.vao;
        self.uniforms = r.uniforms;
        self.map = r.map;
        self.ids = r.ids;
        self.imagery = r.imagery;
        // The caller decodes the imagery again from its encoded bytes.
        self.has_imagery = false;
        self.has_map = false;
        // Buffers and the render target died with the old context.
        self.lines = None;
        self.target = None;
        self.target_lo = None;
        self.color_lo = None;
        if let (Some(ids), true) = (ids, self.ctx.ids_need_upload()) {
            self.upload_country_ids(ids, self.budget.ids_width)?;
        }
        match layers {
            Some(l) if upload => self.upload_map(l),
            _ => Ok(()),
        }
    }

    fn detect_device(gl: &GL) -> DeviceInfo {
        let max_texture_size = gl
            .get_parameter(GL::MAX_TEXTURE_SIZE)
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(4096.0) as u32;
        let window = web_sys::window();
        let device_memory_gb = window.as_ref().and_then(|w| {
            js_sys::Reflect::get(&w.navigator(), &"deviceMemory".into())
                .ok()?
                .as_f64()
        });
        let coarse_pointer = window
            .and_then(|w| w.match_media("(pointer: coarse)").ok().flatten())
            .is_some_and(|m| m.matches());
        DeviceInfo {
            max_texture_size,
            device_memory_gb,
            coarse_pointer,
        }
    }

    /// Rasterize the land fill into a mipmapped equirectangular texture whose
    /// width comes from [`choose_budget`], and upload the vector lines.
    ///
    /// The map is drawn as two square halves so no single canvas exceeds the
    /// 16.7 MP limit some mobile browsers enforce.
    ///
    /// If the context is currently lost the upload is deferred until it is
    /// restored.
    pub fn upload_map(&mut self, layers: &Layers) -> Result<(), JsValue> {
        if !self.ctx.on_map_loaded() || self.gl.is_context_lost() {
            return Ok(());
        }
        let gl = &self.gl;
        let width = self.budget.map_width;
        let height = width / 2;
        let (map_bytes, ids_bytes) = budget_bytes(&self.budget);
        web_sys::console::log_1(
            &format!(
                "map texture {width}x{height}, ids {w}x{}: {:.1} MB",
                self.budget.ids_width / 2,
                (map_bytes + ids_bytes) as f64 / 1048576.0,
                w = self.budget.ids_width,
            )
            .into(),
        );
        let levels = 32 - width.leading_zeros() as i32; // log2(width) + 1

        let tex = gl.create_texture().ok_or("failed to create texture")?;
        gl.bind_texture(GL::TEXTURE_2D, Some(&tex));
        gl.tex_storage_2d(GL::TEXTURE_2D, levels, GL::R8, width as i32, height as i32);
        gl.pixel_storei(GL::UNPACK_COLORSPACE_CONVERSION_WEBGL, GL::NONE as i32);
        gl.pixel_storei(GL::UNPACK_PREMULTIPLY_ALPHA_WEBGL, 0);
        layers.fill_texture(gl, width, height)?;
        gl.generate_mipmap(GL::TEXTURE_2D);
        gl.tex_parameteri(
            GL::TEXTURE_2D,
            GL::TEXTURE_MIN_FILTER,
            GL::LINEAR_MIPMAP_LINEAR as i32,
        );
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MAG_FILTER, GL::LINEAR as i32);
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_WRAP_S, GL::REPEAT as i32);
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_WRAP_T, GL::CLAMP_TO_EDGE as i32);
        self.set_anisotropy();

        let gl = &self.gl;
        gl.delete_texture(Some(&self.map));
        self.map = tex;
        self.has_map = true;
        self.upload_lines(layers.lines())
    }

    /// Upload coastline and border geometry, replacing any earlier upload.
    fn upload_lines(&mut self, set: &LineSet) -> Result<(), JsValue> {
        let gl = &self.gl;
        let gpu = LineGpu {
            coast: upload_levels(gl, &self.line_pass, &set.coast)?,
            borders: upload_levels(gl, &self.line_pass, &set.borders)?,
        };
        if let Some(old) = self.lines.replace(gpu) {
            for b in old.coast.iter().chain(&old.borders) {
                gl.delete_vertex_array(Some(&b.vao));
                gl.delete_buffer(Some(&b.buf));
            }
        }
        Ok(())
    }

    /// Make sure the line target matches the canvas size.
    fn ensure_target(&mut self, w: i32, h: i32) -> Result<(), JsValue> {
        ensure_line_target(&self.gl, &mut self.target, w, h)
    }

    /// Make sure the scaled color target (and its line target) exist at this size.
    fn ensure_scaled(&mut self, w: i32, h: i32) -> Result<(), JsValue> {
        ensure_line_target(&self.gl, &mut self.target_lo, w, h)?;
        if self.color_lo.as_ref().is_some_and(|t| t.w == w && t.h == h) {
            return Ok(());
        }
        let gl = &self.gl;
        if let Some(old) = self.color_lo.take() {
            gl.delete_framebuffer(Some(&old.fbo));
            gl.delete_texture(Some(&old.tex));
        }
        let tex = gl.create_texture().ok_or("failed to create texture")?;
        gl.bind_texture(GL::TEXTURE_2D, Some(&tex));
        gl.tex_storage_2d(GL::TEXTURE_2D, 1, GL::RGBA8, w, h);
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MIN_FILTER, GL::LINEAR as i32);
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MAG_FILTER, GL::LINEAR as i32);
        let fbo = gl
            .create_framebuffer()
            .ok_or("failed to create framebuffer")?;
        gl.bind_framebuffer(GL::FRAMEBUFFER, Some(&fbo));
        gl.framebuffer_texture_2d(
            GL::FRAMEBUFFER,
            GL::COLOR_ATTACHMENT0,
            GL::TEXTURE_2D,
            Some(&tex),
            0,
        );
        gl.bind_framebuffer(GL::FRAMEBUFFER, None);
        self.color_lo = Some(ColorTarget { fbo, tex, w, h });
        Ok(())
    }

    /// Render coastlines (R) and borders (G) into the line target.
    fn draw_lines(
        &self,
        target: &LineTarget,
        camera: &Camera,
        blend: &Blend,
        w: u32,
        h: u32,
        dpr: f64,
    ) {
        let Some(gpu) = &self.lines else {
            return;
        };
        let gl = &self.gl;
        let u = &self.line_pass.uniforms;
        let frame = FlatFrame::new(camera, blend, w, h);
        gl.bind_framebuffer(GL::FRAMEBUFFER, Some(&target.fbo));
        gl.viewport(0, 0, w as i32, h as i32);
        gl.clear_color(0.0, 0.0, 0.0, 0.0);
        gl.clear(GL::COLOR_BUFFER_BIT);
        gl.use_program(Some(&self.line_pass.program));
        gl.uniform2f(u.res.as_ref(), w as f32, h as f32);
        gl.uniform1f(u.dist.as_ref(), camera.dist() as f32);
        gl.uniform1f(u.tan_half_fov.as_ref(), TAN_HALF_FOV as f32);
        gl.uniform_matrix3fv_with_f32_array(u.rot.as_ref(), false, &camera.rotation());
        gl.uniform1f(u.lon0.as_ref(), frame.lon0);
        gl.uniform1i(u.proj_a.as_ref(), frame.proj_a);
        gl.uniform1i(u.proj_b.as_ref(), frame.proj_b);
        gl.uniform1f(u.blend_mix.as_ref(), frame.mix);
        gl.uniform2f(u.cy_ab.as_ref(), frame.cy[0], frame.cy[1]);
        gl.uniform1f(u.flat_px.as_ref(), frame.flat_px);
        // Copies of the world to draw on a flat map: see `line_wraps`.
        let (shifts, clip) = frame.line_copies(blend, w);
        gl.uniform1f(u.clip.as_ref(), if clip { 1.0 } else { 0.0 });
        gl.enable(GL::BLEND);
        gl.blend_equation(GL::MAX);
        let level = lod_for(frame.lod_alt(camera, blend), w.min(h) as f64, TAN_HALF_FOV);
        let dpr = dpr as f32;
        for (mask, width, bufs) in [
            ([true, false, false, false], COAST_WIDTH, &gpu.coast),
            ([false, true, false, false], BORDER_WIDTH, &gpu.borders),
        ] {
            let Some(b) = bufs.get(level.min(bufs.len().saturating_sub(1))) else {
                continue;
            };
            gl.color_mask(mask[0], mask[1], mask[2], mask[3]);
            gl.uniform1f(u.width.as_ref(), width * dpr);
            gl.uniform1f(u.strength.as_ref(), 1.0);
            gl.bind_vertex_array(Some(&b.vao));
            for shift in &shifts {
                gl.uniform1f(u.shift.as_ref(), *shift);
                gl.draw_arrays_instanced(GL::TRIANGLE_STRIP, 0, 4, b.segments);
            }
        }
        gl.color_mask(true, true, true, true);
        gl.disable(GL::BLEND);
        gl.bind_framebuffer(GL::FRAMEBUFFER, None);
    }

    /// Upload the country-ID raster (`width × width/2`, row 0 at the north).
    /// It is fetched with `texelFetch`, so it has no mipmaps or filtering
    /// (2 MB at 2048 wide, 8 MB at 4096). While the context is lost the upload is deferred
    /// until `context_restored`, which is given the raster again.
    pub fn upload_country_ids(&mut self, ids: &[u8], width: usize) -> Result<(), JsValue> {
        if !self.ctx.on_ids_loaded() || self.gl.is_context_lost() {
            return Ok(());
        }
        let gl = &self.gl;
        let tex = gl.create_texture().ok_or("failed to create texture")?;
        gl.bind_texture(GL::TEXTURE_2D, Some(&tex));
        gl.pixel_storei(GL::UNPACK_ALIGNMENT, 1);
        gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
            GL::TEXTURE_2D,
            0,
            GL::R8 as i32,
            width as i32,
            (width / 2) as i32,
            0,
            GL::RED,
            GL::UNSIGNED_BYTE,
            Some(ids),
        )?;
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MIN_FILTER, GL::NEAREST as i32);
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MAG_FILTER, GL::NEAREST as i32);
        gl.delete_texture(Some(&self.ids));
        self.ids = tex;
        Ok(())
    }

    /// Highlight the country with this ID (0 for none). Returns true if the
    /// highlight changed, so the caller knows a redraw is needed.
    pub fn set_highlight(&mut self, id: u8) -> bool {
        if self.ctx.is_lost() {
            return false;
        }
        let changed = self.highlight != id;
        self.highlight = id;
        changed
    }

    /// Per-ID political color indexes (see `Countries::id_colors`). Call this
    /// together with [`Self::upload_country_ids`]: the fills need both.
    pub fn set_country_colors(&mut self, colors: &[u8; 256]) {
        self.country_colors = Some(colors.map(f32::from));
    }

    pub fn set_layer(&mut self, layer: Layer) {
        self.layer = layer;
    }

    pub fn set_sun(&mut self, sun: Option<[f32; 3]>) {
        self.sun = sun;
    }

    /// Draw the map: the globe, a flat projection or a morph between them,
    /// as `blend` says. `scale` below 1 shades it at that fraction of the
    /// canvas resolution into an offscreen target and upscales it; the line
    /// target is scaled the same way so `globe.frag` still reads it
    /// one-to-one with `texelFetch`.
    pub fn draw(
        &mut self,
        camera: &Camera,
        blend: Blend,
        width_px: u32,
        height_px: u32,
        dpr: f64,
        scale: f64,
    ) {
        if self.ctx.is_lost() {
            return;
        }
        let (rw, rh) = scaled_size(width_px, height_px, scale);
        let mut scaled = (rw, rh) != (width_px, height_px);
        if scaled && self.ensure_scaled(rw as i32, rh as i32).is_err() {
            scaled = false; // fall back to full resolution
        }
        let (rw, rh) = if scaled {
            (rw, rh)
        } else {
            (width_px, height_px)
        };
        let s = rw as f64 / width_px.max(1) as f64;
        let mode = blend_mode(&blend);
        if mode == MODE_MORPH && self.morph.is_none() {
            match MorphMesh::create(&self.gl, &self.frag) {
                Ok(m) => self.morph = Some(m),
                Err(e) => web_sys::console::error_1(&e),
            }
        }
        let mut have_lines = false;
        if self.lines.is_some() {
            if scaled {
                have_lines = true;
                if let Some(t) = &self.target_lo {
                    self.draw_lines(t, camera, &blend, rw, rh, dpr * s);
                }
            } else if self
                .ensure_target(width_px as i32, height_px as i32)
                .is_ok()
            {
                have_lines = true;
                if let Some(t) = &self.target {
                    self.draw_lines(t, camera, &blend, rw, rh, dpr);
                }
            }
        }
        let gl = &self.gl;
        gl.bind_vertex_array(Some(&self.vao));
        // Only the ray-traced globe leaves most of the screen empty.
        let bounds = |w: f64, h: f64| {
            if mode == MODE_GLOBE {
                camera.globe_bounds(w, h).map(|v| v as f32)
            } else {
                FULL_SCREEN
            }
        };
        let b = bounds(rw as f64, rh as f64);
        match (&self.color_lo, scaled) {
            (Some(c), true) => {
                gl.bind_framebuffer(GL::FRAMEBUFFER, Some(&c.fbo));
                gl.viewport(0, 0, rw as i32, rh as i32);
                // Texels the upscaler may touch outside the globe quad.
                gl.clear_color(SPACE_COLOR[0], SPACE_COLOR[1], SPACE_COLOR[2], 1.0);
                gl.clear(GL::COLOR_BUFFER_BIT);
            }
            _ => {
                gl.bind_framebuffer(GL::FRAMEBUFFER, None);
                gl.viewport(0, 0, width_px as i32, height_px as i32);
                self.draw_space(b, dpr);
            }
        }
        let frame = FlatFrame::new(camera, &blend, rw, rh);
        let line_target = if scaled {
            &self.target_lo
        } else {
            &self.target
        };
        let lines_tex = line_target.as_ref().filter(|_| have_lines).map(|t| &t.tex);
        // The quad program: the globe, a flat map, or (morph) the space
        // behind the mesh.
        let u = &self.uniforms;
        gl.use_program(Some(&self.program));
        self.set_frame_uniforms(
            u,
            camera,
            &frame,
            (rw, rh),
            dpr,
            s,
            lines_tex,
            if mode == MODE_MORPH { MODE_SPACE } else { mode },
        );
        gl.uniform4f(u.bounds.as_ref(), b[0], b[1], b[2], b[3]);
        gl.draw_arrays(GL::TRIANGLE_STRIP, 0, 4);

        if let (MODE_MORPH, Some(m)) = (mode, &self.morph) {
            gl.use_program(Some(&m.program));
            self.set_frame_uniforms(
                &m.uniforms,
                camera,
                &frame,
                (rw, rh),
                dpr,
                s,
                lines_tex,
                MODE_MORPH,
            );
            gl.uniform1i(m.uniforms.proj_a.as_ref(), frame.proj_a);
            gl.uniform1i(m.uniforms.proj_b.as_ref(), frame.proj_b);
            gl.uniform1f(m.uniforms.blend_mix.as_ref(), frame.mix);
            gl.uniform2f(m.uniforms.cy_ab.as_ref(), frame.cy[0], frame.cy[1]);
            gl.uniform1f(m.uniforms.globe.as_ref(), blend.globe_weight() as f32);
            gl.enable(GL::BLEND);
            gl.blend_func(GL::SRC_ALPHA, GL::ONE_MINUS_SRC_ALPHA);
            gl.bind_vertex_array(Some(&m.vao));
            gl.draw_elements_with_i32(GL::TRIANGLES, m.indices, GL::UNSIGNED_SHORT, 0);
            gl.disable(GL::BLEND);
            gl.bind_vertex_array(Some(&self.vao));
        }

        if let (Some(c), true) = (&self.color_lo, scaled) {
            gl.bind_framebuffer(GL::FRAMEBUFFER, None);
            gl.viewport(0, 0, width_px as i32, height_px as i32);
            let full = bounds(width_px as f64, height_px as f64);
            self.draw_space(full, dpr);
            let bl = &self.blit;
            gl.use_program(Some(&bl.program));
            gl.uniform4f(bl.bounds.as_ref(), full[0], full[1], full[2], full[3]);
            gl.uniform2f(bl.res.as_ref(), width_px as f32, height_px as f32);
            gl.active_texture(GL::TEXTURE0);
            gl.bind_texture(GL::TEXTURE_2D, Some(&c.tex));
            gl.uniform1i(bl.src.as_ref(), 0);
            gl.draw_arrays(GL::TRIANGLE_STRIP, 0, 4);
        }
    }

    /// Uniforms shared by the quad and mesh programs, plus the textures.
    #[allow(clippy::too_many_arguments)]
    fn set_frame_uniforms(
        &self,
        u: &Uniforms,
        camera: &Camera,
        frame: &FlatFrame,
        (rw, rh): (u32, u32),
        dpr: f64,
        s: f64,
        lines_tex: Option<&WebGlTexture>,
        mode: i32,
    ) {
        let gl = &self.gl;
        gl.uniform1f(u.scale.as_ref(), s as f32);
        gl.uniform2f(u.res.as_ref(), rw as f32, rh as f32);
        gl.uniform1f(u.dist.as_ref(), camera.dist() as f32);
        gl.uniform1f(u.tan_half_fov.as_ref(), TAN_HALF_FOV as f32);
        gl.uniform_matrix3fv_with_f32_array(u.rot.as_ref(), false, &camera.rotation());
        gl.uniform1f(u.has_map.as_ref(), if self.has_map { 1.0 } else { 0.0 });
        gl.uniform1f(u.grid_step.as_ref(), camera.grid_step_deg() as f32);
        gl.uniform1f(u.dpr.as_ref(), dpr as f32);
        gl.uniform1i(u.layer.as_ref(), self.layer.shader_index());
        gl.uniform1f(
            u.has_imagery.as_ref(),
            if self.has_imagery { 1.0 } else { 0.0 },
        );
        let sun = self.sun.unwrap_or([0.0, 0.0, 1.0]);
        gl.uniform3f(u.sun.as_ref(), sun[0], sun[1], sun[2]);
        gl.uniform1f(
            u.sun_on.as_ref(),
            if self.sun.is_some() { 1.0 } else { 0.0 },
        );
        gl.uniform1i(u.mode.as_ref(), mode);
        gl.uniform1i(u.proj.as_ref(), frame.proj_a);
        gl.uniform1f(u.flat_px.as_ref(), frame.flat_px);
        gl.uniform1f(u.cy.as_ref(), frame.cy[0]);
        gl.uniform1f(u.lon0.as_ref(), frame.lon0);
        gl.active_texture(GL::TEXTURE0);
        gl.bind_texture(GL::TEXTURE_2D, Some(&self.map));
        gl.uniform1i(u.map.as_ref(), 0);
        gl.active_texture(GL::TEXTURE1);
        gl.bind_texture(GL::TEXTURE_2D, Some(&self.ids));
        gl.uniform1i(u.ids.as_ref(), 1);
        // The line target; unbound (reads as zero) until lines are uploaded.
        gl.active_texture(GL::TEXTURE2);
        gl.bind_texture(GL::TEXTURE_2D, lines_tex);
        gl.uniform1i(u.lines.as_ref(), 2);
        gl.active_texture(GL::TEXTURE3);
        gl.bind_texture(GL::TEXTURE_2D, Some(&self.imagery));
        gl.uniform1i(u.imagery.as_ref(), 3);
        gl.uniform1i(u.highlight.as_ref(), i32::from(self.highlight));
        // The color table is 64 vec4s; `u_idColor[id / 4][id % 4]` is the color of id.
        let colors = self.country_colors.as_ref();
        gl.uniform1f(
            u.political.as_ref(),
            if colors.is_some() { 1.0 } else { 0.0 },
        );
        if let Some(c) = colors {
            gl.uniform4fv_with_f32_array(u.id_color.as_ref(), c);
        }
        gl.active_texture(GL::TEXTURE0);
    }

    /// Fill the canvas with space and stars, unless the globe quad `bounds`
    /// already covers the whole screen.
    fn draw_space(&self, bounds: [f32; 4], dpr: f64) {
        if bounds == FULL_SCREEN {
            return;
        }
        let gl = &self.gl;
        let sp = &self.space;
        gl.use_program(Some(&sp.program));
        gl.uniform4f(sp.bounds.as_ref(), -1.0, -1.0, 1.0, 1.0);
        gl.uniform1f(sp.dpr.as_ref(), dpr as f32);
        gl.uniform1f(sp.scale.as_ref(), 1.0);
        gl.draw_arrays(GL::TRIANGLE_STRIP, 0, 4);
    }
}

/// Create or resize a line-pass target so it is exactly `w × h`.
fn ensure_line_target(
    gl: &GL,
    slot: &mut Option<LineTarget>,
    w: i32,
    h: i32,
) -> Result<(), JsValue> {
    if slot.as_ref().is_some_and(|t| t.w == w && t.h == h) {
        return Ok(());
    }
    if let Some(old) = slot.take() {
        gl.delete_framebuffer(Some(&old.fbo));
        gl.delete_texture(Some(&old.tex));
    }
    let tex = gl.create_texture().ok_or("failed to create texture")?;
    gl.bind_texture(GL::TEXTURE_2D, Some(&tex));
    gl.tex_storage_2d(GL::TEXTURE_2D, 1, GL::RG8, w, h);
    gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MIN_FILTER, GL::NEAREST as i32);
    gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MAG_FILTER, GL::NEAREST as i32);
    let fbo = gl
        .create_framebuffer()
        .ok_or("failed to create framebuffer")?;
    gl.bind_framebuffer(GL::FRAMEBUFFER, Some(&fbo));
    gl.framebuffer_texture_2d(
        GL::FRAMEBUFFER,
        GL::COLOR_ATTACHMENT0,
        GL::TEXTURE_2D,
        Some(&tex),
        0,
    );
    gl.bind_framebuffer(GL::FRAMEBUFFER, None);
    *slot = Some(LineTarget { fbo, tex, w, h });
    Ok(())
}

fn compile(gl: &GL, kind: u32, src: &str) -> Result<WebGlShader, JsValue> {
    let shader = gl.create_shader(kind).ok_or("failed to create shader")?;
    gl.shader_source(&shader, src);
    gl.compile_shader(&shader);
    if gl
        .get_shader_parameter(&shader, GL::COMPILE_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        Ok(shader)
    } else {
        Err(gl.get_shader_info_log(&shader).unwrap_or_default().into())
    }
}

fn link(gl: &GL, vs: &WebGlShader, fs: &WebGlShader) -> Result<WebGlProgram, JsValue> {
    let program = gl.create_program().ok_or("failed to create program")?;
    gl.attach_shader(&program, vs);
    gl.attach_shader(&program, fs);
    gl.link_program(&program);
    if gl
        .get_program_parameter(&program, GL::LINK_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        Ok(program)
    } else {
        Err(gl.get_program_info_log(&program).unwrap_or_default().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The tests below read the shader sources as written (with comments), so
    // they shadow the minified build-time constants imported above.
    const VERT_SRC: &str = include_str!("shaders/globe.vert");
    const FRAG_SRC: &str = include_str!("shaders/globe.frag");
    const SPACE_FRAG_SRC: &str = include_str!("shaders/space.frag");
    const BLIT_FRAG_SRC: &str = include_str!("shaders/blit.frag");
    const LINE_VERT_SRC: &str = include_str!("shaders/line.vert");
    const LINE_FRAG_SRC: &str = include_str!("shaders/line.frag");

    include!("../shader_minify.rs");

    /// The minified shaders that are embedded in the wasm.
    const MINIFIED: [(&str, &str); 6] = [
        ("globe.vert", super::VERT_SRC),
        ("globe.frag", super::FRAG_SRC),
        ("space.frag", super::SPACE_FRAG_SRC),
        ("blit.frag", super::BLIT_FRAG_SRC),
        ("line.vert", super::LINE_VERT_SRC),
        ("line.frag", super::LINE_FRAG_SRC),
    ];
    const SOURCES: [&str; 6] = [
        VERT_SRC,
        FRAG_SRC,
        SPACE_FRAG_SRC,
        BLIT_FRAG_SRC,
        LINE_VERT_SRC,
        LINE_FRAG_SRC,
    ];

    #[test]
    fn minified_shaders_keep_structure_and_line_numbers() {
        for ((name, built), src) in MINIFIED.iter().zip(SOURCES) {
            // What is embedded is what minify() produces from the source.
            assert_eq!(*built, minify(src), "{name}: stale build output");
            assert!(
                built.starts_with("#version 300 es\n"),
                "{name}: #version not first"
            );
            assert_eq!(built.lines().count(), src.lines().count(), "{name}");
            assert!(!built.contains("//"), "{name}: comment left");
            for (n, (m, s)) in built.lines().zip(src.lines()).enumerate() {
                let code = s.split("//").next().unwrap().trim();
                assert_eq!(m, code, "{name}:{}", n + 1);
                // Preprocessor lines stay directives.
                if s.trim_start().starts_with('#') {
                    assert!(m.starts_with('#'), "{name}:{} lost directive", n + 1);
                }
            }
            // Dropping comments and indentation must not change the tokens.
            let toks = |t: &str| -> Vec<String> {
                t.lines()
                    .map(|l| {
                        let code = l.split("//").next().unwrap();
                        code.split_whitespace().collect::<Vec<_>>().join(" ")
                    })
                    .collect()
            };
            assert_eq!(toks(built), toks(src), "{name}: tokens changed");
        }
    }

    #[test]
    fn shader_sources_have_no_string_literals_or_block_comments() {
        // minify() splits on "//" and so assumes these never appear.
        for src in SOURCES {
            for line in src.lines() {
                let code = line.split("//").next().unwrap();
                assert!(!code.contains('"'), "string literal in shader");
                assert!(!line.contains("/*"), "block comment in shader");
            }
        }
    }

    fn dev(max: u32, mem: Option<f64>, coarse: bool) -> DeviceInfo {
        DeviceInfo {
            max_texture_size: max,
            device_memory_gb: mem,
            coarse_pointer: coarse,
        }
    }

    const MB: f64 = 1024.0 * 1024.0;

    #[test]
    fn desktop_keeps_full_resolution() {
        let b = choose_budget(&dev(16384, Some(8.0), false), None);
        assert_eq!((b.map_width, b.ids_width), (8192, 4096));
        // No deviceMemory (Safari, Firefox) on a fine pointer is a desktop.
        let b = choose_budget(&dev(8192, None, false), None);
        assert_eq!(b.map_width, 8192);
    }

    #[test]
    fn mobile_and_low_memory_get_the_small_tier() {
        for d in [
            dev(16384, None, true),
            dev(16384, Some(8.0), true),
            dev(16384, Some(4.0), false),
            dev(16384, Some(0.5), false),
            dev(4096, None, false),
            dev(4096, Some(8.0), false),
        ] {
            let b = choose_budget(&d, None);
            assert_eq!((b.map_width, b.ids_width), (4096, 2048), "{d:?}");
            let (map, ids) = budget_bytes(&b);
            assert!((map + ids) as f64 / MB < 25.0, "{d:?}");
        }
    }

    #[test]
    fn budget_bytes_matches_the_documented_figures() {
        let (map, ids) = budget_bytes(&TextureBudget {
            map_width: 8192,
            ids_width: 4096,
            imagery_width: 4096,
        });
        assert!((map as f64 / MB - 42.7).abs() < 0.1);
        assert_eq!(ids as f64 / MB, 8.0);
        let (map, ids) = budget_bytes(&TextureBudget {
            map_width: 4096,
            ids_width: 2048,
            imagery_width: 2048,
        });
        assert!((map as f64 / MB - 10.7).abs() < 0.1);
        assert_eq!(ids as f64 / MB, 2.0);
    }

    #[test]
    fn imagery_budget_per_tier() {
        let high = choose_budget(&dev(16384, Some(8.0), false), None);
        assert_eq!(high.imagery_width, 4096);
        assert!((imagery_bytes(&high) as f64 / MB - 42.7).abs() < 0.1);
        for d in [
            dev(16384, Some(2.0), true),
            dev(16384, Some(0.5), false),
            dev(4096, None, false),
        ] {
            let low = choose_budget(&d, None);
            assert_eq!(low.imagery_width, 2048, "{d:?}");
            assert!((imagery_bytes(&low) as f64 / MB - 10.7).abs() < 0.1);
        }
        // The forced tiers and the GPU limit apply to the imagery too.
        let forced = choose_budget(&dev(16384, Some(8.0), false), Some(TierOverride::Low));
        assert_eq!(forced.imagery_width, 2048);
        let small = choose_budget(&dev(1024, None, false), Some(TierOverride::High));
        assert_eq!(small.imagery_width, 1024);
    }

    #[test]
    fn sizes_never_exceed_the_gpu_limit() {
        let b = choose_budget(&dev(2048, None, false), None);
        assert_eq!((b.map_width, b.ids_width), (2048, 2048));
        let b = choose_budget(&dev(3000, None, false), Some(TierOverride::High));
        assert_eq!((b.map_width, b.ids_width), (2048, 2048));
        // Nonsense limits still give a usable size.
        let b = choose_budget(&dev(0, None, false), None);
        assert_eq!(b.map_width, 1024);
    }

    #[test]
    fn override_wins() {
        let phone = dev(16384, Some(2.0), true);
        assert_eq!(
            choose_budget(&phone, Some(TierOverride::High)).map_width,
            8192
        );
        let desktop = dev(16384, Some(8.0), false);
        assert_eq!(
            choose_budget(&desktop, Some(TierOverride::Low)).map_width,
            4096
        );
    }

    #[test]
    fn debug_ctxloss_query_parsing() {
        assert!(!parse_debug_ctxloss(""));
        assert!(parse_debug_ctxloss("?debug=ctxloss"));
        assert!(parse_debug_ctxloss("?tex=low&debug=ctxloss"));
        assert!(!parse_debug_ctxloss("?debug=ctxlossy"));
        assert!(!parse_debug_ctxloss("?debug=other"));
    }

    #[test]
    fn tier_query_parsing() {
        assert_eq!(parse_tier(""), None);
        assert_eq!(parse_tier("?tex=low"), Some(TierOverride::Low));
        assert_eq!(parse_tier("?a=1&tex=high"), Some(TierOverride::High));
        assert_eq!(parse_tier("?tex=huge"), None);
    }

    /// Every uniform the renderer looks up must be declared in the shader,
    /// or WebGL silently returns no location and the value is dropped.
    #[test]
    fn uniforms_are_declared_in_shader() {
        let declared: Vec<&str> = FRAG_SRC
            .lines()
            .chain(VERT_SRC.lines())
            .chain(MORPH_VERT_SRC.lines())
            .filter_map(|l| l.trim().strip_prefix("uniform "))
            .filter_map(|l| l.split(';').next()?.split_whitespace().last())
            .map(|n| n.split('[').next().unwrap_or(n))
            .collect();
        for name in [
            "u_res",
            "u_dist",
            "u_tanHalfFov",
            "u_rot",
            "u_map",
            "u_lines",
            "u_hasMap",
            "u_gridStep",
            "u_dpr",
            "u_layer",
            "u_imagery",
            "u_hasImagery",
            "u_ids",
            "u_highlight",
            "u_idColor",
            "u_political",
            "u_sun",
            "u_sunOn",
            "u_bounds",
            "u_scale",
            "u_mode",
            "u_proj",
            "u_flatPx",
            "u_cy",
            "u_lon0",
            "u_globe",
            "u_projA",
            "u_projB",
            "u_mix",
            "u_cyAB",
        ] {
            assert!(declared.contains(&name), "{name} missing from globe.frag");
            let lookup = format!("u(\"{name}\")");
            assert!(
                include_str!("render.rs").contains(&lookup),
                "{name} not looked up"
            );
        }
    }

    /// The space and blit programs share the vertex shader with the globe.
    #[test]
    fn space_and_blit_uniforms_are_declared() {
        let render_src = include_str!("render.rs");
        for (src, names, prefix) in [
            (SPACE_FRAG_SRC, ["u_dpr", "u_scale"], "su"),
            (BLIT_FRAG_SRC, ["u_src", "u_res"], "bu"),
        ] {
            for name in names {
                let decl = format!(" {name};");
                assert!(src.contains(&decl), "{name} undeclared");
                let lookup = format!("{prefix}(\"{name}\")");
                assert!(render_src.contains(&lookup), "{name} not looked up");
            }
            assert!(render_src.contains(&format!("{prefix}(\"u_bounds\")")));
        }
        assert!(VERT_SRC.contains(" u_bounds;"));
    }

    /// Text of a top-level GLSL function, from its signature to the closing
    /// brace at column 0.
    fn glsl_fn<'a>(src: &'a str, signature: &str) -> &'a str {
        let start = src.find(signature).expect("function present");
        let end = start + src[start..].find("\n}").expect("function end") + 2;
        &src[start..end]
    }

    /// Stars are drawn by two shaders (under the globe quad and outside it).
    /// They must agree exactly, or the quad edge would show.
    #[test]
    fn star_code_matches_between_globe_and_space_shaders() {
        for sig in ["float hash(vec2 p)", "vec3 space(vec2 frag)"] {
            assert_eq!(
                glsl_fn(FRAG_SRC, sig),
                glsl_fn(SPACE_FRAG_SRC, sig),
                "{sig}"
            );
        }
    }

    /// Same check for the line program, whose lookups use `lu(...)`.
    #[test]
    fn line_uniforms_are_declared_in_shaders() {
        let declared: Vec<&str> = LINE_VERT_SRC
            .lines()
            .chain(LINE_FRAG_SRC.lines())
            .filter_map(|l| l.trim().strip_prefix("uniform "))
            .filter_map(|l| l.split(';').next()?.split_whitespace().last())
            .map(|n| n.split('[').next().unwrap_or(n))
            .collect();
        for name in [
            "u_res",
            "u_dist",
            "u_tanHalfFov",
            "u_rot",
            "u_width",
            "u_strength",
            "u_lon0",
            "u_projA",
            "u_projB",
            "u_mix",
            "u_cyAB",
            "u_flatPx",
            "u_shift",
            "u_clip",
        ] {
            assert!(declared.contains(&name), "{name} missing from line shaders");
            let lookup = format!("lu(\"{name}\")");
            assert!(
                include_str!("render.rs").contains(&lookup),
                "{name} not looked up"
            );
        }
    }

    #[test]
    fn morph_grid_covers_the_sphere_and_fits_16_bit_indices() {
        let (verts, indices) = morph_grid();
        assert_eq!(verts.len() / 2, 361 * 181);
        assert!(verts.len() / 2 <= u16::MAX as usize + 1);
        assert_eq!(indices.len(), 360 * 180 * 6);
        assert!(indices.iter().all(|&i| (i as usize) < verts.len() / 2));
        let lats: Vec<f32> = verts.chunks(2).map(|v| v[0]).collect();
        let lons: Vec<f32> = verts.chunks(2).map(|v| v[1]).collect();
        let max = |v: &[f32]| v.iter().cloned().fold(f32::MIN, f32::max);
        let min = |v: &[f32]| v.iter().cloned().fold(f32::MAX, f32::min);
        assert!((max(&lats) - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!((min(&lats) + std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!((max(&lons) - std::f32::consts::PI).abs() < 1e-6);
        assert!((min(&lons) + std::f32::consts::PI).abs() < 1e-6);
    }

    #[test]
    fn blend_picks_the_shader_path() {
        assert_eq!(blend_mode(&Blend::settled(Projection::Globe)), MODE_GLOBE);
        assert_eq!(blend_mode(&Blend::settled(Projection::Robinson)), MODE_FLAT);
        let mid = Blend {
            a: Projection::Globe,
            b: Projection::Mercator,
            t: 0.4,
        };
        assert_eq!(blend_mode(&mid), MODE_MORPH);
        // The ends of a morph are the exact paths.
        assert_eq!(blend_mode(&Blend { t: 0.0, ..mid }), MODE_GLOBE);
        assert_eq!(blend_mode(&Blend { t: 1.0, ..mid }), MODE_FLAT);
        // The mode numbers are the ones globe.frag branches on.
        for (name, v) in [
            ("0", MODE_GLOBE),
            ("1", MODE_FLAT),
            ("2", MODE_MORPH),
            ("3", MODE_SPACE),
        ] {
            assert!(
                FRAG_SRC.contains(&format!("u_mode == {name}")) || name == "2",
                "{name}"
            );
            assert_eq!(v.to_string(), name);
        }
    }

    #[test]
    fn draws_are_blocked_only_while_lost() {
        let mut c = ContextState::default();
        assert!(!c.is_lost());
        c.on_lost();
        assert!(c.is_lost());
        assert!(!c.on_restored());
        assert!(!c.is_lost());
    }

    #[test]
    fn restore_reuploads_a_loaded_map() {
        let mut c = ContextState::default();
        assert!(c.on_map_loaded());
        c.on_lost();
        assert!(c.on_restored(), "loaded map must be uploaded again");
    }

    #[test]
    fn restore_before_map_loaded_uploads_nothing() {
        let mut c = ContextState::default();
        c.on_lost();
        assert!(!c.on_restored());
        // Data arriving later uploads immediately.
        assert!(c.on_map_loaded());
    }

    #[test]
    fn map_loaded_while_lost_is_deferred_until_restore() {
        let mut c = ContextState::default();
        c.on_lost();
        assert!(!c.on_map_loaded(), "no GL calls while lost");
        assert!(c.on_restored(), "deferred map uploads on restore");
    }

    #[test]
    fn repeated_loss_and_restore_cycles() {
        let mut c = ContextState::default();
        c.on_map_loaded();
        for _ in 0..3 {
            c.on_lost();
            c.on_lost(); // duplicate events are harmless
            assert!(c.on_restored());
            assert!(!c.is_lost());
        }
    }

    #[test]
    fn ids_loaded_while_lost_are_deferred_until_restore() {
        let mut c = ContextState::default();
        c.on_lost();
        assert!(!c.on_ids_loaded(), "no GL calls while lost");
        c.on_restored();
        assert!(c.ids_need_upload(), "deferred raster uploads on restore");
    }

    #[test]
    fn loaded_ids_are_reuploaded_on_each_restore() {
        let mut c = ContextState::default();
        assert!(!c.ids_need_upload());
        assert!(c.on_ids_loaded());
        for _ in 0..2 {
            c.on_lost();
            c.on_restored();
            assert!(c.ids_need_upload());
        }
    }

    #[test]
    fn shaders_target_glsl_es_300() {
        assert!(VERT_SRC.starts_with("#version 300 es"));
        assert!(FRAG_SRC.starts_with("#version 300 es"));
        assert!(MORPH_VERT_SRC.starts_with("#version 300 es"));
        assert!(LINE_VERT_SRC.starts_with("#version 300 es"));
        assert!(LINE_FRAG_SRC.starts_with("#version 300 es"));
    }
}
