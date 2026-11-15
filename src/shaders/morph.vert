#version 300 es
// Globe <-> flat morph. A lat/lon grid mesh whose vertices are placed by
// blending where each projection puts them (the globe as the camera sees it,
// or a flat map), so the globe visibly unrolls. The fragment shader
// (globe.frag, u_mode = 2) shades each pixel from the interpolated lat/lon.
// Only drawn mid-morph; settled views use the exact per-pixel paths.
precision highp float;

uniform vec2 u_res;           // framebuffer size, device pixels
uniform float u_dist;         // camera distance from globe center, radii
uniform float u_tanHalfFov;   // across the shorter screen axis
uniform mat3 u_rot;           // camera space -> world space
uniform float u_lon0;         // central meridian of the flat maps, radians
uniform int u_projA;          // projection blended from (0 = globe)
uniform int u_projB;          // projection blended to
uniform float u_mix;          // 0 = all A, 1 = all B
uniform vec2 u_cyAB;          // plane y of the screen-center row, for A and B
uniform float u_flatPx;       // flat maps: framebuffer pixels per plane unit

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

// Flat projections (see src/projection.rs). id: 1 equirectangular,
// 2 Mercator (clamped), 3 Robinson, 4 Mollweide. lon is relative to the
// central meridian. Keep identical in morph.vert and line.vert.
float mollTheta(float lat) {
    float th = lat;
    for (int i = 0; i < 14; i++) {
        float d = 2.0 + 2.0 * cos(2.0 * th);
        if (abs(d) < 1e-5) break;
        th -= (2.0 * th + sin(2.0 * th) - PI * sin(lat)) / d;
    }
    return th;
}

vec2 flatForward(int id, float lat, float lon) {
    lat = clamp(lat, -0.5 * PI, 0.5 * PI);
    if (id == 1) return vec2(lon, lat);
    if (id == 2) {
        float l = clamp(lat, -MERC_MAX, MERC_MAX);
        return vec2(lon, log(tan(0.25 * PI + 0.5 * l)));
    }
    if (id == 3) {
        float a = abs(lat) * (18.0 / (0.5 * PI));
        int i = min(int(a), 17);
        float f = a - float(i);
        return vec2(ROB_XS * mix(ROB_X[i], ROB_X[i + 1], f) * lon,
                    ROB_YS * mix(ROB_Y[i], ROB_Y[i + 1], f) * sign(lat));
    }
    float th = mollTheta(lat);
    return vec2(MOLL_X * lon * cos(th), 1.4142135623730951 * sin(th));
}

// Screen position (framebuffer pixels) of a surface point under projection
// `id` (0 = the globe seen by the camera). `front` is > 0 where the point is
// visible. Points behind the globe's limb are pinned to the limb, so that
// as the morph starts they peel off it rather than show through the front.
// Keep identical in morph.vert and line.vert.
vec2 place(int id, float cy, float lat, float rel, out float front) {
    if (id == 0) {
        float lon = u_lon0 + rel;
        vec3 w = vec3(cos(lat) * sin(lon), sin(lat), cos(lat) * cos(lon));
        vec3 c = w * u_rot;
        front = c.z * u_dist - 1.0;
        vec2 q;
        if (front >= 0.0) {
            q = c.xy / ((u_dist - c.z) * u_tanHalfFov);
        } else {
            vec2 dir = length(c.xy) > 1e-6 ? normalize(c.xy) : vec2(0.0, 1.0);
            q = dir / (sqrt(max(u_dist * u_dist - 1.0, 1e-6)) * u_tanHalfFov);
        }
        return 0.5 * u_res + q * 0.5 * min(u_res.x, u_res.y);
    }
    front = 1.0;
    vec2 f = flatForward(id, lat, rel);
    return 0.5 * u_res + vec2(f.x, f.y - cy) * u_flatPx;
}

layout(location = 0) in vec2 a_ll;  // latitude, longitude relative to the central meridian

out vec2 v_ll;
out float v_front;                  // > 0 where visible

void main() {
    float fa;
    float fb;
    vec2 pa = place(u_projA, u_cyAB.x, a_ll.x, a_ll.y, fa);
    vec2 pb = place(u_projB, u_cyAB.y, a_ll.x, a_ll.y, fb);
    vec2 pos = mix(pa, pb, u_mix);
    v_front = mix(fa, fb, u_mix);
    v_ll = a_ll;
    gl_Position = vec4(pos / u_res * 2.0 - 1.0, 0.0, 1.0);
}
