use crate::{feature_info::with_feature_info, nvsdk_ngx::*};
use std::{
    any::Any,
    ptr,
    sync::{Arc, Mutex},
    thread,
};
use uuid::Uuid;
use wgpu::{Device, hal::api::Vulkan};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SdkBackend {
    Vulkan,
    #[cfg(all(windows, feature = "dx12"))]
    Dx12 {
        owns_shutdown: bool,
    },
}

/// Application-wide DLSS object (supports both Vulkan and Windows D3D12).
pub struct DlssSdk {
    pub(crate) parameters: *mut NVSDK_NGX_Parameter,
    pub(crate) device: Device,
    pub(crate) backend: SdkBackend,
    // Features retain this SDK through their ReleaseFeature call. These owners are released
    // only after this SDK's Drop body has destroyed its parameters.
    runtime_keepalives: Vec<Arc<dyn Any + Send + Sync>>,
}

impl DlssSdk {
    /// Retain an application-owned native runtime until all features and SDK parameters are
    /// destroyed. Repeated registration of the same owner is idempotent. This does not transfer
    /// feature handles or suppress any normal NGX destructor.
    pub fn retain_runtime(&mut self, owner: Arc<dyn Any + Send + Sync>) {
        if !self
            .runtime_keepalives
            .iter()
            .any(|retained| Arc::ptr_eq(retained, &owner))
        {
            self.runtime_keepalives.push(owner);
        }
    }

    /// Creates the DLSS SDK for the active wgpu backend (Vulkan or Windows D3D12).
    ///
    /// This should be done once per application.
    pub fn new(project_id: Uuid, device: Device) -> Result<Arc<Mutex<Self>>, DlssError> {
        check_for_updates(project_id);

        if unsafe { device.as_hal::<Vulkan>() }.is_some() {
            let mut parameters = ptr::null_mut();
            unsafe {
                let hal_device = device.as_hal::<Vulkan>().unwrap();
                let shared_instance = hal_device.shared_instance();
                let raw_instance = shared_instance.raw_instance();

                with_feature_info(project_id, Default::default(), |feature_info| {
                    check_ngx_result(NVSDK_NGX_VULKAN_Init_with_ProjectID(
                        feature_info.Identifier.v.ProjectDesc.ProjectId,
                        NVSDK_NGX_EngineType_NVSDK_NGX_ENGINE_TYPE_CUSTOM,
                        feature_info.Identifier.v.ProjectDesc.EngineVersion,
                        feature_info.ApplicationDataPath,
                        raw_instance.handle(),
                        hal_device.raw_physical_device(),
                        hal_device.raw_device().handle(),
                        shared_instance.entry().static_fn().get_instance_proc_addr,
                        raw_instance.fp_v1_0().get_device_proc_addr,
                        feature_info.FeatureInfo,
                        NVSDK_NGX_Version_NVSDK_NGX_Version_API,
                    ))
                })?;

                check_ngx_result(NVSDK_NGX_VULKAN_GetCapabilityParameters(&mut parameters))?;

                let mut dlss_supported = 0;
                let result = check_ngx_result(NVSDK_NGX_Parameter_GetI(
                    parameters,
                    NVSDK_NGX_Parameter_SuperSampling_Available.as_ptr().cast(),
                    &mut dlss_supported,
                ));
                if result.is_err() {
                    check_ngx_result(NVSDK_NGX_VULKAN_DestroyParameters(parameters))?;
                    result?;
                }
                if dlss_supported == 0 {
                    check_ngx_result(NVSDK_NGX_VULKAN_DestroyParameters(parameters))?;
                    return Err(DlssError::FeatureNotSupported);
                }
            }

            return Ok(Arc::new(Mutex::new(Self {
                parameters,
                device,
                backend: SdkBackend::Vulkan,
                runtime_keepalives: Vec::new(),
            })));
        }

        #[cfg(all(windows, feature = "dx12"))]
        if let Some(hal_device) = unsafe { device.as_hal::<wgpu::hal::api::Dx12>() } {
            let raw_device = hal_device.raw_device_ptr().cast::<ID3D12Device>();
            let owns_shutdown = if crate::dx12::is_d3d12_ngx_initialized() {
                false
            } else {
                with_feature_info(project_id, Default::default(), |feature_info| unsafe {
                    check_ngx_result(NVSDK_NGX_D3D12_Init_with_ProjectID(
                        feature_info.Identifier.v.ProjectDesc.ProjectId,
                        NVSDK_NGX_EngineType_NVSDK_NGX_ENGINE_TYPE_CUSTOM,
                        feature_info.Identifier.v.ProjectDesc.EngineVersion,
                        feature_info.ApplicationDataPath,
                        raw_device,
                        feature_info.FeatureInfo,
                        NVSDK_NGX_Version_NVSDK_NGX_Version_API,
                    ))
                })?;
                true
            };

            let mut parameters = ptr::null_mut();
            unsafe {
                check_ngx_result(NVSDK_NGX_D3D12_GetCapabilityParameters(&mut parameters))?;

                let mut dlss_supported = 0;
                let result = check_ngx_result(NVSDK_NGX_Parameter_GetI(
                    parameters,
                    NVSDK_NGX_Parameter_SuperSampling_Available.as_ptr().cast(),
                    &mut dlss_supported,
                ));
                if result.is_err() {
                    let _ = NVSDK_NGX_D3D12_DestroyParameters(parameters);
                    if owns_shutdown {
                        let _ = NVSDK_NGX_D3D12_Shutdown1(raw_device);
                    }
                    result?;
                }
                if dlss_supported == 0 {
                    let _ = NVSDK_NGX_D3D12_DestroyParameters(parameters);
                    if owns_shutdown {
                        let _ = NVSDK_NGX_D3D12_Shutdown1(raw_device);
                    }
                    return Err(DlssError::FeatureNotSupported);
                }
            }

            drop(hal_device);
            return Ok(Arc::new(Mutex::new(Self {
                parameters,
                device,
                backend: SdkBackend::Dx12 { owns_shutdown },
                runtime_keepalives: Vec::new(),
            })));
        }

        Err(DlssError::FeatureNotSupported)
    }

