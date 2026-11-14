#version 300 es
// Ray-traced globe. Each pixel casts a ray from the camera, intersects the
// unit sphere analytically, converts the hit point to latitude/longitude and
// samples an equirectangular map texture. Conventions match src/camera.rs.
precision highp float;

uniform vec2 u_res;           // framebuffer size, device pixels
uniform float u_dist;         // camera distance from globe center, radii
uniform float u_tanHalfFov;   // across the shorter screen axis
uniform mat3 u_rot;           // camera space -> world space
uniform sampler2D u_map;      // R = land mask, G = borders
uniform float u_hasMap;
uniform float u_gridStep;     // graticule spacing, degrees
uniform float u_dpr;
uniform int u_layer;          // 0 = political, 1 = monochrome (see view::Layer)
uniform sampler2D u_ids;      // R8 country IDs (id / 255), 0 = none; fetched, not filtered
uniform int u_highlight;      // country ID to highlight, 0 = none
uniform vec3 u_sun;           // unit vector toward the sun, world space
uniform float u_sunOn;        // 1 = shade the night side

out vec4 outColor;

const float PI = 3.141592653589793;

const vec3 OCEAN_DEEP = vec3(0.035, 0.115, 0.250);
const vec3 OCEAN_LIGHT = vec3(0.070, 0.200, 0.340);
const vec3 LAND = vec3(0.420, 0.530, 0.350);
const vec3 LAND_DRY = vec3(0.600, 0.570, 0.420);
const vec3 ICE = vec3(0.900, 0.925, 0.950);
const vec3 BORDER = vec3(0.150, 0.170, 0.130);
const vec3 COAST = vec3(0.780, 0.860, 0.900);
const vec3 MONO_OCEAN = vec3(0.015, 0.020, 0.030);
const vec3 MONO_LAND = vec3(0.180, 0.190, 0.210);
const vec3 MONO_BORDER = vec3(0.700, 0.730, 0.780);
const vec3 HIGHLIGHT = vec3(1.000, 0.780, 0.300);
const vec3 ATMOSPHERE = vec3(0.300, 0.560, 1.000);

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
    vec2 p = (gl_FragCoord.xy - 0.5 * u_res) / (0.5 * min(u_res.x, u_res.y));
    vec3 rd = normalize(vec3(p * u_tanHalfFov, -1.0));
    vec3 ro = vec3(0.0, 0.0, u_dist);

    // Closest approach of the ray to the globe center.
    float b = dot(ro, rd);
    float dmin2 = max(u_dist * u_dist - b * b, 0.0);
    float dmin = sqrt(dmin2);
    float aa = max(fwidth(dmin), 1e-6);
    float coverage = 1.0 - smoothstep(1.0 - aa, 1.0 + aa, dmin);

    vec3 bg = space(gl_FragCoord.xy);
    bg += ATMOSPHERE * 0.55 * exp(-max(dmin - 1.0, 0.0) * 22.0);

    // No early-out for background pixels: derivatives (fwidth, implicit mip
    // selection) need every pixel in a 2×2 quad on the same control path.
    float t = -b - sqrt(max(1.0 - dmin2, 0.0));
    vec3 n = normalize(ro + t * rd);
    vec3 w = u_rot * n;
    float lat = asin(clamp(w.y, -1.0, 1.0));
    float lon = atan(w.x, w.z);

    // Pick whichever of two longitude parameterizations is continuous across
    // this pixel quad, so mipmapping doesn't draw a seam at the antimeridian.
    float u1 = lon / (2.0 * PI) + 0.5;
    float u2 = fract(u1 + 0.5) - 0.5;
    float u = fwidth(u1) <= fwidth(u2) + 1e-6 ? u1 : u2;
    vec2 uv = vec2(u, 0.5 - lat / PI);

    vec2 m = texture(u_map, uv).rg * u_hasMap;

    // When the map is magnified, threshold the bilinear mask at 0.5 for crisp
    // vector-like edges. When minified, the mipmapped value already is an
    // antialiased coverage estimate, so use it directly.
    vec2 texSize = vec2(textureSize(u_map, 0));
    float texelsPerPx = max(fwidth(uv.x) * texSize.x, fwidth(uv.y) * texSize.y);
    float sharpen = (1.0 - smoothstep(0.6, 1.6, texelsPerPx)) * u_hasMap;
    float fw = max(fwidth(m.r), 1e-4);
    float land = mix(m.r, smoothstep(0.5 - fw, 0.5 + fw, m.r), sharpen);
    float coast = (1.0 - smoothstep(0.5 * fw, 1.6 * fw, abs(m.r - 0.5))) * sharpen;

    float latDeg = lat * 180.0 / PI;
    float alat = abs(latDeg);

    vec3 ocean = mix(OCEAN_LIGHT, OCEAN_DEEP, smoothstep(0.0, 70.0, alat) * 0.6 + 0.4);
    vec3 ground = mix(LAND, LAND_DRY, smoothstep(14.0, 22.0, alat) * (1.0 - smoothstep(30.0, 38.0, alat)) * 0.55);
    float iceLat = latDeg < 0.0 ? 62.0 : 72.0;
    ground = mix(ground, ICE, smoothstep(iceLat - 2.0, iceLat + 4.0, alat));
    vec3 borderCol = BORDER;
    float borderMix = 0.5;
    if (u_layer == 1) {
        // Monochrome: dark ocean, grey land, light borders.
        ocean = MONO_OCEAN;
        ground = MONO_LAND;
        borderCol = MONO_BORDER;
        borderMix = 0.8;
    }

    float fwg = max(fwidth(m.g), 1e-4);
    float borderLine = smoothstep(0.5 - fwg, 0.5 + fwg, m.g);
    float border = mix(m.g, borderLine, sharpen) * land;
    ground = mix(ground, borderCol, border * borderMix);

    // Highlighted country: bilinear blend of "texel is this country" so the
    // edge of the 4096-wide ID raster stays smooth when magnified. The branch
    // depends only on a uniform, so derivatives elsewhere are unaffected.
    if (u_highlight > 0) {
        ivec2 isz = textureSize(u_ids, 0);
        vec2 q = vec2(u * float(isz.x), uv.y * float(isz.y)) - 0.5;
        ivec2 i0 = ivec2(floor(q));
        vec2 f = fract(q);
        float h[4];
        for (int k = 0; k < 4; k++) {
            ivec2 c = i0 + ivec2(k & 1, k >> 1);
            c.x = ((c.x % isz.x) + isz.x) % isz.x;
            c.y = clamp(c.y, 0, isz.y - 1);
            float id = floor(texelFetch(u_ids, c, 0).r * 255.0 + 0.5);
            h[k] = id == float(u_highlight) ? 1.0 : 0.0;
        }
        float mask = mix(mix(h[0], h[1], f.x), mix(h[2], h[3], f.x), f.y);
        float hl = smoothstep(0.35, 0.65, mask) * land * u_hasMap;
        ground = mix(ground, HIGHLIGHT, 0.42 * hl);
    }

    vec3 col = mix(ocean, ground, land);
    col = mix(col, COAST, coast * 0.35);

    // Graticule with constant on-screen width; meridians fade near the poles
    // where they converge.
    vec2 g = vec2(u * 360.0, latDeg + 90.0) / u_gridStep;
    vec2 gw = max(fwidth(g), vec2(1e-5));
    vec2 gd = abs(fract(g - 0.5) - 0.5) / (gw * 0.9);
    float meridian = (1.0 - min(gd.x, 1.0)) * (1.0 - smoothstep(70.0, 86.0, alat));
    float parallel = 1.0 - min(gd.y, 1.0);
    col = mix(col, vec3(0.85, 0.90, 1.0), max(meridian, parallel) * 0.14);

    // Soft key light from the upper left of the camera plus ocean glint.
    vec3 L = normalize(vec3(-0.45, 0.55, 0.85));
    float diffuse = max(dot(n, L), 0.0);
    col *= 0.38 + 0.72 * diffuse;
    vec3 refl = reflect(-L, n);
    col += vec3(0.55, 0.65, 0.80) * pow(max(dot(refl, -rd), 0.0), 36.0) * 0.22 * (1.0 - land);

    // Day/night terminator. sunElev is the sine of the sun's elevation at
    // this point; the smooth band spans roughly civil to nautical twilight
    // (about +2 to -10 degrees of sun elevation).
    float sunElev = dot(w, u_sun);
    float night = (1.0 - smoothstep(-0.17, 0.035, sunElev)) * u_sunOn;
    vec3 nightTint = u_layer == 1 ? vec3(0.50, 0.53, 0.62) : vec3(0.22, 0.26, 0.42);
    col *= mix(vec3(1.0), nightTint, night);

    // Atmospheric scattering toward the limb.
    float fres = pow(1.0 - max(dot(n, -rd), 0.0), 3.0);
    col = mix(col, ATMOSPHERE, fres * 0.55);

    outColor = vec4(mix(bg, col, coverage), 1.0);
}
