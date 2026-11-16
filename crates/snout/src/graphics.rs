//! Backend selection must exclude Vulkan before Windows adapter enumeration.

use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
use eframe::wgpu::Backends;

fn setup(backend_override: Option<Backends>) -> WgpuSetup {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    // Older Intel Vulkan drivers can crash during enumeration, even when DX12
    // would ultimately be selected. Retain explicit overrides for diagnostics.
    if let Some(backends) =
        backend_override.or(cfg!(target_os = "windows").then_some(Backends::DX12))
    {
        setup.instance_descriptor.backends = backends;
    }
    setup.into()
}

pub fn configuration() -> WgpuConfiguration {
    WgpuConfiguration {
        wgpu_setup: setup(Backends::from_env()),
        ..Default::default()
    }
    .with_surface_config(eframe::SurfaceConfig {
        present_mode: if cfg!(target_os = "linux") {
            eframe::wgpu::PresentMode::AutoNoVsync
        } else {
            eframe::wgpu::PresentMode::AutoVsync
        },
        desired_maximum_frame_latency: Some(2),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_default_excludes_vulkan_before_enumeration() {
        let WgpuSetup::CreateNew(setup) = setup(None) else {
            panic!("startup must create a graphics instance");
        };
        let expected = if cfg!(target_os = "windows") {
            Backends::DX12
        } else {
            WgpuSetupCreateNew::without_display_handle()
                .instance_descriptor
                .backends
        };
        assert_eq!(setup.instance_descriptor.backends, expected);
    }

    #[test]
    fn explicit_backend_overrides_are_preserved() {
        for backends in [
            Backends::DX12,
            Backends::VULKAN,
            Backends::GL | Backends::VULKAN,
        ] {
            let WgpuSetup::CreateNew(setup) = setup(Some(backends)) else {
                panic!("startup must create a graphics instance");
            };
            assert_eq!(setup.instance_descriptor.backends, backends);
        }
    }
}
