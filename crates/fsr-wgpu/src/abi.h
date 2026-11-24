/*
 * Pinned AMD FidelityFX SDK FfxApi C ABI, transcribed for provenance.
 *
 * Source (MIT): https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK
 *   Kits/FidelityFX/api/include/ffx_api.h
 *   Kits/FidelityFX/api/include/ffx_api_types.h
 *   Kits/FidelityFX/api/include/ffx_api_loader.h
 *   Kits/FidelityFX/api/include/vk/ffx_api_vk.h
 *   Kits/FidelityFX/upscalers/include/ffx_upscale.h
 *   Kits/FidelityFX/upscalers/include/ffx_upscale.hpp
 *
 * Copyright (C) 2026 Advanced Micro Devices, Inc.
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 * The above copyright notice and this permission notice shall be included in
 * all copies or substantial portions of the Software.
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
 *
 * This header is documentation only. Rust `#[repr(C)]` declarations in
 * src/native.rs mirror it; the compile-time size assertions there pin the
 * x86_64 layout. Regenerate both together when the SDK headers change.
 *
 * AMD's own ffx_api_loader.h loads these five exports with LoadLibrary /
 * GetProcAddress at runtime. We do the same, so no AMD binary is a link input.
 */
#pragma once
#include <stdint.h>
#include <stdbool.h>

typedef void*     ffxContext;
typedef uint32_t  ffxReturnCode_t;
typedef uint64_t  ffxStructType_t;

typedef struct ffxApiHeader {
    ffxStructType_t      type;
    struct ffxApiHeader* pNext;
} ffxApiHeader;
typedef ffxApiHeader ffxCreateContextDescHeader;
typedef ffxApiHeader ffxConfigureDescHeader;
typedef ffxApiHeader ffxQueryDescHeader;
typedef ffxApiHeader ffxDispatchDescHeader;

typedef void (*ffxApiMessage)(uint32_t type, const wchar_t* message);
typedef void* (*ffxAlloc)(void* pUserData, uint64_t size);
typedef void  (*ffxDealloc)(void* pUserData, void* pMem);
typedef struct ffxAllocationCallbacks { void* pUserData; ffxAlloc alloc; ffxDealloc dealloc; } ffxAllocationCallbacks;

ffxReturnCode_t ffxCreateContext (ffxContext* context, ffxCreateContextDescHeader* desc, const ffxAllocationCallbacks* memCb);
ffxReturnCode_t ffxDestroyContext(ffxContext* context, const ffxAllocationCallbacks* memCb);
ffxReturnCode_t ffxConfigure    (ffxContext* context, const ffxConfigureDescHeader* desc);
ffxReturnCode_t ffxQuery        (ffxContext* context, ffxQueryDescHeader* desc);
ffxReturnCode_t ffxDispatch     (ffxContext* context, const ffxDispatchDescHeader* desc);

/* ffx_api_types.h */
struct FfxApiDimensions2D  { uint32_t width, height; };
struct FfxApiFloatCoords2D { float x, y; };
struct FfxApiResourceDescription {
    uint32_t type, format, width /*size*/, height /*stride*/, depth /*alignment*/;
    uint32_t mipCount, flags, usage;
};
struct FfxApiResource {
    void* resource;                          /* Vulkan: VkImage / VkBuffer handle */
    struct FfxApiResourceDescription description;
    uint32_t state;
};

/* ffx_upscale.h */
#define FFX_API_CREATE_CONTEXT_DESC_TYPE_UPSCALE 0x00010000u
struct ffxCreateContextDescUpscale {
    ffxCreateContextDescHeader header;
    uint32_t                   flags;
    struct FfxApiDimensions2D  maxRenderSize;
    struct FfxApiDimensions2D  maxUpscaleSize;
    ffxApiMessage              fpMessage;
};