    /// Whether DLSS Ray Reconstruction (DLSSD) is supported by the active NGX
    /// device/driver (works on both Vulkan and DX12).
    pub fn ray_reconstruction_supported(&self) -> bool {
        let mut supported = 0;
        let ok = unsafe {
            check_ngx_result(NVSDK_NGX_Parameter_GetI(
                self.parameters,
                NVSDK_NGX_Parameter_SuperSamplingDenoising_Available
                    .as_ptr()
                    .cast(),
                &mut supported,
            ))
        };
        ok.is_ok() && supported != 0
    }

    /// Returns the number of bytes of VRAM allocated by DLSS.
    pub fn get_vram_allocated_bytes(&mut self) -> Result<u64, DlssError> {
        let mut vram_allocated_bytes = 0;
        check_ngx_result(unsafe {
            NGX_DLSS_GET_STATS(self.parameters, &mut vram_allocated_bytes)
        })?;
        Ok(vram_allocated_bytes)
    }
}

fn check_for_updates(project_id: Uuid) {
    thread::spawn(move || {
        with_feature_info(
            project_id,
            NVSDK_NGX_Feature_NVSDK_NGX_Feature_SuperSampling,
            |feature_info| unsafe {
                NVSDK_NGX_UpdateFeature(&feature_info.Identifier, feature_info.FeatureID);
            },
        );
        with_feature_info(
            project_id,
            NVSDK_NGX_Feature_NVSDK_NGX_Feature_RayReconstruction,
            |feature_info| unsafe {
                NVSDK_NGX_UpdateFeature(&feature_info.Identifier, feature_info.FeatureID);
            },
        );
    });
}

impl Drop for DlssSdk {
    fn drop(&mut self) {
        match self.backend {
            SdkBackend::Vulkan => unsafe {
                // Teardown can run after the render world is gone; NGX answers
                // "not initialized"/"feature not found" when something already released.
                // A drop cannot report failure, so release best-effort instead of panicking.
                if let Some(hal_device) = self.device.as_hal::<Vulkan>() {
                    let _ = hal_device.raw_device().device_wait_idle();
                    let _ = check_ngx_result(NVSDK_NGX_VULKAN_DestroyParameters(self.parameters));
                    let _ = check_ngx_result(NVSDK_NGX_VULKAN_Shutdown1(
                        hal_device.raw_device().handle(),
                    ));
                }
            },
            #[cfg(all(windows, feature = "dx12"))]
            SdkBackend::Dx12 { owns_shutdown } => {
                let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
                unsafe {
                    let _ = check_ngx_result(NVSDK_NGX_D3D12_DestroyParameters(self.parameters));
                    if owns_shutdown
                        && let Some(hal_device) = self.device.as_hal::<wgpu::hal::api::Dx12>()
                    {
                        let _ = check_ngx_result(NVSDK_NGX_D3D12_Shutdown1(
                            hal_device.raw_device_ptr().cast::<ID3D12Device>(),
                        ));
                    }
                }
            }
        }
    }
}

unsafe impl Send for DlssSdk {}
unsafe impl Sync for DlssSdk {}
