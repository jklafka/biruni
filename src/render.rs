//! WebGL2 renderer: one full-screen triangle and a ray-tracing fragment shader.

use crate::camera::{Camera, TAN_HALF_FOV};
use crate::geo::Layers;
use crate::view::Layer;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    HtmlCanvasElement, WebGl2RenderingContext as GL, WebGlProgram, WebGlShader, WebGlTexture,
    WebGlUniformLocation, WebGlVertexArrayObject,
};

const VERT_SRC: &str = include_str!("shaders/globe.vert");
const FRAG_SRC: &str = include_str!("shaders/globe.frag");

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
    /// Map texture width (height is half). RG8 with a full mip chain.
    pub map_width: u32,
    /// Country-ID raster width (height is half). R8, no mipmaps.
    pub ids_width: usize,
}

/// Devices with at most this much RAM take the small tier.
const LOW_MEMORY_GB: f64 = 4.0;

/// Pick texture sizes. Capable desktops keep 8192 (crisp coastlines at the
/// closest zoom). Touch devices, low-memory devices and GPUs limited to 4096
/// get a 4096 map (about 21 MB with mips) and a 2048 ID raster (2 MB): at
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
    let (map, ids) = if low { (4096, 2048) } else { (8192, 4096) };
    // Largest power of two the GPU supports (at least 1024).
    let cap = if d.max_texture_size >= 1024 {
        1 << (31 - d.max_texture_size.leading_zeros())
    } else {
        1024
    };
    TextureBudget {
        map_width: map.min(cap),
        ids_width: (ids as u32).min(cap) as usize,
    }
}

