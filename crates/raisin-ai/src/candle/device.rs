//! Device selection for Candle inference.

use candle_core::Device;

use super::CandleResult;

/// Select the best available device for inference.
///
/// Preference order:
/// 1. CUDA (if available)
/// 2. Metal (on macOS with Apple Silicon)
/// 3. CPU (always available)
pub fn select_device(prefer_gpu: bool) -> CandleResult<Device> {
    if !prefer_gpu {
        return Ok(Device::Cpu);
    }

    // Try Metal on macOS (Apple Silicon)
    #[cfg(target_os = "macos")]
    {
        if let Ok(device) = Device::new_metal(0) {
            tracing::info!("Using Metal device for inference");
            return Ok(device);
        }
    }

    tracing::info!("Using CPU device for inference");
    Ok(Device::Cpu)
}

/// Get information about the device.
pub fn device_info(device: &Device) -> String {
    match device {
        Device::Cpu => "CPU".to_string(),
        Device::Cuda(_) => "CUDA GPU".to_string(),
        Device::Metal(_) => "Apple Metal GPU".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_cpu() {
        let device = select_device(false).unwrap();
        assert!(matches!(device, Device::Cpu));
    }

    #[test]
    fn test_device_info() {
        assert_eq!(device_info(&Device::Cpu), "CPU");
    }
}
