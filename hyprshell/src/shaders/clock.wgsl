// Analog clock: a rounded-square face with a border and three capsule hands,
// all as signed distance fields evaluated per pixel. One full-screen triangle,
// no vertex buffer. Output is premultiplied alpha over a transparent surface.

struct Uniforms {
    // Widget rectangle in physical pixels of the render target: x, y, w, h.
    rect: vec4<f32>,
    face_color: vec4<f32>,
    border_color: vec4<f32>,
    hour_color: vec4<f32>,
    minute_color: vec4<f32>,
    second_color: vec4<f32>,
    // Per hand: angle (radians clockwise from 12), length (fraction of the
    // radius), half width (physical px), enabled (0 or 1).
    hour: vec4<f32>,
    minute: vec4<f32>,
    second: vec4<f32>,
    // border width (px), corner radius (px), unused, unused.
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
};

// Three vertices covering the whole target; the scissor rect limits it to the
// widget. No buffers to bind.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VertexOut {
    var out: VertexOut;
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

// Signed distance to a box with half-extents `b` and corner radius `r`.
fn sd_rounded_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// Signed distance to the segment a-b.
fn sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

// Coverage of a shape from its signed distance, one pixel of anti-aliasing.
fn coverage(d: f32) -> f32 {
    return 1.0 - smoothstep(-0.5, 0.5, d);
}

// Premultiplied source-over.
fn over(dst: vec4<f32>, src: vec4<f32>, cov: f32) -> vec4<f32> {
    let s = src * cov;
    return s + dst * (1.0 - s.a);
}

fn hand(dst: vec4<f32>, p: vec2<f32>, radius: f32, h: vec4<f32>, color: vec4<f32>) -> vec4<f32> {
    if (h.w < 0.5) {
        return dst;
    }
    let dir = vec2<f32>(sin(h.x), -cos(h.x));
    // Hands start slightly behind the centre so the pivot looks solid.
    let a = -dir * (h.z * 1.5);
    let b = dir * (h.y * radius);
    let d = sd_segment(p, a, b) - h.z;
    return over(dst, color, coverage(d));
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let half = u.rect.zw * 0.5;
    // Pixel position relative to the widget centre, y down.
    let p = in.position.xy - u.rect.xy - half;
    let radius = min(half.x, half.y);
    let border = u.params.x;
    let rounding = u.params.y;

    // Face: a soft radial falloff towards the edge so it does not look flat.
    let d_face = sd_rounded_box(p, half - vec2<f32>(0.5, 0.5), rounding);
    let shade = 1.0 - 0.25 * smoothstep(0.0, radius, length(p));
    // Everything is drawn unclipped, then clipped to the face once at the end
    // so the edge is anti-aliased exactly once.
    var color = vec4<f32>(u.face_color.rgb * shade, u.face_color.a);

    // Border: the band `border` pixels inwards from the edge.
    if (border > 0.0) {
        color = over(color, u.border_color, coverage(-(d_face + border)));
    }

    // Hands stay inside the border ring, tips included.
    let inner = max(radius - border, 0.0);
    color = hand(color, p, inner - u.hour.z, u.hour, u.hour_color);
    color = hand(color, p, inner - u.minute.z, u.minute, u.minute_color);
    color = hand(color, p, inner - u.second.z, u.second, u.second_color);

    // Pivot dot, wider than the minute hand so it reads as a dot.
    color = over(color, u.minute_color, coverage(length(p) - max(u.minute.z * 2.0, 2.0)));

    return color * coverage(d_face);
}
