// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Original display-only perspective and orthographic projection (V13) and Lambert lighting;
// no engineering computation.
struct Camera { right: vec4f, up: vec4f, back: vec4f, eye: vec4f, params: vec4f, mode: vec4f }
@group(0) @binding(0) var<uniform> camera: Camera;
// The V15 cut plane: a count and up to 16 half-spaces (normal.xyz, -normal·point). A fragment
// outside any half-space is discarded, so the model is cut in the image only and never in data.
struct Clip { count: u32, pad0: u32, pad1: u32, pad2: u32, planes: array<vec4f, 16> }
@group(0) @binding(1) var<uniform> clip: Clip;
struct Out { @builtin(position) position: vec4f, @location(0) normal: vec3f, @location(1) world: vec3f }
fn rotate(v: vec3f, q: vec4f) -> vec3f {
    return v + 2.0 * cross(q.xyz, cross(q.xyz, v) + q.w * v);
}
@vertex fn instanced(
    @location(0) p: vec3f,
    @location(1) n: vec3f,
    @location(2) t: vec3f,
    @location(3) q: vec4f,
) -> Out {
    let world = rotate(p, q) + t;
    let v = world - camera.eye.xyz;
    let d = -dot(v, camera.back.xyz);
    let near = camera.params.z;
    let far = camera.params.w;
    var o: Out;
    // Perspective keeps the view depth as w so the GPU divides, and its z term is clip space
    // (NDC after that divide). Orthographic puts the lateral scale in params.xy (half-height,
    // aspect), sets w to 1, and maps the depth linearly into NDC itself.
    let persp = far / (far - near) * d - far * near / (far - near);
    let ortho = (d - near) / (far - near);
    o.position = vec4f(
        dot(v, camera.right.xyz) / (camera.params.x * camera.params.y),
        dot(v, camera.up.xyz) / camera.params.x,
        mix(persp, ortho, camera.mode.x),
        mix(d, 1.0, camera.mode.x),
    );
    o.normal = rotate(n, q);
    o.world = world;
    return o;
}
// Discards every fragment outside all half-spaces: the cut is a display filter.
fn cut(world: vec3f) {
    for (var i = 0u; i < clip.count; i += 1u) {
        let plane = clip.planes[i];
        if (dot(plane.xyz, world) + plane.w < 0.0) { discard; }
    }
}
// Cool steel faces keep the engineering model distinct from the application chrome.
@fragment fn shade(o: Out) -> @location(0) vec4f {
    cut(o.world);
    let n = normalize(o.normal);
    let base = mix(vec3f(0.38, 0.51, 0.59), vec3f(0.74, 0.80, 0.84), clamp(n.z, 0.0, 1.0));
    let light = 0.55 + 0.45 * max(dot(n, normalize(vec3f(0.3, -0.4, 0.85))), 0.0);
    return vec4f(base * light, 1.0);
}
// The ghost preview of the member being placed: the same lighting on a distinct amber, so the
// not-yet-committed shape never reads as a model element.
@fragment fn shade_ghost(o: Out) -> @location(0) vec4f {
    cut(o.world);
    let n = normalize(o.normal);
    let light = 0.55 + 0.45 * max(dot(n, normalize(vec3f(0.3, -0.4, 0.85))), 0.0);
    return vec4f(vec3f(0.85, 0.55, 0.1) * light, 1.0);
}
// The grey mode: the same lighting on one neutral grey, without the part colours.
@fragment fn shade_grey(o: Out) -> @location(0) vec4f {
    cut(o.world);
    let n = normalize(o.normal);
    let light = 0.55 + 0.45 * max(dot(n, normalize(vec3f(0.3, -0.4, 0.85))), 0.0);
    return vec4f(vec3f(0.62, 0.62, 0.62) * light, 1.0);
}
// The transparent mode: the shaded colours at partial opacity; the pipeline blends and keeps
// depth testing but not depth writing, so hidden faces show through.
@fragment fn shade_transparent(o: Out) -> @location(0) vec4f {
    cut(o.world);
    let n = normalize(o.normal);
    let base = mix(vec3f(0.47, 0.71, 0.77), vec3f(0.96, 0.94, 0.88), clamp(n.z, 0.0, 1.0));
    let light = 0.55 + 0.45 * max(dot(n, normalize(vec3f(0.3, -0.4, 0.85))), 0.0);
    return vec4f(base * light, 0.45);
}
