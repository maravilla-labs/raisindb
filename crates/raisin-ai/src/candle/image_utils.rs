//! Image processing utilities for Candle inference.

use candle_core::{DType, Device, Tensor};
use image::DynamicImage;

use super::{CandleError, CandleResult};

/// Load and preprocess an image for CLIP model inference.
///
/// CLIP expects images to be:
/// - Resized to 224x224 (or model-specific size)
/// - RGB format
/// - Normalized with ImageNet mean/std
pub fn preprocess_clip(image_bytes: &[u8], size: usize, device: &Device) -> CandleResult<Tensor> {
    let img = image::load_from_memory(image_bytes)
        .map_err(|e| CandleError::ImageProcessing(format!("Failed to load image: {}", e)))?;

    preprocess_clip_from_image(&img, size, device)
}

/// Preprocess a DynamicImage for CLIP inference.
pub fn preprocess_clip_from_image(
    img: &DynamicImage,
    size: usize,
    device: &Device,
) -> CandleResult<Tensor> {
    // Resize with center crop
    let img = img.resize_to_fill(
        size as u32,
        size as u32,
        image::imageops::FilterType::Triangle,
    );

    // Convert to RGB
    let img = img.to_rgb8();
    let (width, height) = img.dimensions();

    // Convert to tensor [H, W, C]
    let data = img.into_raw();
    let tensor = Tensor::from_vec(data, (height as usize, width as usize, 3), device)
        .map_err(|e| CandleError::Inference(format!("Failed to create tensor: {}", e)))?;

    // Permute to [C, H, W] and convert to f32
    let tensor = tensor
        .permute((2, 0, 1))
        .map_err(|e| CandleError::Inference(format!("Permute failed: {}", e)))?
        .to_dtype(DType::F32)
        .map_err(|e| CandleError::Inference(format!("Dtype conversion failed: {}", e)))?;

    // Normalize to [0, 1]
    let tensor =
        (tensor / 255.0).map_err(|e| CandleError::Inference(format!("Division failed: {}", e)))?;

    // Apply ImageNet normalization
    // mean = [0.48145466, 0.4578275, 0.40821073]
    // std = [0.26862954, 0.26130258, 0.27577711]
    let mean = Tensor::new(&[0.48145466f32, 0.4578275, 0.40821073], device)
        .map_err(|e| CandleError::Inference(format!("Mean tensor failed: {}", e)))?
        .reshape((3, 1, 1))
        .map_err(|e| CandleError::Inference(format!("Reshape failed: {}", e)))?;

    let std = Tensor::new(&[0.26862954f32, 0.261_302_6, 0.275_777_1], device)
        .map_err(|e| CandleError::Inference(format!("Std tensor failed: {}", e)))?
        .reshape((3, 1, 1))
        .map_err(|e| CandleError::Inference(format!("Reshape failed: {}", e)))?;

    let tensor = tensor
        .broadcast_sub(&mean)
        .map_err(|e| CandleError::Inference(format!("Subtraction failed: {}", e)))?
        .broadcast_div(&std)
        .map_err(|e| CandleError::Inference(format!("Division failed: {}", e)))?;

    // Add batch dimension [1, C, H, W]
    tensor
        .unsqueeze(0)
        .map_err(|e| CandleError::Inference(format!("Unsqueeze failed: {}", e)))
}

/// Load and preprocess an image for BLIP model inference.
///
/// BLIP expects images to be:
/// - Resized to 384x384 (or model-specific size)
/// - RGB format
/// - Normalized with BLIP-specific values
pub fn preprocess_blip(image_bytes: &[u8], size: usize, device: &Device) -> CandleResult<Tensor> {
    let img = image::load_from_memory(image_bytes)
        .map_err(|e| CandleError::ImageProcessing(format!("Failed to load image: {}", e)))?;

    preprocess_blip_from_image(&img, size, device)
}

