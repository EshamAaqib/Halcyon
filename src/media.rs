//! Turn images and GIFs into frames for a keyboard's displays (LED matrix or LCD).

use std::io::Cursor;

use base64::Engine;
use image::codecs::gif::{GifDecoder, GifEncoder, Repeat};
use image::imageops::{self, FilterType};
use image::{
    AnimationDecoder, Delay, DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, Rgb,
    RgbImage, RgbaImage,
};

use crate::error::{Error, Result};
use crate::protocol::{Frame, LcdFrame};

const MAX_SOURCE_FRAMES: usize = 400;
/// Largest picture accepted, per side. A GIF declares its size up front, so a tiny file can otherwise ask for
/// gigabytes per frame (a decompression bomb).
const MAX_SOURCE_SIDE: u32 = 4096;
/// Total source pixels decoded from one animation; later frames are dropped, like those past MAX_SOURCE_FRAMES.
const MAX_SOURCE_PIXELS: u64 = 200_000_000;

fn unreadable(e: impl std::fmt::Display) -> Error {
    Error::Bad(format!("not an image Halcyon can read ({e})"))
}

fn limits() -> Limits {
    let mut l = Limits::default();
    l.max_image_width = Some(MAX_SOURCE_SIDE);
    l.max_image_height = Some(MAX_SOURCE_SIDE);
    l.max_alloc = Some(256 * 1024 * 1024);
    l
}

/// Decode a still image with size limits, so a hostile file cannot exhaust memory.
pub fn load_image(data: &[u8]) -> Result<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(unreadable)?;
    reader.limits(limits());
    reader.decode().map_err(unreadable)
}

/// Decode an image or animated GIF into frames of `width` x `height`, at most `max_frames` of them.
/// `fit` is "cover" (fill, cropping the edges) or "contain" (fit inside, black bars).
pub fn load_frames(
    data: &[u8],
    width: u32,
    height: u32,
    fit: &str,
    max_frames: usize,
) -> Result<Vec<LcdFrame>> {
    if width == 0 || height == 0 || width > MAX_SOURCE_SIDE || height > MAX_SOURCE_SIDE {
        return Err(Error::Internal(
            "the keyboard's definition gives this display no valid size".into(),
        ));
    }
    let mut frames = Vec::new();
    if image::guess_format(data).map_err(unreadable)? == ImageFormat::Gif {
        let mut decoder = GifDecoder::new(Cursor::new(data)).map_err(unreadable)?;
        let (sw, sh) = decoder.dimensions();
        decoder.set_limits(limits()).map_err(unreadable)?; // GifDecoder::new has no limits of its own
        let budget = (MAX_SOURCE_PIXELS / (sw as u64 * sh as u64).max(1)).max(1) as usize;
        for frame in decoder.into_frames().take(MAX_SOURCE_FRAMES.min(budget)) {
            let frame = frame.map_err(unreadable)?; // frames decode as they are reached, so a damaged file can fail here
            let (num, den) = frame.delay().numer_denom_ms();
            let ms = num.checked_div(den).unwrap_or(100);
            frames.push(LcdFrame {
                image: fit_to(frame.buffer(), width, height, fit),
                delay_ms: if ms == 0 { 100 } else { ms.max(20) },
            });
        }
    } else {
        let img = load_image(data)?;
        frames.push(LcdFrame {
            image: fit_to(&img.to_rgba8(), width, height, fit),
            delay_ms: 100,
        });
    }
    if frames.is_empty() {
        return Err(unreadable("it has no frames"));
    }
    Ok(thin(frames, max_frames))
}

/// Composite onto black, then scale to the display.
fn fit_to(src: &RgbaImage, w: u32, h: u32, fit: &str) -> RgbImage {
    let rgb = RgbImage::from_fn(src.width(), src.height(), |x, y| {
        let [r, g, b, a] = src.get_pixel(x, y).0;
        let k = |c: u8| (c as u32 * a as u32 / 255) as u8;
        Rgb([k(r), k(g), k(b)])
    });
    let (sw, sh) = (rgb.width() as f64, rgb.height() as f64);
    if fit == "contain" {
        let s = (w as f64 / sw).min(h as f64 / sh);
        let (nw, nh) = (
            ((sw * s).round() as u32).clamp(1, w),
            ((sh * s).round() as u32).clamp(1, h),
        );
        let scaled = imageops::resize(&rgb, nw, nh, FilterType::Lanczos3);
        let mut out = RgbImage::new(w, h);
        imageops::overlay(
            &mut out,
            &scaled,
            ((w - nw) / 2) as i64,
            ((h - nh) / 2) as i64,
        );
        out
    } else {
        // crop the source to the display's shape around its centre, then scale
        let target = w as f64 / h as f64;
        let (cw, ch) = if sw / sh > target {
            ((sh * target).round().max(1.0), sh)
        } else {
            (sw, (sw / target).round().max(1.0))
        };
        let (cx, cy) = (((sw - cw) / 2.0) as u32, ((sh - ch) / 2.0) as u32);
        let cropped = imageops::crop_imm(&rgb, cx, cy, cw as u32, ch as u32).to_image();
        imageops::resize(&cropped, w, h, FilterType::Lanczos3)
    }
}

