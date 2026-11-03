import bevy_render::view::View;
import bevy_solarik::realtime::gbuffer_utils::gpixel_resolve;

@group(0) @binding(0) var raw_diffuse: texture_2d<f32>;
@group(0) @binding(1) var raw_specular: texture_2d<f32>;
@group(0) @binding(2) var gbuffer: texture_2d<u32>;
@group(0) @binding(3) var depth_buffer: texture_depth_2d;
@group(0) @binding(4) var motion: texture_2d<f32>;
@group(0) @binding(5) var<uniform> view: View;
@group(0) @binding(6) var normal_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(7) var motion_out: texture_storage_2d<rg16float, write>;
@group(0) @binding(8) var view_z_out: texture_storage_2d<r32float, write>;
@group(0) @binding(9) var diffuse_in: texture_storage_2d<rgba16float, write>;
@group(0) @binding(10) var specular_in: texture_storage_2d<rgba16float, write>;
@group(0) @binding(11) var diffuse_out: texture_2d<f32>;
@group(0) @binding(12) var specular_out: texture_2d<f32>;
@group(0) @binding(13) var view_output: texture_storage_2d<rgba16float, write>;

// Encoding 2 (R10G10B10A2_UNORM), as configured in the source-built SDK.
fn encode_normal(n: vec3f, roughness: f32) -> vec4f {
    let oct = n / (abs(n.x) + abs(n.y) + abs(n.z));
    let y = oct.y * 0.5 + 0.5;
    let signed_roughness = select(1.0, -1.0, oct.z < 0.0) * max(roughness, 1.5 / 512.0);
    return vec4f(oct.x * 0.5 + y, y - oct.x * 0.5, signed_roughness * 0.5 + 0.5, 0.0);
}

@compute @workgroup_size(8, 8, 1)
fn prepare_nrd(@builtin(global_invocation_id) gid: vec3u) {
    let p = gid.xy;
    if any(p >= textureDimensions(raw_diffuse)) { return; }
    let depth = textureLoad(depth_buffer, p, 0);
    let gpixel = textureLoad(gbuffer, p, 0);
    textureStore(motion_out, p, textureLoad(motion, p, 0));
    if depth == 0.0 || gpixel.a == 0u {
        textureStore(view_z_out, p, vec4f(1e6));
        textureStore(normal_out, p, vec4f(0.5, 0.5, 1.0, 0.0));
        textureStore(diffuse_in, p, vec4f(0.0));
        textureStore(specular_in, p, vec4f(0.0));
        return;
    }
    let s = gpixel_resolve(gpixel, depth, p, view.main_pass_viewport.zw, view.world_from_clip);
    let diffuse_factor = max(s.material.base_color * (1.0 - s.material.metallic), vec3f(0.01));
    let specular_factor = max(mix(vec3f(0.16 * s.material.reflectance * s.material.reflectance), s.material.base_color, s.material.metallic), vec3f(0.01));
    let d = textureLoad(raw_diffuse, p, 0);
    let r = textureLoad(raw_specular, p, 0);
    textureStore(normal_out, p, encode_normal(s.world_normal, s.material.roughness));
    textureStore(view_z_out, p, vec4f(max(-(view.view_from_world * vec4f(s.world_position, 1.0)).z, 0.0)));
    textureStore(diffuse_in, p, vec4f(max(d.rgb / diffuse_factor, vec3f(0.0)), min(d.a, 65504.0)));
    textureStore(specular_in, p, vec4f(max(r.rgb / specular_factor, vec3f(0.0)), min(r.a, 65504.0)));
}

@compute @workgroup_size(8, 8, 1)
fn resolve_nrd(@builtin(global_invocation_id) gid: vec3u) {
    let p = gid.xy;
    if any(p >= textureDimensions(raw_diffuse)) { return; }
    let depth = textureLoad(depth_buffer, p, 0);
    let gpixel = textureLoad(gbuffer, p, 0);
    if depth == 0.0 || gpixel.a == 0u { return; }
    let s = gpixel_resolve(gpixel, depth, p, view.main_pass_viewport.zw, view.world_from_clip);
    let diffuse_factor = max(s.material.base_color * (1.0 - s.material.metallic), vec3f(0.01));
    let specular_factor = max(mix(vec3f(0.16 * s.material.reflectance * s.material.reflectance), s.material.base_color, s.material.metallic), vec3f(0.01));
    let color = textureLoad(diffuse_out, p, 0).rgb * diffuse_factor
        + textureLoad(specular_out, p, 0).rgb * specular_factor + s.material.emissive * view.exposure;
    textureStore(view_output, p, vec4f(color, 1.0));
}
