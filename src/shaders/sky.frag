#version 300 es
// The sky view's background: each pixel is inverted through the
// stereographic projection to a direction (see src/sky.rs, which has the
// same math and its tests), then turned into horizontal space for the
// ground and into equatorial space for the grid.
precision highp float;

uniform vec2 u_res;           // framebuffer size, device pixels
uniform float u_halfShort;    // half the shorter axis, device pixels
uniform float u_tanQ;         // tan(fov / 4)
uniform mat3 u_camToHor;      // camera -> horizontal (SkyCamera::basis)
uniform mat3 u_horToEq;       // horizontal -> equatorial J2000
uniform float u_dpr;
uniform float u_gridStep;     // radians between grid lines, 0 for no grid
uniform float u_daylight;     // 0 at night, 1 in full day (sky_bodies::daylight)
uniform sampler2D u_milkyWay; // equirectangular RA/Dec (scripts/make_milky_way.py)
uniform float u_milkyWayOn;   // strength, 0 until the image is loaded
uniform float u_milkyWayTexel; // radians per texel of the image's level 0

out vec4 outColor;

const float PI = 3.14159265358979;
const vec3 SKY = vec3(0.008, 0.012, 0.030);
const vec3 GLOW = vec3(0.030, 0.040, 0.070);
const vec3 GROUND = vec3(0.028, 0.030, 0.026);
const vec3 DAY_ZENITH = vec3(0.24, 0.42, 0.74);
const vec3 DAY_HORIZON = vec3(0.58, 0.70, 0.84);
const vec3 DAY_GROUND = vec3(0.10, 0.11, 0.09);
const vec3 GRID = vec3(0.30, 0.42, 0.70);
const vec3 HORIZON = vec3(0.45, 0.50, 0.55);

// Coverage of a line `d` radians away whose width is `w` radians.
float line(float d, float w) {
    return 1.0 - smoothstep(0.5 * w, 1.5 * w, d);
}

void main() {
    vec2 p = (gl_FragCoord.xy - 0.5 * u_res) / u_halfShort;
    float r = length(p);
    float t = r * u_tanQ;            // tan(angle from the view center / 2)
    float k = 1.0 / (1.0 + t * t);
    vec2 side = r > 0.0 ? p / r * (2.0 * t * k) : vec2(0.0);
    vec3 cam = vec3(side, -(1.0 - t * t) * k);
    vec3 hor = u_camToHor * cam;
    vec3 eq = u_horToEq * hor;
    // Radians per device pixel here (the stereographic scale grows outward).
    float radPerPx = 2.0 * u_tanQ * k / u_halfShort;
    float px = radPerPx * u_dpr; // one CSS pixel

    float alt = asin(clamp(hor.y, -1.0, 1.0));
    vec3 col = SKY + GLOW * exp(-max(alt, 0.0) / 0.12);
    if (u_milkyWayOn > 0.0) {
        // u = 0.5 - RA / 2pi, v = 0.5 - Dec / pi; the level of detail is
        // computed, not taken from derivatives, so the RA wrap has no seam.
        float ra = atan(eq.x, eq.z);
        float dec = asin(clamp(eq.y, -1.0, 1.0));
        vec2 uv = vec2(fract(0.5 - ra / (2.0 * PI)), 0.5 - dec / PI);
        float lod = max(log2(radPerPx / u_milkyWayTexel), 0.0);
        col += textureLod(u_milkyWay, uv, lod).rgb * u_milkyWayOn * (1.0 - u_daylight);
    }
    vec3 day = mix(DAY_ZENITH, DAY_HORIZON, exp(-max(alt, 0.0) / 0.25));
    col = mix(col, day, u_daylight);

    if (u_gridStep > 0.0) {
        float dec = asin(clamp(eq.y, -1.0, 1.0));
        float ra = atan(eq.x, eq.z);
        float s = u_gridStep;
        float dDec = abs(dec - s * floor(dec / s + 0.5));
        float dRa = abs(ra - s * floor(ra / s + 0.5)) * cos(dec);
        // Meridians crowd toward the poles: fade them once neighbours are
        // closer than about 12 pixels, so the pole is not a solid blob.
        float spread = smoothstep(6.0, 12.0, s * cos(dec) / radPerPx);
        float g = max(line(dDec, px), line(dRa, px) * spread);
        float equator = line(abs(dec), 1.5 * px);
        col = mix(col, GRID, 0.14 * g + 0.2 * equator);
    }

    // Ground below the horizon, antialiased over a pixel, with a thin line.
    float ground = smoothstep(-px, px, -alt);
    col = mix(col, mix(GROUND, DAY_GROUND, u_daylight), ground);
    col = mix(col, HORIZON, 0.6 * line(abs(alt), 1.5 * px));
    outColor = vec4(col, 1.0);
}
