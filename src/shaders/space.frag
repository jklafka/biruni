#version 300 es
// Empty space with stars, for everything outside the globe quad. The hash
// and `space` function are copied verbatim from globe.frag (a unit test
// checks they match), so the two passes agree to the last bit.
precision highp float;

uniform float u_dpr;
uniform float u_scale;        // render scale; stars are placed in full-resolution pixels

out vec4 outColor;

float hash(vec2 p) {
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
}

vec3 space(vec2 frag) {
    vec3 col = vec3(0.006, 0.010, 0.024);
    vec2 cell = floor(frag / (2.0 * u_dpr));
    float h = hash(cell);
    if (h > 0.9975) {
        col += vec3(0.55, 0.60, 0.75) * (h - 0.9975) / 0.0025;
    }
    return col;
}

void main() {
    outColor = vec4(space(gl_FragCoord.xy / u_scale), 1.0);
}
