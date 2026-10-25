// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Original display-only perspective and Lambert lighting; no engineering computation.
struct Camera { right: vec4f, up: vec4f, back: vec4f, eye: vec4f, params: vec4f }
@group(0) @binding(0) var<uniform> camera: Camera;
struct Out { @builtin(position) position: vec4f, @location(0) normal: vec3f }
fn rotate(v: vec3f, q: vec4f) -> vec3f {
    return v + 2.0 * cross(q.xyz, cross(q.xyz, v) + q.w * v);
}
@vertex fn instanced(
    @location(0) p: vec3f,
    @location(1) n: vec3f,
    @location(2) t: vec3f,
    @location(3) q: vec4f,
) -> Out {
    let v = rotate(p, q) + t - camera.eye.xyz;
    let d = -dot(v, camera.back.xyz);
    let near = camera.params.z;
    let far = camera.params.w;
    var o: Out;
    o.position = vec4f(
        dot(v, camera.right.xyz) / (camera.params.x * camera.params.y),
        dot(v, camera.up.xyz) / camera.params.x,
        far / (far - near) * d - far * near / (far - near),
        d,
    );
    o.normal = rotate(n, q);
    return o;
}
// The colours of the BeamCraft logo: faces turned up are cream, sides grey-teal.
@fragment fn shade(o: Out) -> @location(0) vec4f {
    let n = normalize(o.normal);
    let base = mix(vec3f(0.47, 0.71, 0.77), vec3f(0.96, 0.94, 0.88), clamp(n.z, 0.0, 1.0));
    let light = 0.55 + 0.45 * max(dot(n, normalize(vec3f(0.3, -0.4, 0.85))), 0.0);
    return vec4f(base * light, 1.0);
}