#define FFX_API_DISPATCH_DESC_TYPE_UPSCALE 0x00010001u
struct ffxDispatchDescUpscale {
    ffxDispatchDescHeader      header;
    void*                      commandList;      /* Vulkan: VkCommandBuffer */
    struct FfxApiResource      color, depth, motionVectors, exposure;
    struct FfxApiResource      reactive, transparencyAndComposition, output;
    struct FfxApiFloatCoords2D jitterOffset, motionVectorScale;
    struct FfxApiDimensions2D  renderSize, upscaleSize;
    bool                       enableSharpening;
    float                      sharpness, frameTimeDelta, preExposure;
    bool                       reset;
    float                      cameraNear, cameraFar, cameraFovAngleVertical;
    float                      viewSpaceToMetersFactor;
    uint32_t                   flags;
};

#define FFX_API_QUERY_DESC_TYPE_UPSCALE_GETUPSCALERATIOFROMQUALITYMODE       0x00010002u
#define FFX_API_QUERY_DESC_TYPE_UPSCALE_GETRENDERRESOLUTIONFROMQUALITYMODE   0x00010003u
#define FFX_API_QUERY_DESC_TYPE_UPSCALE_GETJITTERPHASECOUNT                  0x00010004u
#define FFX_API_QUERY_DESC_TYPE_UPSCALE_GETJITTEROFFSET                      0x00010005u
struct ffxQueryDescUpscaleGetUpscaleRatioFromQualityMode {
    ffxQueryDescHeader header; uint32_t qualityMode; float* pOutUpscaleRatio;
};
struct ffxQueryDescUpscaleGetRenderResolutionFromQualityMode {
    ffxQueryDescHeader header; uint32_t displayWidth, displayHeight, qualityMode;
    uint32_t *pOutRenderWidth, *pOutRenderHeight;
};
struct ffxQueryDescUpscaleGetJitterPhaseCount {
    ffxQueryDescHeader header; uint32_t renderWidth, displayWidth; int32_t* pOutPhaseCount;
};
struct ffxQueryDescUpscaleGetJitterOffset {
    ffxQueryDescHeader header; int32_t index, phaseCount; float *pOutX, *pOutY;
};

/* ffx_api_vk.h — chained from ffxCreateContextDescUpscale.header.pNext */
typedef struct VkDeviceContext {
    VkDevice             vkDevice;
    VkPhysicalDevice     vkPhysicalDevice;
    PFN_vkGetDeviceProcAddr vkDeviceProcAddr;
} VkDeviceContext;
struct ffxCreateBackendVKDesc {
    ffxCreateContextDescHeader header;
    VkDeviceContext            deviceContext;
};
/* ffx_api.h (2.4.0): FFX_API_BACKEND_ID_VK = 0x01000000, DX12 = 0x00000000,
 * XBOX = 0x02000000. FFX_API_MAKE_BACKEND_SUB_ID(id, 0x03) gives 0x01000003
 * for Vulkan; 0x00000003 is the 1.1.x deprecated type; 0x02000003 is Xbox.
 * The struct layout is identical; Context::new retries the Vulkan types. */

/* ffx_api_dx12.h — chained on any effect create desc for the DX12 backend */
#define FFX_API_CREATE_CONTEXT_DESC_TYPE_BACKEND_DX12 0x00000002u
struct ffxCreateBackendDX12Desc {
    ffxCreateContextDescHeader header;
    ID3D12Device*              device;
};

/* ffx_upscale.h — version descriptor chained on the upscaler create desc */
#define FFX_UPSCALER_VERSION_MAJOR 4
#define FFX_UPSCALER_VERSION_MINOR 1
#define FFX_UPSCALER_VERSION_PATCH 2
#define FFX_UPSCALER_MAKE_VERSION(major, minor, patch) (((major) << 22) | ((minor) << 12) | (patch))
#define FFX_UPSCALER_VERSION FFX_UPSCALER_MAKE_VERSION(4, 1, 2)
#define FFX_API_CREATE_CONTEXT_DESC_TYPE_UPSCALE_VERSION 0x0001000bu
struct ffxCreateContextDescUpscaleVersion {
    ffxCreateContextDescHeader header;
    uint32_t                   version; /* FFX_UPSCALER_VERSION */
};

