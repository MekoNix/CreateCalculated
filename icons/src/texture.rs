//! RGBA images: PNG decoding and encoding, and the first frame of animated textures.

use crate::assets::{texture_path, Assets};
use serde_json::Value;
use std::io::Cursor;

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[u8; 4]>,
}

impl Image {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, pixels: vec![[0; 4]; (width * height) as usize] }
    }

    pub fn get(&self, x: u32, y: u32) -> [u8; 4] {
        self.pixels[(y * self.width + x) as usize]
    }

    pub fn set(&mut self, x: u32, y: u32, color: [u8; 4]) {
        self.pixels[(y * self.width + x) as usize] = color;
    }

    pub fn is_blank(&self) -> bool {
        self.pixels.iter().all(|p| p[3] == 0)
    }

    pub fn decode_png(bytes: &[u8]) -> Result<Image, String> {
        let mut decoder = png::Decoder::new(Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16 | png::Transformations::ALPHA);
        let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
        let size = reader.output_buffer_size().ok_or("image too large")?;
        let mut data = vec![0; size];
        let frame = reader.next_frame(&mut data).map_err(|e| e.to_string())?;
        let data = &data[..frame.buffer_size()];
        let pixels: Vec<[u8; 4]> = match frame.color_type {
            png::ColorType::Rgba => data.as_chunks::<4>().0.to_vec(),
            png::ColorType::GrayscaleAlpha => data.as_chunks::<2>().0.iter().map(|&[v, a]| [v, v, v, a]).collect(),
            other => return Err(format!("unexpected colour type {other:?}")),
        };
        Ok(Image { width: frame.width, height: frame.height, pixels })
    }

    pub fn encode_png(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::High);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(self.pixels.as_flattened()).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
        Ok(out)
    }

    fn crop(&self, x: u32, y: u32, width: u32, height: u32) -> Image {
        let mut out = Image::new(width, height);
        for dy in 0..height {
            for dx in 0..width {
                out.set(dx, dy, self.get(x + dx, y + dy));
            }
        }
        out
    }

    /// Applies an `.mcmeta` the way the game cuts its sprites. Without an animation
    /// section the whole picture is the sprite, however tall it is.
    pub fn first_frame(self, meta: &Value) -> Image {
        let Some(animation) = meta.get("animation") else { return self };
        let side = self.width.min(self.height);
        let dimension = |key| animation.get(key).and_then(Value::as_u64).map(|v| v as u32);
        let (w, h) = (dimension("width").unwrap_or(side), dimension("height").unwrap_or(side));
        if w == 0 || h == 0 || w > self.width || h > self.height {
            return self;
        }
        let first = animation.pointer("/frames/0").and_then(|f| f.as_u64().or_else(|| f.get("index").and_then(Value::as_u64))).unwrap_or(0) as u32;
        let columns = self.width / w;
        let (x, y) = ((first % columns) * w, (first / columns) * h);
        if y + h > self.height {
            return self.crop(0, 0, w, h);
        }
        self.crop(x, y, w, h)
    }
}

/// Loads a texture by location, reduced to its first animation frame.
pub fn load(assets: &Assets, location: &str) -> Option<Image> {
    let path = texture_path(location);
    load_path(assets, &path)
}

pub fn load_path(assets: &Assets, path: &str) -> Option<Image> {
    let image = Image::decode_png(assets.get(path)?).ok()?;
    match assets.json(&format!("{path}.mcmeta")) {
        Some(meta) => Some(image.first_frame(&meta)),
        None => Some(image),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Three frames stacked: red, green, blue.
    fn strip() -> Image {
        let mut image = Image::new(2, 6);
        for y in 0..6 {
            let colour = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]][(y / 2) as usize];
            for x in 0..2 {
                image.set(x, y, colour);
            }
        }
        image
    }

    #[test]
    fn png_round_trip() {
        let image = strip();
        assert_eq!(Image::decode_png(&image.encode_png().unwrap()).unwrap(), image);
    }

    #[test]
    fn animated_texture_gives_its_first_frame() {
        let frame = strip().first_frame(&json!({"animation": {"frametime": 2}}));
        assert_eq!((frame.width, frame.height), (2, 2));
        assert_eq!(frame.get(1, 1), [255, 0, 0, 255]);
    }

    #[test]
    fn frame_order_can_start_elsewhere() {
        let frame = strip().first_frame(&json!({"animation": {"frames": [{"index": 2, "time": 5}, 0]}}));
        assert_eq!(frame.get(0, 0), [0, 0, 255, 255]);
        let frame = strip().first_frame(&json!({"animation": {"frames": [1, 0]}}));
        assert_eq!(frame.get(0, 0), [0, 255, 0, 255]);
    }

    #[test]
    fn tall_picture_without_animation_stays_whole() {
        let image = strip().first_frame(&json!({}));
        assert_eq!(image.height, 6);
    }
}
