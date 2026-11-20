#version 300 es
// A star: a soft disk of radius v_radius with a faint halo, added to the sky.
precision highp float;

in vec3 v_color;
in float v_radius;
in float v_size;

out vec4 outColor;

void main() {
    float r = length(gl_PointCoord - 0.5) * v_size;   // device pixels from the center
    float core = 1.0 - smoothstep(v_radius - 0.5, v_radius + 0.5, r);
    float halo = exp(-r * r / (v_radius * v_radius * 2.5)) * 0.35;
    outColor = vec4(v_color * max(core, halo), 1.0);
}
