#version 300 es
// A body of the solar system view (src/solar_scene.rs): a sphere ray-traced
// per pixel, lit by the Sun, with Saturn's rings and the Sun's glow. Each
// pixel is inverted through the stereographic projection to a ray, as in
// sky.frag. Everything is camera-relative and in units of the body's radius
// (u_center), so no heliocentric coordinate reaches the GPU (src/solar.rs).
// Output is premultiplied alpha.
precision highp float;

uniform vec2 u_res;           // framebuffer size, device pixels
uniform float u_halfShort;    // half the shorter axis, device pixels
uniform float u_tanQ;         // tan(fov / 4)
uniform vec3 u_center;        // sphere center, camera space, body radii
uniform vec3 u_sun;           // unit vector to the Sun, camera space (0 for the Sun)
uniform vec3 u_pole;          // rotation pole, camera space
uniform vec3 u_color;
uniform float u_emissive;     // 1 for the Sun
uniform float u_bands;        // strength of latitude bands (the giants)
uniform vec2 u_rings;         // inner and outer ring radius, body radii (0 for none)
uniform vec3 u_dot;           // disk center and radius, device pixels
uniform float u_dpr;

out vec4 outColor;

// Brightness of Saturn's rings at radius r (Saturn radii): the faint C
// ring, the bright B ring, the Cassini division and the A ring with the
// Encke gap.
float ringDensity(float r) {
    float c = smoothstep(1.239, 1.26, r) * (1.0 - smoothstep(1.51, 1.53, r)) * 0.18;
    float b = smoothstep(1.52, 1.56, r) * (1.0 - smoothstep(1.94, 1.95, r)) * (0.75 + 0.25 * sin(r * 40.0));
    float a = smoothstep(2.02, 2.03, r) * (1.0 - smoothstep(2.26, 2.27, r)) * 0.55;
    float encke = 1.0 - 0.8 * (smoothstep(2.21, 2.213, r) * (1.0 - smoothstep(2.217, 2.22, r)));
    return c + b + a * encke;
}

void main() {
    vec2 frag = gl_FragCoord.xy;
    vec2 p = (frag - 0.5 * u_res) / u_halfShort;
    float r = length(p);
    float t = r * u_tanQ;
    float k = 1.0 / (1.0 + t * t);
    vec2 side = r > 0.0 ? p / r * (2.0 * t * k) : vec2(0.0);
    vec3 dir = vec3(side, -(1.0 - t * t) * k);
    float radPerPx = 2.0 * u_tanQ * k / u_halfShort;

    // Too small to resolve: a round dot of the body's color (the Sun's
    // bigger, with a glow).
    float dotR = (u_emissive > 0.5 ? 3.0 : 1.6) * u_dpr;
    if (u_dot.z < dotR) {
        float d = length(frag - u_dot.xy);
        float a = clamp(dotR + 0.5 - d, 0.0, 1.0);
        vec3 glow = u_emissive * u_color * 0.5 * exp(-max(d - dotR, 0.0) / (8.0 * u_dpr));
        outColor = vec4(u_color * a + glow * (1.0 - a), a);
        return;
    }

    // Ray from the camera (the origin) against the unit sphere at u_center.
    // The perpendicular distance avoids the cancellation in b^2 - c.
    float dist = length(u_center);
    float b = dot(dir, u_center);
    vec3 perp = u_center - b * dir;
    float miss = length(perp);           // 1 at the outline
    float h = 1.0 - miss * miss;
    // Coverage over a pixel at the outline: body radii per pixel there.
    float edgePx = (miss - 1.0) / max(dist * radPerPx, 1e-12);
    float cover = clamp(0.5 - edgePx, 0.0, 1.0);
    vec4 col = vec4(0.0);
    float tSphere = 1e30;
    if (h > -0.05 && b > 0.0) {
        float tHit = b - sqrt(max(h, 0.0));
        tSphere = tHit;
        vec3 n = normalize(tHit * dir - u_center);
        vec3 c = u_color;
        if (u_bands > 0.0) {
            float lat = dot(n, u_pole);
            c *= 1.0 + u_bands * (0.6 * sin(lat * 18.0) + 0.4 * sin(lat * 41.0 + 1.3));
        }
        if (u_emissive > 0.5) {
            // Limb darkening.
            float mu = max(dot(n, -dir), 0.0);
            c *= 0.55 + 0.45 * sqrt(mu);
        } else {
            // Lambert, with a faint night side so the disk stays visible.
            c *= 0.02 + max(dot(n, u_sun), 0.0);
        }
        col = vec4(c * cover, cover);
    }

    // Saturn's rings: the ray against the plane through the center.
    if (u_rings.y > 0.0) {
        float dn = dot(dir, u_pole);
        if (abs(dn) > 1e-6) {
            float tr = dot(u_center, u_pole) / dn;
            vec3 q = tr * dir - u_center;
            float rr = length(q);
            if (tr > 0.0 && rr > u_rings.x && rr < u_rings.y) {
                float dens = ringDensity(rr);
                // The lit face is the one towards the Sun.
                float sunSide = dot(u_sun, u_pole);
                float viewSide = dot(-dir, u_pole);
                float face = sunSide * viewSide > 0.0 ? 1.0 : 0.3;
                // The planet's shadow on the rings.
                float along = dot(q, u_sun);
                float off = length(q - along * u_sun);
                float shade = (along < 0.0 && off < 1.0) ? 0.08 : 1.0;
                vec3 rc = vec3(0.86, 0.78, 0.62) * (0.25 + 0.75 * abs(sunSide)) * face * shade;
                float a = clamp(dens, 0.0, 0.95);
                vec4 ring = vec4(rc * a, a);
                if (tr < tSphere) {
                    col = ring + col * (1.0 - ring.a);
                } else {
                    col = col + ring * (1.0 - col.a);
                }
            }
        }
    }

    // The Sun's glow beyond its disk.
    if (u_emissive > 0.5) {
        float glow = 0.55 * exp(-max(edgePx, 0.0) / (10.0 * u_dpr + 0.4 * u_dot.z));
        col.rgb += u_color * glow * (1.0 - cover);
    }
    outColor = col;
}