/// Preprocess a DynamicImage for BLIP inference.
pub fn preprocess_blip_from_image(
    img: &DynamicImage,
    size: usize,
    device: &Device,
) -> CandleResult<Tensor> {
    // Resize with center crop
    let img = img.resize_to_fill(
        size as u32,
        size as u32,
        image::imageops::FilterType::Triangle,
    );

    // Convert to RGB
    let img = img.to_rgb8();
    let (width, height) = img.dimensions();

    // Convert to tensor [H, W, C]
    let data = img.into_raw();
    let tensor = Tensor::from_vec(data, (height as usize, width as usize, 3), device)
        .map_err(|e| CandleError::Inference(format!("Failed to create tensor: {}", e)))?;

    // Permute to [C, H, W] and convert to f32
    let tensor = tensor
        .permute((2, 0, 1))
        .map_err(|e| CandleError::Inference(format!("Permute failed: {}", e)))?
        .to_dtype(DType::F32)
        .map_err(|e| CandleError::Inference(format!("Dtype conversion failed: {}", e)))?;

    // Normalize to [0, 1]
    let tensor =
        (tensor / 255.0).map_err(|e| CandleError::Inference(format!("Division failed: {}", e)))?;

    // Apply BLIP normalization (same as CLIP/ImageNet)
    let mean = Tensor::new(&[0.48145466f32, 0.4578275, 0.40821073], device)
        .map_err(|e| CandleError::Inference(format!("Mean tensor failed: {}", e)))?
        .reshape((3, 1, 1))
        .map_err(|e| CandleError::Inference(format!("Reshape failed: {}", e)))?;

    let std = Tensor::new(&[0.26862954f32, 0.261_302_6, 0.275_777_1], device)
        .map_err(|e| CandleError::Inference(format!("Std tensor failed: {}", e)))?
        .reshape((3, 1, 1))
        .map_err(|e| CandleError::Inference(format!("Reshape failed: {}", e)))?;

    let tensor = tensor
        .broadcast_sub(&mean)
        .map_err(|e| CandleError::Inference(format!("Subtraction failed: {}", e)))?
        .broadcast_div(&std)
        .map_err(|e| CandleError::Inference(format!("Division failed: {}", e)))?;

    // Add batch dimension [1, C, H, W]
    tensor
        .unsqueeze(0)
        .map_err(|e| CandleError::Inference(format!("Unsqueeze failed: {}", e)))
}

/// Load and preprocess an image for Moondream model inference.
///
/// Moondream expects images to be:
/// - Resized to 378x378
/// - RGB format
/// - Normalized with mean/std of [0.5, 0.5, 0.5]
pub fn preprocess_moondream(image_bytes: &[u8], device: &Device) -> CandleResult<Tensor> {
    let img = image::load_from_memory(image_bytes)
        .map_err(|e| CandleError::ImageProcessing(format!("Failed to load image: {}", e)))?;

    preprocess_moondream_from_image(&img, device)
}

/// Moondream image size (378x378).
const MOONDREAM_IMAGE_SIZE: usize = 378;

