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
/* FFX_API_CREATE_CONTEXT_DESC_TYPE_BACKEND_VK discriminator by SDK generation:
 *   0x00000003 (1.1.x), 0x01000003 (current main), 0x02000003 (2.4.0).
 * The struct layout is identical; Context::new retries each value. */

/* ffx_upscale.h — quality modes and flags */
enum FfxApiUpscaleQualityMode {
    FFX_UPSCALE_QUALITY_MODE_NATIVEAA = 0, QUALITY = 1, BALANCED = 2,
    PERFORMANCE = 3, ULTRA_PERFORMANCE = 4
};
