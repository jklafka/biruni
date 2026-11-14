#version 300 es
// Antialiased capsule around the segment: coverage from the distance to the
// segment in pixels. Round ends make joins between segments seamless; the
// renderer blends with MAX so overlaps do not double up.
precision highp float;

uniform float u_width;        // line width, device pixels
uniform float u_strength;     // overall opacity of this layer, 0..1

flat in vec2 v_a;
flat in vec2 v_b;
in float v_front;

out vec4 outColor;

void main() {
    // Hidden behind the horizon. v_front is linear along the segment, so the
    // cut lands within a pixel of the true limb.
    if (v_front < 0.0) discard;
    vec2 p = gl_FragCoord.xy;
    vec2 ab = v_b - v_a;
    float l2 = dot(ab, ab);
    float t = l2 > 1e-8 ? clamp(dot(p - v_a, ab) / l2, 0.0, 1.0) : 0.0;
    float d = length(p - (v_a + t * ab));
    float cov = clamp(0.5 * u_width + 0.5 - d, 0.0, 1.0) * u_strength;
    outColor = vec4(cov);
}
