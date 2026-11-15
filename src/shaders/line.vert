#version 300 es
// One instance per line segment: endpoints a and b on the unit sphere (world
// space). Each is projected with the same camera model as globe.frag, then
// the segment is expanded into a screen-space quad whose width is constant in
// pixels, whatever the zoom. Conventions match src/camera.rs. On a flat map
// (or mid-morph) the endpoints go through the same projection math as
// morph.vert instead; segments that cross the map's side edge are drawn once
// per copy of the world and clipped to one world where the map is a single
// outline.
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
uniform float u_width;        // line width, device pixels
uniform float u_shift;        // flat maps: longitude offset of this copy of the world, radians
uniform float u_clip;         // 1 = clip to one world (-180..180 from the central meridian)

layout(location = 0) in vec2 a_corner;  // x: 0 = at a, 1 = at b; y: -1 or +1 side
layout(location = 1) in vec3 a_a;
layout(location = 2) in vec3 a_b;

flat out vec2 v_a;            // endpoints in pixels (gl_FragCoord space)
flat out vec2 v_b;
out float v_front;            // > 0 on the camera's side of the horizon

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

vec2 project(vec3 c) {
    vec2 q = c.xy / ((u_dist - c.z) * u_tanHalfFov);
    return 0.5 * u_res + q * 0.5 * min(u_res.x, u_res.y);
}

float wrapPi(float a) {
    return mod(a + PI, 2.0 * PI) - PI;
}

float lonOf(vec3 p) {
    return abs(p.x) + abs(p.z) < 1e-7 ? u_lon0 : atan(p.x, p.z);
}

void main() {
    vec2 sa;
    vec2 sb;
    float fa;
    float fb;
    bool hidden = false;
    if (u_projA == 0 && u_projB == 0) {
        // transpose(u_rot) * p: world space -> camera space.
        vec3 ca = a_a * u_rot;
        vec3 cb = a_b * u_rot;
        // A point on the unit sphere faces the camera when dot(p, eye - p) > 0,
        // i.e. z * dist > 1 in camera space.
        fa = ca.z * u_dist - 1.0;
        fb = cb.z * u_dist - 1.0;
        hidden = fa < 0.0 && fb < 0.0;  // wholly behind the limb
        sa = project(ca);
        sb = project(cb);
    } else {
        float la = asin(clamp(a_a.y, -1.0, 1.0));
        float lb = asin(clamp(a_b.y, -1.0, 1.0));
        float ra = wrapPi(lonOf(a_a) - u_lon0);
        // Bring b next to a, so a segment over the side edge stays short.
        float rb = ra + wrapPi(lonOf(a_b) - u_lon0 - ra);
        float xa = ra + u_shift;
        float xb = rb + u_shift;
        float t0 = 0.0;
        float t1 = 1.0;
        if (u_clip > 0.5) {
            float dx = xb - xa;
            if (abs(dx) < 1e-9) {
                hidden = abs(xa) > PI;
            } else {
                float s0 = (-PI - xa) / dx;
                float s1 = (PI - xa) / dx;
                t0 = max(0.0, min(s0, s1));
                t1 = min(1.0, max(s0, s1));
                hidden = t0 >= t1;
            }
        }
        // Mercator shows nothing past its latitude limit.
        if (u_projA == 2 && u_projB == 2 && min(abs(la), abs(lb)) > MERC_MAX) hidden = true;
        float lat0 = mix(la, lb, t0);
        float lat1 = mix(la, lb, t1);
        float x0 = mix(xa, xb, t0);
        float x1 = mix(xa, xb, t1);
        float fa0;
        float fa1;
        float fb0;
        float fb1;
        sa = mix(place(u_projA, u_cyAB.x, lat0, x0, fa0), place(u_projB, u_cyAB.y, lat0, x0, fb0), u_mix);
        sb = mix(place(u_projA, u_cyAB.x, lat1, x1, fa1), place(u_projB, u_cyAB.y, lat1, x1, fb1), u_mix);
        fa = mix(fa0, fb0, u_mix);
        fb = mix(fa1, fb1, u_mix);
        hidden = hidden || (fa < 0.0 && fb < 0.0);
    }
    if (hidden) {
        gl_Position = vec4(2.0, 2.0, 2.0, 1.0);  // clipped
        v_a = v_b = vec2(0.0);
        v_front = -1.0;
        return;
    }
    vec2 d = sb - sa;
    float l = length(d);
    vec2 dir = l > 1e-4 ? d / l : vec2(1.0, 0.0);
    vec2 nrm = vec2(-dir.y, dir.x);
    float half_ = 0.5 * u_width + 1.5;  // line plus antialiasing margin
    vec2 pos = mix(sa, sb, a_corner.x)
        + dir * (a_corner.x * 2.0 - 1.0) * half_
        + nrm * a_corner.y * half_;
    v_a = sa;
    v_b = sb;
    v_front = mix(fa, fb, a_corner.x);
    gl_Position = vec4(pos / u_res * 2.0 - 1.0, 0.0, 1.0);
}
