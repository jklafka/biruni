#version 300 es
// One instance per line segment: endpoints a and b on the unit sphere (world
// space). Each is projected with the same camera model as globe.frag, then
// the segment is expanded into a screen-space quad whose width is constant in
// pixels, whatever the zoom. Conventions match src/camera.rs.
precision highp float;

uniform vec2 u_res;           // framebuffer size, device pixels
uniform float u_dist;         // camera distance from globe center, radii
uniform float u_tanHalfFov;   // across the shorter screen axis
uniform mat3 u_rot;           // camera space -> world space
uniform float u_width;        // line width, device pixels

layout(location = 0) in vec2 a_corner;  // x: 0 = at a, 1 = at b; y: -1 or +1 side
layout(location = 1) in vec3 a_a;
layout(location = 2) in vec3 a_b;

flat out vec2 v_a;            // endpoints in pixels (gl_FragCoord space)
flat out vec2 v_b;
out float v_front;            // > 0 on the camera's side of the horizon

vec2 project(vec3 c) {
    vec2 q = c.xy / ((u_dist - c.z) * u_tanHalfFov);
    return 0.5 * u_res + q * 0.5 * min(u_res.x, u_res.y);
}

void main() {
    // transpose(u_rot) * p: world space -> camera space.
    vec3 ca = a_a * u_rot;
    vec3 cb = a_b * u_rot;
    // A point on the unit sphere faces the camera when dot(p, eye - p) > 0,
    // i.e. z * dist > 1 in camera space.
    float fa = ca.z * u_dist - 1.0;
    float fb = cb.z * u_dist - 1.0;
    if (fa < 0.0 && fb < 0.0) {
        gl_Position = vec4(2.0, 2.0, 2.0, 1.0);  // clipped: wholly behind the limb
        v_a = v_b = vec2(0.0);
        v_front = -1.0;
        return;
    }
    vec2 sa = project(ca);
    vec2 sb = project(cb);
    vec2 d = sb - sa;
    float l = length(d);
    vec2 dir = l > 1e-4 ? d / l : vec2(1.0, 0.0);
    vec2 nrm = vec2(-dir.y, dir.x);
    float half_ = 0.5 * u_width + 1.5;  // line plus antialiasing margin
    vec2 pos = mix(sa, sb, a_corner.x)
        + dir * (a_corner.x * 2.0 - 1.0) * half_
        + nrm * a_corner.y * half_;
    v_a = sa;
    v_b = sb;
    v_front = mix(fa, fb, a_corner.x);
    gl_Position = vec4(pos / u_res * 2.0 - 1.0, 0.0, 1.0);
}