/* ffx_denoiser.h — FSR Ray Regeneration (DX12) */
#define FFX_API_EFFECT_ID_DENOISER 0x00050000u
#define FFX_DENOISER_VERSION FFX_DENOISER_MAKE_VERSION(1, 2, 1)
#define FFX_API_CREATE_CONTEXT_DESC_TYPE_DENOISER 0x00050001u
struct ffxCreateContextDescDenoiser {
    ffxCreateContextDescHeader header; uint32_t version;
    struct FfxApiDimensions2D maxRenderSize; uint32_t signalFlags;
    uint32_t checkerboardSignalFlags; uint32_t flags;
};
#define FFX_API_DISPATCH_DESC_TYPE_DENOISER 0x00050041u
struct ffxDispatchDescDenoiser {
    ffxDispatchDescHeader header; void* commandList;
    struct FfxApiResource linearDepth, motionVectors, normals, specularAlbedo, diffuseAlbedo;
    struct FfxApiFloatCoords3D motionVectorScale; struct FfxApiFloatCoords2D jitterOffsets;
    struct FfxApiFloatCoords3D cameraPositionDelta;
    FfxApiMatrix4x4 view, projection; struct FfxApiFloatBounds linearDepthBounds;
    struct FfxApiDimensions2D renderSize; uint32_t frameIndex, flags;
};

/* ffx_radiancecache.h — neural radiance caching (DX12, technical preview 0.9.0) */
#define FFX_API_EFFECT_ID_RADIANCECACHE 0x00060000u
#define FFX_RADIANCECACHE_VERSION FFX_RADIANCECACHE_MAKE_VERSION(0, 9, 0)
#define FFX_API_CREATE_CONTEXT_DESC_TYPE_RADIANCECACHE 0x00060002u
struct ffxCreateContextDescRadianceCache {
    ffxCreateContextDescHeader header; uint32_t flags, version;
    uint32_t maxInferenceSampleCount, maxTrainingSampleCount;
};
#define FFX_API_DISPATCH_DESC_TYPE_RADIANCECACHE 0x00060004u
struct ffxDispatchDescRadianceCache {
    ffxDispatchDescHeader header; void* commandList;
    struct FfxApiResource predictionInputs, predictionOutputs, trainInputs, trainTargets, sampleCounters;
    uint32_t flags;
    struct { float learningRate; float weightSmoothing; } overrides;
};

/* ffx_framegeneration.h — FSR frame generation (Vulkan + DX12) */
#define FFX_API_EFFECT_ID_FRAMEGENERATION 0x00020000u
#define FFX_FRAMEGENERATION_VERSION FFX_FRAMEGENERATION_MAKE_VERSION(4, 0, 2)
#define FFX_API_CREATE_CONTEXT_DESC_TYPE_FRAMEGENERATION 0x00020001u
struct ffxCreateContextDescFrameGeneration {
    ffxCreateContextDescHeader header; uint32_t flags;
    struct FfxApiDimensions2D displaySize, maxRenderSize; uint32_t backBufferFormat;
};
#define FFX_API_DISPATCH_DESC_TYPE_FRAMEGENERATION 0x00020003u
struct ffxDispatchDescFrameGeneration {
    ffxDispatchDescHeader header; void* commandList;
    struct FfxApiResource presentColor, outputs[4];
    uint32_t numGeneratedFrames; bool reset; uint32_t backbufferTransferFunction;
    float minMaxLuminance[2]; struct FfxApiRect2D generationRect; uint64_t frameID;
};

/* ffx_api.h — provider version enumeration / override */
#define FFX_API_QUERY_DESC_TYPE_GET_VERSIONS 4u
struct ffxQueryDescGetVersions {
    ffxQueryDescHeader header; uint64_t createDescType; void* device;
    uint64_t *outputCount, *versionIds; const char** versionNames;
};
#define FFX_API_DESC_TYPE_OVERRIDE_VERSION 5u
struct ffxOverrideVersion { ffxApiHeader header; uint64_t versionId; };

