#version 300 es
// Upscale the reduced-resolution globe render onto the canvas.
precision highp float;

uniform sampler2D u_src;      // LINEAR filtered
uniform vec2 u_res;           // canvas size, device pixels

out vec4 outColor;

void main() {
    outColor = vec4(texture(u_src, gl_FragCoord.xy / u_res).rgb, 1.0);
}
