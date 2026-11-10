// cubicle GPU renderer. Mirrors src/raster.rs: same camera, light and shading.

struct U {
    c0: vec4<f32>,   // rotation matrix columns
    c1: vec4<f32>,
    c2: vec4<f32>,
    dims: vec4<f32>, // target width, target height, centre x, centre y (pixels)
    proj: vec4<f32>, // x_scale, focal, cam
    light: vec4<f32>,
    opts: vec4<f32>, // smooth, two_sided, use_tex
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VIn {
    @location(0) pos: vec4<f32>, // xyz, w = has texture coordinates
    @location(1) n: vec4<f32>,
    @location(2) nf: vec4<f32>,
    @location(3) col: vec4<f32>,
    @location(4) uv: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) p: vec3<f32>,
    @location(1) n: vec3<f32>,
    @location(2) nf: vec3<f32>,
    @location(3) col: vec3<f32>,
    @location(4) uv: vec3<f32>, // u, v, has_uv
};

@vertex
fn vs(v: VIn) -> VOut {
    let rot = mat3x3<f32>(u.c0.xyz, u.c1.xyz, u.c2.xyz);
    let p = rot * v.pos.xyz;
    // The camera sits at +z looking toward -z; z is the depth in front of it.
    let z = u.proj.z - p.z;
    let sx = u.dims.z + p.x * u.proj.x / z;
    let sy = u.dims.w - p.y * u.proj.y / z;
    let nx = sx / u.dims.x * 2.0 - 1.0;
    let ny = 1.0 - sy / u.dims.y * 2.0;
    // 1/z is linear in screen space, so this depth is too (larger = farther).
    let d = 1.0 - 0.1 / z;
    var o: VOut;
    // w = z makes the hardware's attribute interpolation perspective-correct.
    o.clip = vec4<f32>(nx * z, ny * z, d * z, z);
    o.p = p;
    o.n = rot * v.n.xyz;
    o.nf = rot * v.nf.xyz;
    o.col = v.col.xyz;
    o.uv = vec3<f32>(v.uv.xy, v.pos.w);
    return o;
}

@fragment
fn fs(i: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var n = normalize(i.nf);
    if (u.opts.x > 0.5 && length(i.n) > 1e-6) {
        n = normalize(i.n);
    }
    if (!front) {
        n = -n; // two-sided: light back faces from their own side
    }
    let to_light = u.light.xyz - i.p;
    let d2 = dot(to_light, to_light);
    let diff = max(dot(n, normalize(to_light)), 0.0);
    let atten = 1.0 / (1.0 + 0.06 * d2);
    let lum = min(0.08 + 1.9 * diff * atten, 1.0);
    var col = i.col;
    if (u.opts.z > 0.5 && i.uv.z > 0.5) {
        col = textureSampleLevel(tex, samp, vec2<f32>(i.uv.x, 1.0 - i.uv.y), 0.0).rgb;
    }
    return vec4<f32>(col, lum);
}
