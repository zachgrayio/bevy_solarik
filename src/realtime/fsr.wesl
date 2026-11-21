// Compose AMD FidelityFX FSR's upscaled output into the camera's HDR target.
// FSR writes into a dedicated texture; this pass scales/copies it into the view.
// With NativeAA both textures share the same size and this is a 1:1 resolve.

@group(0) @binding(0) var upscaled: texture_2d<f32>;
@group(0) @binding(1) var dst: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8)
fn compose_fsr(@builtin(global_invocation_id) id: vec3<u32>) {
    let out_size = textureDimensions(dst);
    if (id.x >= out_size.x || id.y >= out_size.y) {
        return;
    }
    let src_size = vec2<f32>(textureDimensions(upscaled));
    let uv = (vec2<f32>(id.xy) + vec2<f32>(0.5, 0.5)) / vec2<f32>(out_size);
    let src_pixel = vec2<i32>(min(uv * src_size, src_size - vec2<f32>(1.0, 1.0)));
    let color = textureLoad(upscaled, src_pixel, 0);
    textureStore(dst, vec2<i32>(id.xy), color);
}
