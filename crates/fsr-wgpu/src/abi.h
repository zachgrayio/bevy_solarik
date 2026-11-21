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