/// GPU bytes for the map texture (RG8, full mip chain) and the ID raster
/// (R8). The mip chain adds a third of the base level.
pub fn budget_bytes(b: &TextureBudget) -> (u64, u64) {
    let (w, h) = (b.map_width as u64, b.map_width as u64 / 2);
    let mut map = 0;
    let (mut lw, mut lh) = (w, h);
    loop {
        map += lw * lh * 2;
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

struct Uniforms {
    res: Option<WebGlUniformLocation>,
    dist: Option<WebGlUniformLocation>,
    tan_half_fov: Option<WebGlUniformLocation>,
    rot: Option<WebGlUniformLocation>,
    map: Option<WebGlUniformLocation>,
    has_map: Option<WebGlUniformLocation>,
    grid_step: Option<WebGlUniformLocation>,
    dpr: Option<WebGlUniformLocation>,
    layer: Option<WebGlUniformLocation>,
    ids: Option<WebGlUniformLocation>,
    highlight: Option<WebGlUniformLocation>,
    sun: Option<WebGlUniformLocation>,
    sun_on: Option<WebGlUniformLocation>,
}

/// GL objects that die with the context and must be rebuilt after a restore.
struct Resources {
    program: WebGlProgram,
    vao: WebGlVertexArrayObject,
    uniforms: Uniforms,
    map: WebGlTexture,
    /// Country-ID raster (R8, one byte per texel, 0 = no country).
    ids: WebGlTexture,
}

impl Resources {
    fn create(gl: &GL) -> Result<Self, JsValue> {
        let vs = compile(gl, GL::VERTEX_SHADER, VERT_SRC)?;
        let fs = compile(gl, GL::FRAGMENT_SHADER, FRAG_SRC)?;
        let program = link(gl, &vs, &fs)?;
        let u = |name: &str| gl.get_uniform_location(&program, name);
        let uniforms = Uniforms {
            res: u("u_res"),
            dist: u("u_dist"),
            tan_half_fov: u("u_tanHalfFov"),
            rot: u("u_rot"),
            map: u("u_map"),
            has_map: u("u_hasMap"),
            grid_step: u("u_gridStep"),
            dpr: u("u_dpr"),
            layer: u("u_layer"),
            ids: u("u_ids"),
            highlight: u("u_highlight"),
            sun: u("u_sun"),
            sun_on: u("u_sunOn"),
        };
        let vao = gl.create_vertex_array().ok_or("failed to create VAO")?;

        // 1×1 placeholder until the vector data has been rasterized.
        let map = gl.create_texture().ok_or("failed to create texture")?;
        gl.bind_texture(GL::TEXTURE_2D, Some(&map));
        gl.pixel_storei(GL::UNPACK_ALIGNMENT, 1);
        gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
            GL::TEXTURE_2D,
            0,
            GL::RG8 as i32,
            1,
            1,
            0,
            GL::RG,
            GL::UNSIGNED_BYTE,
            Some(&[0, 0]),
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

        Ok(Self {
            program,
            vao,
            uniforms,
            map,
            ids,
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

pub struct Renderer {
    gl: GL,
    program: WebGlProgram,
    vao: WebGlVertexArrayObject,
    uniforms: Uniforms,
    map: WebGlTexture,
    has_map: bool,
    layer: Layer,
    /// World-space direction to the sun, or `None` when the terminator is off.
    sun: Option<[f32; 3]>,
    ctx: ContextState,
    /// Country-ID raster texture (placeholder until uploaded).
    ids: WebGlTexture,
    /// ID of the highlighted country, 0 for none.
    highlight: u8,
    /// Texture sizes, chosen once from the device and kept across context loss.
    budget: TextureBudget,
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
        Ok(Self {
            gl,
            program: r.program,
            vao: r.vao,
            uniforms: r.uniforms,
            map: r.map,
            has_map: false,
            layer: Layer::default(),
            sun: None,
            ctx: ContextState::default(),
            ids: r.ids,
            highlight: 0,
            budget,
        })
    }

    /// Width of the country-ID raster this device uses. Rasterize the IDs at
    /// this width before calling [`Self::upload_country_ids`].
    pub fn ids_width(&self) -> usize {
        self.budget.ids_width
    }

    pub fn is_lost(&self) -> bool {
        self.ctx.is_lost()
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
        self.vao = r.vao;
        self.uniforms = r.uniforms;
        self.map = r.map;
        self.ids = r.ids;
        self.has_map = false;
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

    /// Rasterize the vector layers into a mipmapped equirectangular texture
    /// whose width comes from [`choose_budget`].
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

        let doc = web_sys::window()
            .ok_or("no window")?
            .document()
            .ok_or("no document")?;
        let tile: HtmlCanvasElement = doc.create_element("canvas")?.dyn_into()?;
        tile.set_width(height);
        tile.set_height(height);

        let tex = gl.create_texture().ok_or("failed to create texture")?;
        gl.bind_texture(GL::TEXTURE_2D, Some(&tex));
        gl.tex_storage_2d(GL::TEXTURE_2D, levels, GL::RG8, width as i32, height as i32);
        gl.pixel_storei(GL::UNPACK_COLORSPACE_CONVERSION_WEBGL, GL::NONE as i32);
        gl.pixel_storei(GL::UNPACK_PREMULTIPLY_ALPHA_WEBGL, 0);
        for x in [0, height] {
            layers.draw_tile(&tile, width as f64, height as f64, x as f64)?;
            gl.tex_sub_image_2d_with_u32_and_u32_and_html_canvas_element(
                GL::TEXTURE_2D,
                0,
                x as i32,
                0,
                GL::RG,
                GL::UNSIGNED_BYTE,
                &tile,
            )?;
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

        gl.delete_texture(Some(&self.map));
        self.map = tex;
        self.has_map = true;
        Ok(())
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

    pub fn set_layer(&mut self, layer: Layer) {
        self.layer = layer;
    }

    pub fn set_sun(&mut self, sun: Option<[f32; 3]>) {
        self.sun = sun;
    }

    pub fn draw(&self, camera: &Camera, width_px: u32, height_px: u32, dpr: f64) {
        if self.ctx.is_lost() {
            return;
        }
        let gl = &self.gl;
        let u = &self.uniforms;
        gl.viewport(0, 0, width_px as i32, height_px as i32);
        gl.use_program(Some(&self.program));
        gl.uniform2f(u.res.as_ref(), width_px as f32, height_px as f32);
        gl.uniform1f(u.dist.as_ref(), camera.dist() as f32);
        gl.uniform1f(u.tan_half_fov.as_ref(), TAN_HALF_FOV as f32);
        gl.uniform_matrix3fv_with_f32_array(u.rot.as_ref(), false, &camera.rotation());
        gl.uniform1f(u.has_map.as_ref(), if self.has_map { 1.0 } else { 0.0 });
        gl.uniform1f(u.grid_step.as_ref(), camera.grid_step_deg() as f32);
        gl.uniform1f(u.dpr.as_ref(), dpr as f32);
        gl.uniform1i(u.layer.as_ref(), self.layer.shader_index());
        let sun = self.sun.unwrap_or([0.0, 0.0, 1.0]);
        gl.uniform3f(u.sun.as_ref(), sun[0], sun[1], sun[2]);
        gl.uniform1f(
            u.sun_on.as_ref(),
            if self.sun.is_some() { 1.0 } else { 0.0 },
        );
        gl.active_texture(GL::TEXTURE0);
        gl.bind_texture(GL::TEXTURE_2D, Some(&self.map));
        gl.uniform1i(u.map.as_ref(), 0);
        gl.active_texture(GL::TEXTURE1);
        gl.bind_texture(GL::TEXTURE_2D, Some(&self.ids));
        gl.uniform1i(u.ids.as_ref(), 1);
        gl.uniform1i(u.highlight.as_ref(), i32::from(self.highlight));
        gl.active_texture(GL::TEXTURE0);
        gl.bind_vertex_array(Some(&self.vao));
        gl.draw_arrays(GL::TRIANGLES, 0, 3);
    }
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
        });
        assert!((map as f64 / MB - 85.3).abs() < 0.1);
        assert_eq!(ids as f64 / MB, 8.0);
        let (map, ids) = budget_bytes(&TextureBudget {
            map_width: 4096,
            ids_width: 2048,
        });
        assert!((map as f64 / MB - 21.3).abs() < 0.1);
        assert_eq!(ids as f64 / MB, 2.0);
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
            .filter_map(|l| l.trim().strip_prefix("uniform "))
            .filter_map(|l| l.split(';').next()?.split_whitespace().last())
            .collect();
        for name in [
            "u_res",
            "u_dist",
            "u_tanHalfFov",
            "u_rot",
            "u_map",
            "u_hasMap",
            "u_gridStep",
            "u_dpr",
            "u_layer",
            "u_ids",
            "u_highlight",
            "u_sun",
            "u_sunOn",
        ] {
            assert!(declared.contains(&name), "{name} missing from globe.frag");
            let lookup = format!("u(\"{name}\")");
            assert!(
                include_str!("render.rs").contains(&lookup),
                "{name} not looked up"
            );
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
    }
}