/// Preprocess a DynamicImage for Moondream inference.
pub fn preprocess_moondream_from_image(
    img: &DynamicImage,
    device: &Device,
) -> CandleResult<Tensor> {
    // Resize with center crop
    let img = img.resize_to_fill(
        MOONDREAM_IMAGE_SIZE as u32,
        MOONDREAM_IMAGE_SIZE as u32,
        image::imageops::FilterType::Triangle,
    );

    // Convert to RGB
    let img = img.to_rgb8();
    let (width, height) = img.dimensions();

    // Convert to tensor [H, W, C]
    let data = img.into_raw();
    let tensor = Tensor::from_vec(data, (height as usize, width as usize, 3), device)
        .map_err(|e| CandleError::Inference(format!("Failed to create tensor: {}", e)))?;

    // Permute to [C, H, W] and convert to f32
    let tensor = tensor
        .permute((2, 0, 1))
        .map_err(|e| CandleError::Inference(format!("Permute failed: {}", e)))?
        .to_dtype(DType::F32)
        .map_err(|e| CandleError::Inference(format!("Dtype conversion failed: {}", e)))?;

    // Normalize to [0, 1]
    let tensor =
        (tensor / 255.0).map_err(|e| CandleError::Inference(format!("Division failed: {}", e)))?;

    // Apply Moondream normalization (mean=0.5, std=0.5)
    let mean = Tensor::new(&[0.5f32, 0.5, 0.5], device)
        .map_err(|e| CandleError::Inference(format!("Mean tensor failed: {}", e)))?
        .reshape((3, 1, 1))
        .map_err(|e| CandleError::Inference(format!("Reshape failed: {}", e)))?;

    let std = Tensor::new(&[0.5f32, 0.5, 0.5], device)
        .map_err(|e| CandleError::Inference(format!("Std tensor failed: {}", e)))?
        .reshape((3, 1, 1))
        .map_err(|e| CandleError::Inference(format!("Reshape failed: {}", e)))?;

    let tensor = tensor
        .broadcast_sub(&mean)
        .map_err(|e| CandleError::Inference(format!("Subtraction failed: {}", e)))?
        .broadcast_div(&std)
        .map_err(|e| CandleError::Inference(format!("Division failed: {}", e)))?;

    // Add batch dimension [1, C, H, W]
    tensor
        .unsqueeze(0)
        .map_err(|e| CandleError::Inference(format!("Unsqueeze failed: {}", e)))
}

