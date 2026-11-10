use std::io::Cursor;

use image::DynamicImage;

use crate::error::{CoreError, Result};

/// Decodes an image from arbitrary supported memory formats (PNG, JPEG) and normalizes it to PNG bytes.
pub fn normalize_to_png(bytes: &[u8]) -> Result<Vec<u8>> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| CoreError::Image(format!("failed to decode image: {e}")))?;
    let mut cursor = Cursor::new(Vec::new());
    img.write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| CoreError::Image(format!("failed to encode PNG: {e}")))?;
    Ok(cursor.into_inner())
}

/// Converts raw RGBA pixel bytes (4 bytes per pixel: Red, Green, Blue, Alpha) into a PNG-encoded image.
///
/// Validates that `width` and `height` are non-zero and that `rgba.len() == width * height * 4`.
pub fn rgba_to_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    if width == 0 || height == 0 {
        return Err(CoreError::Image("image dimensions must be non-zero".into()));
    }

    let expected_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| CoreError::Image("image dimensions overflow buffer size".into()))?;

    if rgba.len() != expected_len {
        return Err(CoreError::Image(format!(
            "RGBA buffer length mismatch: expected {expected_len} bytes, got {}",
            rgba.len()
        )));
    }

    let img_buffer = image::RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| CoreError::Image("failed to construct RGBA buffer".into()))?;

    let dynamic = DynamicImage::ImageRgba8(img_buffer);
    let mut cursor = Cursor::new(Vec::new());
    dynamic
        .write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| CoreError::Image(format!("failed to encode PNG: {e}")))?;
    Ok(cursor.into_inner())
}

/// Decodes a PNG image and extracts its raw RGBA pixels along with width and height.
pub fn png_to_rgba(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|e| CoreError::Image(format!("failed to decode PNG: {e}")))?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok((width, height, rgba.into_raw()))
}

/// Generates a scaled-down PNG thumbnail preserving the aspect ratio.
///
/// Neither dimension of the resulting thumbnail will exceed `max_side` pixels.
pub fn thumbnail_png(bytes: &[u8], max_side: u32) -> Result<Vec<u8>> {
    if max_side == 0 {
        return Err(CoreError::Image("max_side must be greater than 0".into()));
    }

    let img = image::load_from_memory(bytes)
        .map_err(|e| CoreError::Image(format!("failed to decode image for thumbnail: {e}")))?;
    let thumb = img.thumbnail(max_side, max_side);
    let mut cursor = Cursor::new(Vec::new());
    thumb
        .write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| CoreError::Image(format!("failed to encode thumbnail PNG: {e}")))?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgba_to_png_and_back() {
        let width = 2u32;
        let height = 2u32;
        // 2x2 image = 4 pixels * 4 = 16 bytes
        let rgba_data = vec![
            255, 0, 0, 255, // red
            0, 255, 0, 255, // green
            0, 0, 255, 255, // blue
            255, 255, 255, 255, // white
        ];

        let png_bytes = rgba_to_png(width, height, &rgba_data).unwrap();
        assert!(!png_bytes.is_empty());

        let (w, h, decoded_rgba) = png_to_rgba(&png_bytes).unwrap();
        assert_eq!(w, width);
        assert_eq!(h, height);
        assert_eq!(decoded_rgba, rgba_data);
    }

    #[test]
    fn test_rgba_invalid_dimensions() {
        assert!(rgba_to_png(0, 10, &[0; 40]).is_err());
        assert!(rgba_to_png(10, 0, &[0; 40]).is_err());
        // Wrong buffer size
        assert!(rgba_to_png(2, 2, &[0; 10]).is_err());
    }

    #[test]
    fn test_thumbnail_png_bounds() {
        let width = 100u32;
        let height = 50u32;
        let rgba_data = vec![128u8; (width * height * 4) as usize];
        let png_bytes = rgba_to_png(width, height, &rgba_data).unwrap();

        let thumb_bytes = thumbnail_png(&png_bytes, 32).unwrap();
        let (tw, th, _) = png_to_rgba(&thumb_bytes).unwrap();

        assert!(tw <= 32);
        assert!(th <= 32);
        assert_eq!(tw, 32);
        assert_eq!(th, 16);
    }
}