/* ffx_upscale.h — quality modes and flags */
enum FfxApiUpscaleQualityMode {
    FFX_UPSCALE_QUALITY_MODE_NATIVEAA = 0, QUALITY = 1, BALANCED = 2,
    PERFORMANCE = 3, ULTRA_PERFORMANCE = 4
};

/* =========================================================================
 * FidelityFX SDK 2.4.0 additional descriptors. Source (MIT, v2.4.0):
 *   Kits/FidelityFX/framegeneration/include/ffx_framegeneration.h
 *   Kits/FidelityFX/framegeneration/include/vk/ffx_api_framegeneration_vk.h
 *   Kits/FidelityFX/framegeneration/include/dx12/ffx_api_framegeneration_dx12.h
 *   Kits/FidelityFX/api/include/vk/ffx_api_vk.h
 *   Kits/FidelityFX/denoisers/include/ffx_denoiser.h
 *   Kits/FidelityFX/radiancecache/include/ffx_radiancecache.h
 *
 * Provenance / documentation ONLY (this header is never compiled as part of
 * the crate). All Vulkan/DX12 opaque handles and function pointers are modeled
 * as void* (8 bytes on x86_64); VkSwapchainCreateInfoKHR / VkAllocationCallbacks
 * are kept opaque. The matching #[repr(C)] Rust mirrors live in src/effects.rs;
 * their sizes were verified with an MSVC C++ sizeof probe.
 *
 * Effect sub-ids: (effect & 0x00ff0000) | (sub & ~0x00ff0000).
 * ========================================================================= */

/* Versions (make_version = major<<22 | minor<<12 | patch): 4.0.0 -> 0x01000000 */
#define FFX_FRAMEGENERATION_SWAPCHAIN_VK_VERSION   0x01000000u
#define FFX_FRAMEGENERATION_SWAPCHAIN_DX12_VERSION 0x01000000u

/* ---- Descriptor types (FGSWAPCHAIN Vulkan, effect 0x00040000) ---------- */
#define FFX_API_CREATE_CONTEXT_DESC_TYPE_FGSWAPCHAIN_VK                          0x00040001u
#define FFX_API_CONFIGURE_DESC_TYPE_FGSWAPCHAIN_REGISTERUIRESOURCE_VK             0x00040002u
#define FFX_API_QUERY_DESC_TYPE_FGSWAPCHAIN_INTERPOLATIONCOMMANDLIST_VK            0x00040003u
#define FFX_API_QUERY_DESC_TYPE_FGSWAPCHAIN_INTERPOLATIONTEXTURE_VK                 0x00040004u
#define FFX_API_QUERY_DESC_TYPE_FGSWAPCHAIN_FUNCTIONS_VK                           0x00040005u
#define FFX_API_DISPATCH_DESC_TYPE_FGSWAPCHAIN_WAIT_FOR_PRESENTS_VK                0x00040007u
#define FFX_API_CONFIGURE_DESC_TYPE_FRAMEGENERATIONSWAPCHAIN_KEYVALUE_VK           0x00040008u
#define FFX_API_QUERY_DESC_TYPE_FRAMEGENERATIONSWAPCHAIN_GPU_MEMORY_USAGE_VK        0x00040009u
#define FFX_API_CREATE_CONTEXT_DESC_TYPE_FRAMEGENERATIONSWAPCHAIN_VERSION_VK       0x0004000au
#define FFX_API_QUERY_DESC_TYPE_FRAMEGENERATIONSWAPCHAIN_GPU_MEMORY_USAGE_VK_V2    0x0004000bu
#define FFX_API_CREATE_CONTEXT_DESC_TYPE_FGSWAPCHAIN_MODE_VK                      0x00040010u

