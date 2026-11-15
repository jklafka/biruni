#version 300 es
// Screen-space quad (triangle strip) between two NDC corners. The globe
// pass draws only the rectangle that bounds the globe and its halo; the
// space and blit passes draw the whole screen. All globe shading happens
// per pixel in globe.frag. (v_ll and v_front are filled in by morph.vert;
// here they only keep the interface the same.)
uniform vec4 u_bounds;        // x0, y0, x1, y1 in normalized device coordinates

out vec2 v_ll;
out float v_front;

void main() {
    vec2 c = vec2(float(gl_VertexID & 1), float((gl_VertexID >> 1) & 1));
    v_ll = vec2(0.0);
    v_front = 1.0;
    gl_Position = vec4(mix(u_bounds.xy, u_bounds.zw, c), 0.0, 1.0);
}
