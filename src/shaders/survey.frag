#version 300 es
// A survey tile, added to the sky: the survey's own sky background (u_black)
// is taken off so the night stays dark; nothing below the horizon.
precision highp float;

uniform sampler2D u_tex;
uniform vec3 u_zenith;        // the zenith, in equatorial space
uniform float u_strength;     // 0..1 (fades out by day)
uniform float u_black;        // background level to remove, 0..1

in vec2 v_uv;
in vec3 v_dir;

out vec4 outColor;

void main() {
    // Sample before deciding to discard: the implicit level of detail needs
    // every pixel of the quad.
    vec3 c = texture(u_tex, v_uv).rgb;
    if (dot(normalize(v_dir), u_zenith) < 0.0) {
        discard;
    }
    c = max(c - u_black, 0.0) / (1.0 - u_black);
    outColor = vec4(c * u_strength, 1.0);
}
