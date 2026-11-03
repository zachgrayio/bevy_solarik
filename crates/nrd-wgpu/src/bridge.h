#pragma once
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct SnrdPipeline {
    const uint32_t* spirv;
    uint64_t bytes;
    uint32_t textures;
    uint32_t storage_textures;
    uint32_t common_space;
    uint32_t resource_space;
    uint32_t constant_bytes;
} SnrdPipeline;
typedef struct SnrdTexture { uint32_t format; uint32_t downsample; } SnrdTexture;
typedef struct SnrdResource { uint32_t kind; uint32_t index; uint32_t storage; } SnrdResource;
typedef struct SnrdDispatch {
    const uint8_t* constants;
    const SnrdResource* resources;
    uint32_t constant_bytes;
    uint32_t resource_count;
    uint32_t pipeline;
    uint32_t width;
    uint32_t height;
} SnrdDispatch;
typedef struct SnrdFrame {
    float projection[16];
    float previous_projection[16];
    float world_to_view[16];
    float previous_world_to_view[16];
    uint32_t width;
    uint32_t height;
    uint32_t index;
    uint32_t reset;
} SnrdFrame;
void* snrd_create(void);
void snrd_destroy(void* instance);
uint32_t snrd_pipeline_count(void* instance);
SnrdPipeline snrd_pipeline(void* instance, uint32_t index);
uint32_t snrd_texture_count(void* instance, uint32_t permanent);
SnrdTexture snrd_texture(void* instance, uint32_t permanent, uint32_t index);
uint32_t snrd_update(void* instance, const SnrdFrame* frame);
uint32_t snrd_dispatch_count(void* instance);
SnrdDispatch snrd_dispatch(void* instance, uint32_t index);
#ifdef __cplusplus
}
#endif
