// This software contains source code provided by NVIDIA Corporation.
// NRD itself is built separately under its NVIDIA RTX SDK license, never vendored here.
#include "bridge.h"
#include <NRD.h>
#include <cstring>
#include <vector>
static_assert(sizeof(SnrdPipeline) == 40 && sizeof(SnrdDispatch) == 40 && sizeof(SnrdFrame) == 272);
static_assert(NRD_VERSION_MAJOR == 4 && NRD_VERSION_MINOR == 18 && NRD_VERSION_BUILD == 0);
struct State {
    nrd::Instance* instance = nullptr;
    const nrd::DispatchDesc* dispatches = nullptr;
    uint32_t count = 0;
    std::vector<std::vector<SnrdResource>> resources;
};
static uint32_t format(nrd::Format f) {
    switch(f) {
    case nrd::Format::R8_UNORM: return 0;
    case nrd::Format::R8_UINT: return 1;
    case nrd::Format::RG8_UNORM: return 2;
    case nrd::Format::R16_SFLOAT: return 3;
    case nrd::Format::RG16_SFLOAT: return 4;
    case nrd::Format::RGBA16_SFLOAT: return 5;
    case nrd::Format::R32_SFLOAT: return 6;
    case nrd::Format::RG32_SFLOAT: return 7;
    case nrd::Format::RGBA32_SFLOAT: return 8;
    case nrd::Format::R16_UINT: return 9;
    case nrd::Format::RG16_UINT: return 10;
    case nrd::Format::RGBA16_UINT: return 11;
    case nrd::Format::R32_UINT: return 12;
    case nrd::Format::RG32_UINT: return 13;
    case nrd::Format::RGBA32_UINT: return 14;
    case nrd::Format::RG8_UINT: return 15;
    case nrd::Format::RGBA8_UNORM: return 16;
    case nrd::Format::R16_UNORM: return 17;
    case nrd::Format::RG16_UNORM: return 18;
    case nrd::Format::RGBA16_UNORM: return 19;
    case nrd::Format::R10_G10_B10_A2_UNORM: return 20;
    default: return UINT32_MAX;
    }
}
void* snrd_create() {
    const auto* library = nrd::GetLibraryDesc();
    if (library->versionMajor != 4 || library->versionMinor != 18 ||
        library->normalEncoding != nrd::NormalEncoding::R10_G10_B10_A2_UNORM ||
        library->roughnessEncoding != nrd::RoughnessEncoding::LINEAR) return nullptr;
    auto* state = new State;
    const nrd::DenoiserDesc denoiser {0, nrd::Denoiser::RELAX_DIFFUSE_SPECULAR};
    const nrd::InstanceCreationDesc creation {{}, &denoiser, 1};
    if (nrd::CreateInstance(creation, state->instance) != nrd::Result::SUCCESS) { delete state; return nullptr; }
    nrd::RelaxSettings settings {};
    settings.enableAntiFirefly = true;
    if (nrd::SetDenoiserSettings(*state->instance, 0, &settings) != nrd::Result::SUCCESS) {
        nrd::DestroyInstance(*state->instance); delete state; return nullptr;
    }
    return state;
}
void snrd_destroy(void* ptr) { auto* s = static_cast<State*>(ptr); nrd::DestroyInstance(*s->instance); delete s; }
uint32_t snrd_pipeline_count(void* ptr) { return nrd::GetInstanceDesc(*static_cast<State*>(ptr)->instance)->pipelinesNum; }
SnrdPipeline snrd_pipeline(void* ptr, uint32_t index) {
    const auto* d = nrd::GetInstanceDesc(*static_cast<State*>(ptr)->instance);
    const auto& p = d->pipelines[index];
    SnrdPipeline result {static_cast<const uint32_t*>(p.computeShaderSPIRV.bytecode), p.computeShaderSPIRV.size,
        0, 0, d->constantBufferAndSamplersSpaceIndex, d->resourcesSpaceIndex, d->constantBufferMaxDataSize};
    for (uint32_t r = 0; r < p.resourceRangesNum; ++r) {
        if (p.resourceRanges[r].descriptorType == nrd::DescriptorType::TEXTURE) result.textures += p.resourceRanges[r].descriptorsNum;
        else result.storage_textures += p.resourceRanges[r].descriptorsNum;
    }
    return result;
}
uint32_t snrd_texture_count(void* ptr, uint32_t permanent) {
    const auto* d = nrd::GetInstanceDesc(*static_cast<State*>(ptr)->instance);
    return permanent ? d->permanentPoolSize : d->transientPoolSize;
}
SnrdTexture snrd_texture(void* ptr, uint32_t permanent, uint32_t index) {
    const auto* d = nrd::GetInstanceDesc(*static_cast<State*>(ptr)->instance);
    const auto& t = permanent ? d->permanentPool[index] : d->transientPool[index];
    return {format(t.format), t.downsampleFactor};
}
static uint32_t kind(nrd::ResourceType type) {
    switch(type) {
    case nrd::ResourceType::IN_MV: return 0;
    case nrd::ResourceType::IN_NORMAL_ROUGHNESS: return 1;
    case nrd::ResourceType::IN_VIEWZ: return 2;
    case nrd::ResourceType::IN_DIFF_RADIANCE_HITDIST: return 3;
    case nrd::ResourceType::IN_SPEC_RADIANCE_HITDIST: return 4;
    case nrd::ResourceType::OUT_DIFF_RADIANCE_HITDIST: return 5;
    case nrd::ResourceType::OUT_SPEC_RADIANCE_HITDIST: return 6;
    case nrd::ResourceType::TRANSIENT_POOL: return 7;
    case nrd::ResourceType::PERMANENT_POOL: return 8;
    default: return UINT32_MAX;
    }
}
uint32_t snrd_update(void* ptr, const SnrdFrame* frame) {
    auto* s = static_cast<State*>(ptr);
    if (!frame || !frame->width || !frame->height || frame->width > 65535 || frame->height > 65535) return 2;
    nrd::CommonSettings common {};
    std::memcpy(common.viewToClipMatrix, frame->projection, 64);
    std::memcpy(common.viewToClipMatrixPrev, frame->previous_projection, 64);
    std::memcpy(common.worldToViewMatrix, frame->world_to_view, 64);
    std::memcpy(common.worldToViewMatrixPrev, frame->previous_world_to_view, 64);
    common.resourceSize[0] = common.resourceSizePrev[0] = common.rectSize[0] = common.rectSizePrev[0] = static_cast<uint16_t>(frame->width);
    common.resourceSize[1] = common.resourceSizePrev[1] = common.rectSize[1] = common.rectSizePrev[1] = static_cast<uint16_t>(frame->height);
    // Bevy vectors are current minus previous in normalized texture coordinates.
    common.motionVectorScale[0] = common.motionVectorScale[1] = -1.0f;
    common.isMotionVectorInWorldSpace = false;
    common.frameIndex = frame->index;
    common.accumulationMode = frame->reset ? nrd::AccumulationMode::CLEAR_AND_RESTART : nrd::AccumulationMode::CONTINUE;
    auto result = nrd::SetCommonSettings(*s->instance, common);
    if(result != nrd::Result::SUCCESS) return static_cast<uint32_t>(result);
    const nrd::Identifier id = 0;
    result = nrd::GetComputeDispatches(*s->instance, &id, 1, s->dispatches, s->count);
    if(result != nrd::Result::SUCCESS) return static_cast<uint32_t>(result);
    s->resources.resize(s->count);
    for(uint32_t i = 0; i < s->count; ++i) {
        auto& resources = s->resources[i]; resources.clear();
        for(uint32_t r = 0; r < s->dispatches[i].resourcesNum; ++r) {
            const auto& resource = s->dispatches[i].resources[r];
            resources.push_back({kind(resource.type), resource.indexInPool, resource.descriptorType == nrd::DescriptorType::STORAGE_TEXTURE ? 1u : 0u});
        }
    }
    return 0;
}
uint32_t snrd_dispatch_count(void* ptr) { return static_cast<State*>(ptr)->count; }
SnrdDispatch snrd_dispatch(void* ptr, uint32_t index) {
    const auto* s = static_cast<State*>(ptr);
    const auto& d = s->dispatches[index];
    return {d.constantBufferData, s->resources[index].data(), d.constantBufferDataSize,
        d.resourcesNum, d.pipelineIndex, d.gridWidth, d.gridHeight};
}
