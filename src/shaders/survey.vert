#version 300 es
// Deep-sky survey tiles (src/survey.rs): mesh vertices are equatorial unit
// vectors with texture coordinates, placed with the stereographic
// projection of src/sky.rs. The CPU culls tiles far from the view, so no
// vertex comes near the point behind it.
uniform mat3 u_eqToCam;       // equatorial -> camera
uniform vec2 u_res;           // framebuffer size, device pixels
uniform float u_halfShort;    // half the shorter axis, device pixels
uniform float u_tanQ;         // tan(fov / 4)

layout(location = 0) in vec3 a_dir;
layout(location = 1) in vec2 a_uv;

out vec2 v_uv;
out vec3 v_dir;

void main() {
    vec3 cam = u_eqToCam * a_dir;
    float d = max(1.0 - cam.z, 1e-4);
    vec2 p = cam.xy / (d * u_tanQ) * u_halfShort;
    gl_Position = vec4(p / (0.5 * u_res), 0.0, 1.0);
    v_uv = a_uv;
    v_dir = a_dir;
}