/// Normalize a vector to unit length (L2 normalization).
pub fn l2_normalize(vector: &[f32]) -> Vec<f32> {
    let norm: f32 = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        vector.iter().map(|x| x / norm).collect()
    } else {
        vector.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_l2_normalize() {
        let vec = vec![3.0, 4.0];
        let normalized = l2_normalize(&vec);

        // 3-4-5 triangle, so normalized should be [0.6, 0.8]
        assert!((normalized[0] - 0.6).abs() < 0.001);
        assert!((normalized[1] - 0.8).abs() < 0.001);

        // Check unit length
        let norm: f32 = normalized.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_l2_normalize_zero() {
        let vec = vec![0.0, 0.0, 0.0];
        let normalized = l2_normalize(&vec);
        assert_eq!(normalized, vec);
    }

    #[test]
    fn test_l2_normalize_single_value() {
        let vec = vec![5.0];
        let normalized = l2_normalize(&vec);
        assert!((normalized[0] - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_l2_normalize_unit_vector() {
        let vec = vec![1.0, 0.0, 0.0];
        let normalized = l2_normalize(&vec);
        assert_eq!(normalized, vec);
    }

    #[test]
    fn test_l2_normalize_negative_values() {
        let vec = vec![-3.0, 4.0];
        let normalized = l2_normalize(&vec);

        // Norm is 5, so [-0.6, 0.8]
        assert!((normalized[0] - (-0.6)).abs() < 0.001);
        assert!((normalized[1] - 0.8).abs() < 0.001);
    }

    #[test]
    fn test_l2_normalize_embedding_dimension() {
        // Test with typical embedding dimension (512)
        let vec: Vec<f32> = (0..512).map(|i| i as f32 * 0.001).collect();
        let normalized = l2_normalize(&vec);

        // Check unit length
        let norm: f32 = normalized.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.001);
        assert_eq!(normalized.len(), 512);
    }

    // Helper to create a test JPEG image bytes
    fn create_test_jpeg(width: u32, height: u32) -> Vec<u8> {
        use image::{DynamicImage, Rgb, RgbImage};

        let mut img = RgbImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let r = ((x * 255) / width) as u8;
                let g = ((y * 255) / height) as u8;
                let b = 128u8;
                img.put_pixel(x, y, Rgb([r, g, b]));
            }
        }

        let mut buffer = std::io::Cursor::new(Vec::new());
        let dynamic = DynamicImage::ImageRgb8(img);
        dynamic
            .write_to(&mut buffer, image::ImageFormat::Jpeg)
            .expect("Failed to encode JPEG");

        buffer.into_inner()
    }

    #[test]
    fn test_preprocess_clip_output_shape() {
        let jpeg_bytes = create_test_jpeg(640, 480);
        let device = Device::Cpu;

        let tensor = preprocess_clip(&jpeg_bytes, 224, &device).unwrap();
        let shape = tensor.dims();

        // Should be [1, 3, 224, 224] - batch, channels, height, width
        assert_eq!(shape.len(), 4);
        assert_eq!(shape[0], 1, "Batch size should be 1");
        assert_eq!(shape[1], 3, "Should have 3 color channels");
        assert_eq!(shape[2], 224, "Height should be 224");
        assert_eq!(shape[3], 224, "Width should be 224");
    }

    #[test]
    fn test_preprocess_blip_output_shape() {
        let jpeg_bytes = create_test_jpeg(640, 480);
        let device = Device::Cpu;

        let tensor = preprocess_blip(&jpeg_bytes, 384, &device).unwrap();
        let shape = tensor.dims();

        // Should be [1, 3, 384, 384]
        assert_eq!(shape.len(), 4);
        assert_eq!(shape[0], 1, "Batch size should be 1");
        assert_eq!(shape[1], 3, "Should have 3 color channels");
        assert_eq!(shape[2], 384, "Height should be 384");
        assert_eq!(shape[3], 384, "Width should be 384");
    }

    #[test]
    fn test_preprocess_clip_values_normalized() {
        let jpeg_bytes = create_test_jpeg(100, 100);
        let device = Device::Cpu;

        let tensor = preprocess_clip(&jpeg_bytes, 224, &device).unwrap();
        let values = tensor.flatten_all().unwrap().to_vec1::<f32>().unwrap();

        // After ImageNet normalization, values should be roughly in [-2, 3] range
        let min = values.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = values.iter().cloned().fold(f32::NEG_INFINITY, f32::max);

        assert!(min > -10.0, "Min value {} is too low", min);
        assert!(max < 10.0, "Max value {} is too high", max);
    }

    #[test]
    fn test_preprocess_clip_small_image() {
        // Test with smaller than target size
        let jpeg_bytes = create_test_jpeg(50, 50);
        let device = Device::Cpu;

        let tensor = preprocess_clip(&jpeg_bytes, 224, &device).unwrap();
        let shape = tensor.dims();

        assert_eq!(shape[2], 224, "Should upscale to 224");
        assert_eq!(shape[3], 224, "Should upscale to 224");
    }

    #[test]
    fn test_preprocess_clip_large_image() {
        // Test with larger than target size
        let jpeg_bytes = create_test_jpeg(1024, 768);
        let device = Device::Cpu;

        let tensor = preprocess_clip(&jpeg_bytes, 224, &device).unwrap();
        let shape = tensor.dims();

        assert_eq!(shape[2], 224, "Should downscale to 224");
        assert_eq!(shape[3], 224, "Should downscale to 224");
    }

    #[test]
    fn test_preprocess_clip_invalid_image() {
        let invalid_bytes = b"not an image";
        let device = Device::Cpu;

        let result = preprocess_clip(invalid_bytes, 224, &device);
        assert!(result.is_err());
    }

    #[test]
    fn test_preprocess_from_dynamic_image() {
        use image::{DynamicImage, Rgb, RgbImage};

        let mut img = RgbImage::new(100, 100);
        for y in 0..100 {
            for x in 0..100 {
                img.put_pixel(x, y, Rgb([255, 0, 0])); // Red image
            }
        }
        let dynamic = DynamicImage::ImageRgb8(img);
        let device = Device::Cpu;

        let tensor = preprocess_clip_from_image(&dynamic, 224, &device).unwrap();
        let shape = tensor.dims();

        assert_eq!(shape, &[1, 3, 224, 224]);
    }
}
