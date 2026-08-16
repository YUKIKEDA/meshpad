struct Uniforms {
    view_proj: mat4x4<f32>,
    light_dir: vec3<f32>,
    _pad0: f32,
    color: vec4<f32>,
    origin: vec3<f32>,
    _pad1: f32,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec3<f32>) -> VsOut {
    var o: VsOut;
    // 頂点はファイル座標。AABB 中心を引き、巨大座標の float32 ジッタを抑える。
    let p = pos - u.origin;
    o.world = p;
    o.clip = u.view_proj * vec4<f32>(p, 1.0);
    return o;
}

@fragment
fn fs_main(inp: VsOut) -> @location(0) vec4<f32> {
    // 法線は頂点に無い。偏微分から面法線を復元する。両面ライトは abs(N·L)。
    let dx = dpdx(inp.world);
    let dy = dpdy(inp.world);
    var n = normalize(cross(dx, dy));
    let ndotl = abs(dot(n, normalize(u.light_dir)));
    let lit = u.color.rgb * (0.22 + 0.78 * ndotl);
    return vec4<f32>(lit, 1.0);
}