/// Drop frames evenly and stretch the delays so the animation keeps its length.
pub fn thin<T: Clone + HasDelay>(frames: Vec<T>, max_frames: usize) -> Vec<T> {
    if frames.len() <= max_frames || max_frames == 0 {
        return frames;
    }
    let step = frames.len() as f64 / max_frames as f64;
    (0..max_frames)
        .map(|i| {
            let mut f = frames[(i as f64 * step) as usize].clone();
            f.scale_delay(step);
            f
        })
        .collect()
}

pub trait HasDelay {
    fn scale_delay(&mut self, by: f64);
}

impl HasDelay for LcdFrame {
    fn scale_delay(&mut self, by: f64) {
        self.delay_ms = (self.delay_ms as f64 * by) as u32;
    }
}

impl HasDelay for Frame {
    fn scale_delay(&mut self, _by: f64) {}
}

pub fn pixels(img: &RgbImage) -> Frame {
    img.pixels().map(|p| p.0).collect()
}

/// An animated GIF data URL for showing the result in the UI.
pub fn preview_gif(frames: &[LcdFrame]) -> Result<String> {
    let mut buf = Vec::new();
    {
        let mut enc = GifEncoder::new_with_speed(&mut buf, 20);
        enc.set_repeat(Repeat::Infinite)
            .map_err(|e| Error::Internal(e.to_string()))?;
        let gif_frames = frames.iter().map(|f| {
            let rgba = RgbaImage::from_fn(f.image.width(), f.image.height(), |x, y| {
                let [r, g, b] = f.image.get_pixel(x, y).0;
                image::Rgba([r, g, b, 255])
            });
            image::Frame::from_parts(rgba, 0, 0, Delay::from_numer_denom_ms(f.delay_ms, 1))
        });
        enc.encode_frames(gif_frames)
            .map_err(|e| Error::Internal(e.to_string()))?;
    }
    Ok(format!(
        "data:image/gif;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(buf)
    ))
}

/// Area-average resampling: every source pixel contributes by how much of it falls in each target pixel,
/// so thin lines stay visible when a larger matrix design is shrunk.
pub fn box_resize(src: &[[u8; 3]], sw: usize, sh: usize, dw: usize, dh: usize) -> Frame {
    let (fx, fy) = (sw as f64 / dw as f64, sh as f64 / dh as f64);
    let mut out = Vec::with_capacity(dw * dh);
    for y in 0..dh {
        let (y0, y1) = (y as f64 * fy, (y + 1) as f64 * fy);
        for x in 0..dw {
            let (x0, x1) = (x as f64 * fx, (x + 1) as f64 * fx);
            let mut acc = [0f64; 3];
            let mut area = 0f64;
            for sy in y0.floor() as usize..(y1.ceil() as usize).min(sh) {
                let wy = (y1.min((sy + 1) as f64) - y0.max(sy as f64)).max(0.0);
                for sx in x0.floor() as usize..(x1.ceil() as usize).min(sw) {
                    let w = wy * (x1.min((sx + 1) as f64) - x0.max(sx as f64)).max(0.0);
                    let p = src[sy * sw + sx];
                    for c in 0..3 {
                        acc[c] += p[c] as f64 * w;
                    }
                    area += w;
                }
            }
            out.push(acc.map(|v| {
                if area > 0.0 {
                    (v / area).round().clamp(0.0, 255.0) as u8
                } else {
                    0
                }
            }));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_fits_both_ways() {
        let mut png = Vec::new();
        RgbImage::from_pixel(100, 20, Rgb([10, 200, 30]))
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        for fit in ["cover", "contain"] {
            let f = load_frames(&png, 63, 5, fit, 260).unwrap();
            assert_eq!((f.len(), f[0].image.dimensions()), (1, (63, 5)));
        }
        assert!(load_frames(b"not an image", 63, 5, "cover", 10).is_err());
    }

    #[test]
    fn huge_gif_canvas_is_refused_not_allocated() {
        // a 30-byte GIF that declares a 65535 x 65535 canvas: ~17 GB per decoded frame without limits
        let mut gif = b"GIF89a\xff\xff\xff\xff\x80\x00\x00".to_vec();
        gif.extend_from_slice(&[0, 0, 0, 255, 255, 255]); // two-colour palette
        gif.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0]); // one 1x1 frame
        gif.extend_from_slice(&[2, 2, 0x44, 0x01, 0, 0x3B]);
        assert!(load_frames(&gif, 63, 5, "cover", 10).is_err());
        assert!(load_frames(&gif, 0, 5, "contain", 10).is_err()); // a definition without a size
    }

    #[test]
    fn thin_keeps_length() {
        let frames: Vec<LcdFrame> = (0..10)
            .map(|_| LcdFrame {
                image: RgbImage::new(1, 1),
                delay_ms: 100,
            })
            .collect();
        let t = thin(frames, 5);
        assert_eq!(t.len(), 5);
        assert_eq!(t[0].delay_ms, 200);
    }

    #[test]
    fn box_resize_averages() {
        let src = vec![[255, 0, 0], [0, 0, 255]];
        assert_eq!(box_resize(&src, 2, 1, 1, 1), vec![[128, 0, 128]]);
    }
}
