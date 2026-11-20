#version 300 es
// Stars of the sky view as points. Each vertex is a star's equatorial unit
// vector, its magnitude and color (src/stars.rs). The projection is the
// stereographic one of src/sky.rs and sky.frag.
uniform mat3 u_eqToCam;       // equatorial -> camera
uniform vec3 u_zenith;        // the zenith, in equatorial space
uniform vec2 u_res;           // framebuffer size, device pixels
uniform float u_halfShort;    // half the shorter axis, device pixels
uniform float u_tanQ;         // tan(fov / 4)
uniform float u_dpr;
uniform float u_magLimit;     // faintest magnitude shown

layout(location = 0) in vec3 a_dir;
layout(location = 1) in float a_mag;
layout(location = 2) in vec3 a_color;

out vec3 v_color;
out float v_radius;           // core radius, device pixels
out float v_size;             // gl_PointSize

void main() {
    vec3 cam = u_eqToCam * a_dir;
    float d = 1.0 - cam.z;
    float alt = dot(a_dir, u_zenith);
    float dm = u_magLimit - a_mag;
    // Behind the view, below the horizon or too faint: outside the clip box.
    if (d < 1e-6 || alt < 0.0 || dm < 0.0) {
        gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
        gl_PointSize = 1.0;
        v_color = vec3(0.0);
        v_radius = 1.0;
        v_size = 1.0;
        return;
    }
    vec2 p = cam.xy / (d * u_tanQ) * u_halfShort;
    gl_Position = vec4(p / (0.5 * u_res), 0.0, 1.0);
    // Brighter stars are bigger and stronger; stars near the limit fade in.
    // Near the horizon the air dims them (a rough extinction).
    float radius = clamp(0.45 + 0.4 * dm, 0.5, 4.0) * u_dpr;
    float strength = clamp(0.12 + dm / 2.5, 0.0, 1.0) * smoothstep(0.0, 0.12, alt);
    v_color = a_color * strength;
    v_radius = radius;
    v_size = 2.0 * radius + 2.0 * u_dpr;
    gl_PointSize = v_size;
}
