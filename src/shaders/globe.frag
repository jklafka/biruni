#version 300 es
// Ray-traced globe. Each pixel casts a ray from the camera, intersects the
// unit sphere analytically, converts the hit point to latitude/longitude and
// samples an equirectangular map texture. Conventions match src/camera.rs.
precision highp float;

uniform vec2 u_res;           // framebuffer size, device pixels
uniform float u_dist;         // camera distance from globe center, radii
uniform float u_tanHalfFov;   // across the shorter screen axis
uniform mat3 u_rot;           // camera space -> world space
uniform sampler2D u_map;      // R = land mask (lakes cut out)
uniform sampler2D u_lines;    // RG8, screen-sized: R = coastlines, G = borders (line pass)
uniform float u_hasMap;
uniform float u_gridStep;     // graticule spacing, degrees
uniform float u_dpr;
uniform float u_scale;        // render scale (1 = full resolution); stars stay in full-res pixels
uniform int u_layer;          // 0 = political, 1 = monochrome (see view::Layer)
uniform sampler2D u_ids;      // R8 country IDs (id / 255), 0 = none; fetched, not filtered
uniform int u_highlight;      // country ID to highlight, 0 = none
uniform vec3 u_sun;           // unit vector toward the sun, world space
uniform float u_sunOn;        // 1 = shade the night side
uniform int u_mode;           // 0 = ray-traced globe, 1 = flat map per pixel, 2 = morph mesh, 3 = space only
uniform int u_proj;           // mode 1: flat projection id (see src/projection.rs)
uniform float u_flatPx;       // flat maps: framebuffer pixels per plane unit
uniform float u_cy;           // mode 1: plane y of the screen-center row
uniform float u_lon0;         // flat maps: central meridian, radians
uniform float u_globe;        // mode 2: weight of the globe in the blend, 0..1

in vec2 v_ll;                 // mode 2: latitude, longitude relative to the central meridian
in float v_front;             // mode 2: > 0 where the surface is visible

out vec4 outColor;

const float PI = 3.141592653589793;
const float MERC_MAX = 1.4844222297453324;   // Mercator stops at 85.0511 degrees
const float ROB_XS = 0.8487;
const float ROB_YS = 1.3523;
const float MOLL_X = 0.9003163161571061;     // 2 sqrt(2) / pi
// Robinson table every 5 degrees of latitude, equator to pole.
const float ROB_X[19] = float[19](
    1.0000, 0.9986, 0.9954, 0.9900, 0.9822, 0.9730, 0.9600, 0.9427, 0.9216, 0.8962,
    0.8679, 0.8350, 0.7986, 0.7597, 0.7186, 0.6732, 0.6213, 0.5722, 0.5322);
const float ROB_Y[19] = float[19](
    0.0000, 0.0620, 0.1240, 0.1860, 0.2480, 0.3100, 0.3720, 0.4340, 0.4958, 0.5571,
    0.6176, 0.6769, 0.7346, 0.7903, 0.8435, 0.8936, 0.9394, 0.9761, 1.0000);

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

// Inverse of the flat projections (see src/projection.rs; the forward
// functions are in morph.vert and line.vert). q is the plane position, x from
// the central meridian. Returns latitude and longitude relative to the
// central meridian, and the coverage of the map at this pixel: 1 on the map,
// 0 outside its outline, antialiased across the edge.
float flatInverse(int id, vec2 q, out float lat, out float lon) {
    float px = 1.0 / u_flatPx;  // plane units per pixel
    float cov = 1.0;
    if (id == 1) {
        lat = clamp(q.y, -0.5 * PI, 0.5 * PI);
        lon = q.x;
        cov = clamp((0.5 * PI - abs(q.y)) / px + 0.5, 0.0, 1.0);
    } else if (id == 2) {
        lat = 2.0 * atan(exp(clamp(q.y, -PI, PI))) - 0.5 * PI;
        lon = q.x;
        cov = clamp((PI - abs(q.y)) / px + 0.5, 0.0, 1.0);
    } else if (id == 3) {
        float v = clamp(abs(q.y) / ROB_YS, 0.0, 1.0);
        int i = 0;
        for (int k = 0; k < 17; k++) {
            if (v > ROB_Y[k + 1]) i = k + 1;
        }
        float f = (v - ROB_Y[i]) / (ROB_Y[i + 1] - ROB_Y[i]);
        lat = sign(q.y) * (float(i) + f) * (0.5 * PI / 18.0);
        lon = q.x / (ROB_XS * mix(ROB_X[i], ROB_X[i + 1], f));
        cov = clamp((ROB_YS - abs(q.y)) / px + 0.5, 0.0, 1.0);
    } else {
        float th = asin(clamp(q.y / 1.4142135623730951, -1.0, 1.0));
        lat = asin(clamp((2.0 * th + sin(2.0 * th)) / PI, -1.0, 1.0));
        lon = q.x / (MOLL_X * max(cos(th), 1e-4));
        cov = clamp((1.4142135623730951 - abs(q.y)) / px + 0.5, 0.0, 1.0);
    }
    if (id >= 3) {
        // One world only: the side edge is where |lon| reaches 180 degrees.
        cov *= clamp((PI - abs(lon)) / max(fwidth(lon), 1e-6) + 0.5, 0.0, 1.0);
    }
    return cov;
}

