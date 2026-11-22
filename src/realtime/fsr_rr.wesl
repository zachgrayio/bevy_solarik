// AMD FSR Ray Regeneration (FidelityFX denoiser) inputs and composition for DX12.
//
// `prepare_fsr_rr` turns Solari's split lighting (the NRD_SIGNALS diffuse/specular
// radiance with hit distance in alpha) into the denoiser's inputs: first-bounce
// albedo is divided out (passed separately, non-encoded), normals are world-space
// octahedral with linear roughness, depth is signed view-space z and motion is
// Bevy's UV motion plus the signed view-z delta. `resolve_fsr_rr` multiplies the
// denoised signals by the same albedo factors and adds emission, like NRD.

import bevy_render::view::View;
import bevy_pbr::prepass::bindings::PreviousViewUniforms;
import bevy_solarik::realtime::gbuffer_utils::gpixel_resolve;

@group(0) @binding(0) var raw_diffuse: texture_2d<f32>;
@group(0) @binding(1) var raw_specular: texture_2d<f32>;
@group(0) @binding(2) var gbuffer: texture_2d<u32>;
@group(0) @binding(3) var depth_buffer: texture_depth_2d;
@group(0) @binding(4) var motion: texture_2d<f32>;
@group(0) @binding(5) var<uniform> view: View;
@group(0) @binding(6) var<uniform> previous_view: PreviousViewUniforms;
@group(0) @binding(7) var linear_depth_out: texture_storage_2d<r32float, write>;
@group(0) @binding(8) var motion_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(9) var normals_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(10) var diffuse_albedo_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(11) var specular_albedo_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(12) var diffuse_in: texture_storage_2d<rgba16float, write>;
@group(0) @binding(13) var specular_in: texture_storage_2d<rgba16float, write>;
@group(0) @binding(14) var diffuse_out: texture_2d<f32>;
@group(0) @binding(15) var specular_out: texture_2d<f32>;
@group(0) @binding(16) var view_output: texture_storage_2d<rgba16float, write>;

// Signed view-space z for pixels without geometry; outside the dispatch's
// `linearDepthBounds`, so the denoiser passes them through.
const SKY_LINEAR_DEPTH: f32 = -1.0e6;
const FP16_MAX: f32 = 65504.0;

// Octahedral encoding from AMD's denoising guide (`NormalToOctahedronUv`).
fn octahedron_uv(n: vec3f) -> vec2f {
    var p = n.xy / (abs(n.x) + abs(n.y) + abs(n.z));
    if n.z < 0.0 {
        p = (1.0 - abs(p.yx)) * select(vec2f(-1.0), vec2f(1.0), p >= vec2f(0.0));
    }
    return p * 0.5 + 0.5;
}

struct AlbedoFactors {
    diffuse: vec3f,
    specular: vec3f,
}

// The same demodulation factors as the NRD path, so resolve restores exactly
// what prepare divided out.
fn albedo_factors(base_color: vec3f, metallic: f32, reflectance: f32) -> AlbedoFactors {
    let f0 = vec3f(0.16 * reflectance * reflectance);
    return AlbedoFactors(
        max(base_color * (1.0 - metallic), vec3f(0.01)),
        max(mix(f0, base_color, metallic), vec3f(0.01)),
    );
}

@compute @workgroup_size(8, 8, 1)
fn prepare_fsr_rr(@builtin(global_invocation_id) gid: vec3u) {
    let p = gid.xy;
    if any(p >= textureDimensions(raw_diffuse)) { return; }
    let depth = textureLoad(depth_buffer, p, 0);
    let gpixel = textureLoad(gbuffer, p, 0);
    let uv_motion = textureLoad(motion, p, 0).xy;
    if depth == 0.0 || gpixel.a == 0u {
        textureStore(linear_depth_out, p, vec4f(SKY_LINEAR_DEPTH));
        textureStore(motion_out, p, vec4f(uv_motion, 0.0, 0.0));
        textureStore(normals_out, p, vec4f(0.5, 0.5, 1.0, 0.0));
        textureStore(diffuse_albedo_out, p, vec4f(0.0));
        textureStore(specular_albedo_out, p, vec4f(0.0));
        textureStore(diffuse_in, p, vec4f(0.0, 0.0, 0.0, FP16_MAX));
        textureStore(specular_in, p, vec4f(0.0, 0.0, 0.0, FP16_MAX));
        return;
    }
    let s = gpixel_resolve(gpixel, depth, p, view.main_pass_viewport.zw, view.world_from_clip);
    let world = vec4f(s.world_position, 1.0);
    let view_z = (view.view_from_world * world).z;
    let previous_view_z = (previous_view.view_from_world * world).z;
    let factors = albedo_factors(s.material.base_color, s.material.metallic, s.material.reflectance);
    let d = textureLoad(raw_diffuse, p, 0);
    let r = textureLoad(raw_specular, p, 0);
    // Solari stores roughness squared; the denoiser wants linear roughness.
    let linear_roughness = sqrt(max(s.material.roughness, 0.0));
    textureStore(linear_depth_out, p, vec4f(view_z));
    textureStore(motion_out, p, vec4f(uv_motion, previous_view_z - view_z, 0.0));
    textureStore(normals_out, p, vec4f(octahedron_uv(s.world_normal), linear_roughness, 0.0));
    textureStore(diffuse_albedo_out, p, vec4f(factors.diffuse, 1.0));
    textureStore(specular_albedo_out, p, vec4f(factors.specular, 1.0));
    let diffuse_hit = select(d.a, FP16_MAX, d.a <= 0.0);
    let specular_hit = select(r.a, FP16_MAX, r.a <= 0.0);
    textureStore(diffuse_in, p, vec4f(max(d.rgb / factors.diffuse, vec3f(0.0)), min(diffuse_hit, FP16_MAX)));
    textureStore(specular_in, p, vec4f(max(r.rgb / factors.specular, vec3f(0.0)), min(specular_hit, FP16_MAX)));
}

@compute @workgroup_size(8, 8, 1)
fn resolve_fsr_rr(@builtin(global_invocation_id) gid: vec3u) {
    let p = gid.xy;
    if any(p >= textureDimensions(raw_diffuse)) { return; }
    let depth = textureLoad(depth_buffer, p, 0);
    let gpixel = textureLoad(gbuffer, p, 0);
    if depth == 0.0 || gpixel.a == 0u { return; }
    let s = gpixel_resolve(gpixel, depth, p, view.main_pass_viewport.zw, view.world_from_clip);
    let factors = albedo_factors(s.material.base_color, s.material.metallic, s.material.reflectance);
    let color = textureLoad(diffuse_out, p, 0).rgb * factors.diffuse
        + textureLoad(specular_out, p, 0).rgb * factors.specular
        + s.material.emissive * view.exposure;
    textureStore(view_output, p, vec4f(color, 1.0));
}