/* ---- Descriptor types (FGSWAPCHAIN DX12, effect 0x00030000) ------------- */
#define FFX_API_CREATE_CONTEXT_DESC_TYPE_FRAMEGENERATIONSWAPCHAIN_WRAP_DX12         0x00030001u
#define FFX_API_CREATE_CONTEXT_DESC_TYPE_FRAMEGENERATIONSWAPCHAIN_NEW_DX12          0x00030005u
#define FFX_API_CREATE_CONTEXT_DESC_TYPE_FRAMEGENERATIONSWAPCHAIN_FOR_HWND_DX12     0x00030006u
#define FFX_API_CREATE_CONTEXT_DESC_TYPE_FRAMEGENERATIONSWAPCHAIN_VERSION_DX12      0x0003000bu

/* ---- Descriptor types (frame generation, effect 0x00020000) ------------ */
/* 0x02 is the CONFIGURE descriptor; 0x04 is the deprecated PREPARE; 0x0c V2. */
#define FFX_API_CONFIGURE_DESC_TYPE_FRAMEGENERATION                    0x00020002u
#define FFX_API_DISPATCH_DESC_TYPE_FRAMEGENERATION_PREPARE           0x00020004u
#define FFX_API_CALLBACK_DESC_TYPE_FRAMEGENERATION_PRESENT            0x00020005u
#define FFX_API_CONFIGURE_DESC_TYPE_FRAMEGENERATION_KEYVALUE          0x00020006u
#define FFX_API_QUERY_DESC_TYPE_FRAMEGENERATION_GPU_MEMORY_USAGE      0x00020007u
#define FFX_API_DISPATCH_DESC_TYPE_FRAMEGENERATION_PREPARE_CAMERAINFO 0x0002000au
#define FFX_API_QUERY_DESC_TYPE_FRAMEGENERATION_GPU_MEMORY_USAGE_V2   0x0002000bu
#define FFX_API_DISPATCH_DESC_TYPE_FRAMEGENERATION_PREPARE_V2         0x0002000cu
#define FFX_API_CREATE_CONTEXT_DESC_TYPE_FRAMEGENERATION_VERSION      0x0002000eu

/* ---- Descriptor types (denoiser, effect 0x00050000) --------------------- */
/* 2.4.0: GPU-memory query is 0x82 (NOT 0x07); signals are 0x42..0x49. */
#define FFX_API_DISPATCH_DESC_TYPE_DENOISER_DEBUG_VIEW          0x00050042u
#define FFX_API_DISPATCH_DESC_TYPE_DENOISER_AMBIENT_OCCLUSION   0x00050043u
#define FFX_API_DISPATCH_DESC_TYPE_DENOISER_DIRECT_DIFFUSE       0x00050044u
#define FFX_API_DISPATCH_DESC_TYPE_DENOISER_DIRECT_SPECULAR      0x00050045u
#define FFX_API_DISPATCH_DESC_TYPE_DENOISER_DOMINANT_LIGHT      0x00050046u
#define FFX_API_DISPATCH_DESC_TYPE_DENOISER_INDIRECT_DIFFUSE    0x00050047u
#define FFX_API_DISPATCH_DESC_TYPE_DENOISER_INDIRECT_SPECULAR    0x00050048u
#define FFX_API_DISPATCH_DESC_TYPE_DENOISER_SPECULAR_OCCLUSION  0x00050049u
#define FFX_API_QUERY_DESC_TYPE_DENOISER_GET_DEFAULT_KEYVALUE   0x00050081u
#define FFX_API_QUERY_DESC_TYPE_DENOISER_GPU_MEMORY_USAGE        0x00050082u

/* ---- Descriptor types (radiance cache, effect 0x00060000) -------------- */
/* 2.4.0 defines NO gpu-memory query at 0x07; only a debug dispatch 0x05. */
#define FFX_API_DEBUG_DISPATCH_DESC_TYPE_RADIANCECACHE            0x00060005u

