//! WebGL2 renderer: one full-screen triangle and a ray-tracing fragment shader.

use crate::camera::{Camera, TAN_HALF_FOV};
use crate::geo::Layers;
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

struct Uniforms {
    res: Option<WebGlUniformLocation>,
    dist: Option<WebGlUniformLocation>,
    tan_half_fov: Option<WebGlUniformLocation>,
    rot: Option<WebGlUniformLocation>,
    map: Option<WebGlUniformLocation>,
    has_map: Option<WebGlUniformLocation>,
    grid_step: Option<WebGlUniformLocation>,
    dpr: Option<WebGlUniformLocation>,
}

pub struct Renderer {
    gl: GL,
    program: WebGlProgram,
    vao: WebGlVertexArrayObject,
    uniforms: Uniforms,
    map: WebGlTexture,
    has_map: bool,
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

        let vs = compile(&gl, GL::VERTEX_SHADER, VERT_SRC)?;
        let fs = compile(&gl, GL::FRAGMENT_SHADER, FRAG_SRC)?;
        let program = link(&gl, &vs, &fs)?;
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

        Ok(Self {
            gl,
            program,
            vao,
            uniforms,
            map,
            has_map: false,
        })
    }

    /// Width of the map texture to build: 8192×4096 where the GPU allows it.
    fn map_width(&self) -> u32 {
        let max = self
            .gl
            .get_parameter(GL::MAX_TEXTURE_SIZE)
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(4096.0) as u32;
        if max >= 8192 {
            8192
        } else {
            4096
        }
    }

    /// Rasterize the vector layers into a mipmapped equirectangular texture.
    ///
    /// The map is drawn as two square halves so no single canvas exceeds the
    /// 16.7 MP limit some mobile browsers enforce.
    pub fn upload_map(&mut self, layers: &Layers) -> Result<(), JsValue> {
        let gl = &self.gl;
        let width = self.map_width();
        let height = width / 2;
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

    pub fn draw(&self, camera: &Camera, width_px: u32, height_px: u32, dpr: f64) {
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
        gl.active_texture(GL::TEXTURE0);
        gl.bind_texture(GL::TEXTURE_2D, Some(&self.map));
        gl.uniform1i(u.map.as_ref(), 0);
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