void main() {
    if (u_mode == 3) {
        // Uniform across the draw, so every pixel quad takes the same path.
        outColor = vec4(space(gl_FragCoord.xy / u_scale), 1.0);
        return;
    }
    vec2 p = (gl_FragCoord.xy - 0.5 * u_res) / (0.5 * min(u_res.x, u_res.y));
    vec3 rd = normalize(vec3(p * u_tanHalfFov, -1.0));
    vec3 ro = vec3(0.0, 0.0, u_dist);

    vec3 bg = space(gl_FragCoord.xy / u_scale);
    float coverage = 1.0;
    vec3 n = vec3(0.0, 0.0, 1.0);   // camera-space normal (lighting, globe and morph only)
    vec3 w;                         // world-space normal (the sun)
    float lat;
    float lon;
    float u;                        // longitude as a texture coordinate; not wrapped on flat maps
    float globeW = 1.0;             // how much globe lighting to apply
    float uScale = 1.0;
    float dmin = 0.0;

    // No early-out for background pixels: derivatives (fwidth, implicit mip
    // selection) need every pixel in a 2×2 quad on the same control path.
    if (u_mode == 0) {
        // Closest approach of the ray to the globe center.
        float b = dot(ro, rd);
        float dmin2 = max(u_dist * u_dist - b * b, 0.0);
        dmin = sqrt(dmin2);
        float aa = max(fwidth(dmin), 1e-6);
        coverage = 1.0 - smoothstep(1.0 - aa, 1.0 + aa, dmin);

        bg += ATMOSPHERE * 0.55 * exp(-max(dmin - 1.0, 0.0) * 22.0);

        float t = -b - sqrt(max(1.0 - dmin2, 0.0));
        n = normalize(ro + t * rd);
        w = u_rot * n;
        lat = asin(clamp(w.y, -1.0, 1.0));
        lon = atan(w.x, w.z);

        // Pick whichever of two longitude parameterizations is continuous across
        // this pixel quad, so mipmapping doesn't draw a seam at the antimeridian.
        float u1 = lon / (2.0 * PI) + 0.5;
        float u2 = fract(u1 + 0.5) - 0.5;
        u = fwidth(u1) <= fwidth(u2) + 1e-6 ? u1 : u2;
        // The texture is equirectangular, so a screen pixel spans 1/cos(lat) times
        // more u than v near a pole. Implicit LOD would take the longer axis and
        // pick a far-too-coarse mip (blurred coasts). Scale the u gradient by
        // cos(lat) so the footprint matches the real surface area. Keep in sync
        // with `polar_u_scale` in src/camera.rs.
        uScale = max(cos(lat), 0.02);
    } else if (u_mode == 1) {
        // Flat map: invert the projection for this pixel. Longitude is
        // continuous across the screen (the map texture repeats), so there is
        // no seam to work around and the footprint is the true one.
        vec2 q = (gl_FragCoord.xy - 0.5 * u_res) / u_flatPx;
        q.y += u_cy;
        float rel;
        coverage = flatInverse(u_proj, q, lat, rel);
        lon = u_lon0 + rel;
        w = vec3(cos(lat) * sin(lon), sin(lat), cos(lat) * cos(lon));
        u = lon / (2.0 * PI) + 0.5;
        globeW = 0.0;
    } else {
        // Morph mesh: lat/lon come from the vertices. The surface faces the
        // camera as far as the globe is blended in.
        lat = v_ll.x;
        lon = u_lon0 + v_ll.y;
        w = vec3(cos(lat) * sin(lon), sin(lat), cos(lat) * cos(lon));
        n = w * u_rot;
        u = lon / (2.0 * PI) + 0.5;
        coverage = clamp(v_front / max(fwidth(v_front), 1e-6) + 0.5, 0.0, 1.0);
        globeW = u_globe;
        uScale = mix(1.0, max(cos(lat), 0.02), u_globe);
    }
    vec2 uv = vec2(u, 0.5 - lat / PI);

    vec2 duvdx = vec2(dFdx(u) * uScale, dFdx(uv.y));
    vec2 duvdy = vec2(dFdy(u) * uScale, dFdy(uv.y));

    float m = textureGrad(u_map, uv, duvdx, duvdy).r * u_hasMap;
    // Vector coastlines (R) and borders (G), drawn at constant pixel width.
    vec2 ln = texelFetch(u_lines, ivec2(gl_FragCoord.xy), 0).rg;

    // When the map is magnified, threshold the bilinear mask at 0.5 for crisp
    // vector-like edges. When minified, the mipmapped value already is an
    // antialiased coverage estimate, so use it directly.
    vec2 texSize = vec2(textureSize(u_map, 0));
    float texelsPerPx = max(
        (abs(duvdx.x) + abs(duvdy.x)) * texSize.x,
        (abs(duvdx.y) + abs(duvdy.y)) * texSize.y);
    float sharpen = (1.0 - smoothstep(0.6, 1.6, texelsPerPx)) * u_hasMap;
    float fw = max(fwidth(m), 1e-4);
    float land = mix(m, smoothstep(0.5 - fw, 0.5 + fw, m), sharpen);

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

    float border = ln.g * land;
    ground = mix(ground, borderCol, border * borderMix);

    // Highlighted country: bilinear blend of "texel is this country" so the
    // edge of the ID raster (2048 or 4096 wide) stays smooth when magnified. The branch
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
    col = mix(col, COAST, ln.r * 0.35);

    // Graticule with constant on-screen width; meridians fade near the poles
    // where they converge.
    vec2 g = vec2(u * 360.0, latDeg + 90.0) / u_gridStep;
    vec2 gw = max(fwidth(g), vec2(1e-5));
    vec2 gd = abs(fract(g - 0.5) - 0.5) / (gw * 0.9);
    float meridianFade = 1.0 - smoothstep(70.0, 86.0, alat);
    if (u_mode == 1 && u_proj < 3) meridianFade = 1.0;  // parallel meridians run to the edge
    float meridian = (1.0 - min(gd.x, 1.0)) * meridianFade;
    float parallel = 1.0 - min(gd.y, 1.0);
    col = mix(col, vec3(0.85, 0.90, 1.0), max(meridian, parallel) * 0.14);

    // Soft key light from the upper left of the camera plus ocean glint.
    // A flat map is lit evenly (globeW = 0); the morph fades the lighting out.
    if (globeW > 0.0) {
        vec3 rdv = u_mode == 0 ? rd : normalize(n - ro);
        vec3 L = normalize(vec3(-0.45, 0.55, 0.85));
        float diffuse = max(dot(n, L), 0.0);
        col *= mix(1.0, 0.38 + 0.72 * diffuse, globeW);
        vec3 refl = reflect(-L, n);
        col += vec3(0.55, 0.65, 0.80) * pow(max(dot(refl, -rdv), 0.0), 36.0) * 0.22 * (1.0 - land) * globeW;
    }

    // Day/night terminator. sunElev is the sine of the sun's elevation at
    // this point; the smooth band spans roughly civil to nautical twilight
    // (about +2 to -10 degrees of sun elevation).
    float sunElev = dot(w, u_sun);
    float night = (1.0 - smoothstep(-0.17, 0.035, sunElev)) * u_sunOn;
    vec3 nightTint = u_layer == 1 ? vec3(0.24, 0.28, 0.42) : vec3(0.22, 0.26, 0.42);
    col *= mix(vec3(1.0), nightTint, night);

    // Atmospheric scattering toward the limb.
    if (globeW > 0.0) {
        vec3 rdv = u_mode == 0 ? rd : normalize(n - ro);
        float fres = pow(1.0 - max(dot(n, -rdv), 0.0), 3.0);
        col = mix(col, ATMOSPHERE, fres * 0.55 * globeW);
    }

    if (u_mode == 2) {
        // Over the space pass drawn first; coverage is the visible fraction.
        outColor = vec4(col, coverage);
    } else {
        outColor = vec4(mix(bg, col, coverage), 1.0);
    }
}