/* ---- Structures (modeled with void* opaque handles) ------------------- */
struct FfxApiDenoiserSignal {           /* 104 bytes */
    struct FfxApiResource input, output;
    uint32_t checkerboardOrigin;
};
struct VkQueueInfoFFXAPI {              /* 24 bytes */ void* queue; uint32_t familyIndex; void* submitFunc; };
struct ffxCreateContextDescFrameGenerationSwapChainVK {   /* 152 bytes */
    ffxCreateContextDescHeader header; void* physicalDevice, device; void** swapchain;
    void* allocator; const void* createInfo;
    struct VkQueueInfoFFXAPI gameQueue, asyncComputeQueue, presentQueue, imageAcquireQueue;
};
struct ffxQueryDescSwapchainReplacementFunctionsVK {       /* 72 bytes */
    ffxQueryDescHeader header;
    void* pOutCreateSwapchainFFXAPI; void* pOutDestroySwapchainFFXAPI;
    void* pOutGetSwapchainImagesKHR; void* pOutAcquireNextImageKHR;
    void* pOutQueuePresentKHR; void* pOutSetHdrMetadataEXT; void* pOutGetLastPresentCountFFXAPI;
};
struct ffxConfigureDescFrameGenerationSwapChainRegisterUiResourceVK { /* 72 bytes */
    ffxConfigureDescHeader header; struct FfxApiResource uiResource; uint32_t flags;
};
struct ffxQueryDescFrameGenerationSwapChainInterpolationCommandListVK { /* 24 bytes */
    ffxQueryDescHeader header; void** pOutCommandList;
};
struct ffxQueryDescFrameGenerationSwapChainInterpolationTextureVK { /* 24 bytes */
    ffxQueryDescHeader header; struct FfxApiResource* pOutTexture;
};
struct ffxDispatchDescFrameGenerationSwapChainWaitForPresentsVK { /* 16 */
    ffxDispatchDescHeader header;
};
struct ffxCreateContextDescFrameGenerationSwapChainModeVK { /* 24 */
    ffxCreateContextDescHeader header; bool composeOnPresentQueue;
};
struct ffxCreateContextDescFrameGenerationSwapChainVersionVK { /* 24 */
    ffxCreateContextDescHeader header; uint32_t version;
};
struct ffxConfigureDescFrameGenerationSwapChainKeyValueVK { /* 40 */
    ffxConfigureDescHeader header; uint64_t key, u64; void* ptr;
};
struct ffxDispatchDescFrameGenerationPrepare {              /* 184 */
    ffxDispatchDescHeader header; uint64_t frameID; uint32_t flags; void* commandList;
    struct FfxApiDimensions2D renderSize; struct FfxApiFloatCoords2D jitterOffset, motionVectorScale;
    float frameTimeDelta; bool unused_reset;
    float cameraNear, cameraFar, cameraFovAngleVertical, viewSpaceToMetersFactor;
    struct FfxApiResource depth, motionVectors;
};
struct ffxDispatchDescFrameGenerationPrepareV2 {            /* 232 */
    /* = Prepare, plus: */ float cameraPosition[3], cameraUp[3], cameraRight[3], cameraForward[3];
};
typedef struct ffxCallbackDescFrameGenerationPresent {       /* 192 */
    ffxDispatchDescHeader header; void* device; void* commandList;
    struct FfxApiResource currentBackBuffer, currentUI, outputSwapChainBuffer;
    bool isGeneratedFrame; uint64_t frameID;
} ffxCallbackDescFrameGenerationPresent;
struct ffxDispatchDescDenoiserDebugView {                    /* 80 */
    ffxDispatchDescHeader header; struct FfxApiResource output;
    struct FfxApiDimensions2D outputSize; uint32_t mode, viewportIndex;
};
/* All 0x43/0x44/0x45/0x47/0x48/0x49 signal descriptors are 120 bytes:
 * header + struct FfxApiDenoiserSignal signal. Dominant-light (0x46) is 152:
 * header + signal + FloatCoords3D direction + FloatCoords3D emission + float angularRadius. */
struct ffxDispatchDescDenoiserSignalBase {                   /* 120 */
    ffxDispatchDescHeader header; struct FfxApiDenoiserSignal signal;
};
struct ffxDispatchDescDenoiserDominantLight {               /* 152 */
    ffxDispatchDescHeader header; struct FfxApiDenoiserSignal signal;
    struct FfxApiFloatCoords3D direction, emission; float angularRadius;
};
