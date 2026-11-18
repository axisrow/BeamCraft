// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The picture of a model in the model library: `thumbnail.png` in the model folder. The window
//! saves it when a model is left (closed, switched or returned to the library) from the active
//! view as it was last seen, so the library shows the model without reading and projecting its
//! geometry each time.
//!
//! A picture BeamCraft saved carries a PNG text chunk ([`MARKER_KEY`]). A `thumbnail.png`
//! without that chunk was put there by a person, to tell models apart at a glance: it is shown
//! as it is and never replaced.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

/// The file name of the picture inside a model folder.
pub const FILE_NAME: &str = "thumbnail.png";

/// The PNG text keyword that marks a picture BeamCraft saved itself.
pub const MARKER_KEY: &str = "BeamCraft";

/// The value of the [`MARKER_KEY`] text chunk.
pub const MARKER_VALUE: &str = "model-thumbnail";

/// The largest saved picture, in pixels; a bigger view is scaled down to fit.
pub const MAX_SIZE: [u32; 2] = [536, 368];

/// The largest picture read back, in pixels per side; a person's own larger file is scaled
/// down to fit before it reaches the window.
const MAX_READ_SIDE: u32 = 1024;

/// The picture of the model at `store`.
pub fn path(store: &Path) -> PathBuf {
    store.join(FILE_NAME)
}

/// Whether `file` is a picture BeamCraft saved: a PNG with the marker text chunk.
pub fn is_generated(file: &Path) -> bool {
    let Ok(handle) = std::fs::File::open(file) else {
        return false;
    };
    let decoder = png::Decoder::new(std::io::BufReader::new(handle));
    let Ok(reader) = decoder.read_info() else {
        return false;
    };
    let info = reader.info();
    info.uncompressed_latin1_text
        .iter()
        .any(|chunk| chunk.keyword == MARKER_KEY && chunk.text == MARKER_VALUE)
}

/// Cuts `rect` (in pixels, clamped to the image) out of `image` and scales it down to fit
/// [`MAX_SIZE`], keeping its proportions. `None` for an empty cut.
pub fn crop(image: &egui::ColorImage, rect: [usize; 4]) -> Option<image::RgbaImage> {
    let [width, height] = image.size;
    let [x0, y0, x1, y1] = rect;
    let (x0, x1) = (x0.min(width), x1.min(width));
    let (y0, y1) = (y0.min(height), y1.min(height));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (w, h) = (x1 - x0, y1 - y0);
    let mut bytes = Vec::with_capacity(w.checked_mul(h)?.checked_mul(4)?);
    for y in y0..y1 {
        let row = image.pixels.get(y * width + x0..y * width + x1)?;
        for pixel in row {
            bytes.extend_from_slice(&pixel.to_srgba_unmultiplied());
        }
    }
    let cut = image::RgbaImage::from_raw(u32::try_from(w).ok()?, u32::try_from(h).ok()?, bytes)?;
    let scale = (MAX_SIZE[0] as f32 / w as f32)
        .min(MAX_SIZE[1] as f32 / h as f32)
        .min(1.0);
    if scale >= 1.0 {
        return Some(cut);
    }
    let fit = |side: usize| ((side as f32 * scale).round() as u32).max(1);
    Some(image::imageops::resize(
        &cut,
        fit(w),
        fit(h),
        image::imageops::FilterType::Triangle,
    ))
}

/// What became of a save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Saved {
    /// The picture was written.
    Written,
    /// A person's own `thumbnail.png` is there; it was left as it is.
    KeptOwn,
}

/// Saves `picture` as the model's `thumbnail.png`, marked as BeamCraft's own. A person's own
/// file is never replaced. The picture is written next to the target and renamed over it, so a
/// failed write never leaves a broken file behind.
pub fn save(store: &Path, picture: &image::RgbaImage) -> Result<Saved, String> {
    if !store.is_dir() {
        return Err("the model folder does not exist".into());
    }
    let target = path(store);
    if target.exists() && !is_generated(&target) {
        return Ok(Saved::KeptOwn);
    }
    let partial = store.join(format!(".{FILE_NAME}.partial"));
    let result = write_png(&partial, picture).and_then(|()| {
        std::fs::rename(&partial, &target)
            .map_err(|e| format!("the model picture could not be saved: {e}"))
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result.map(|()| Saved::Written)
}

fn write_png(file: &Path, picture: &image::RgbaImage) -> Result<(), String> {
    let fail = |e: &dyn std::fmt::Display| format!("the model picture could not be saved: {e}");
    let handle = std::fs::File::create(file).map_err(|e| fail(&e))?;
    let mut out = BufWriter::new(handle);
    let mut encoder = png::Encoder::new(&mut out, picture.width(), picture.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .add_text_chunk(MARKER_KEY.into(), MARKER_VALUE.into())
        .map_err(|e| fail(&e))?;
    let mut writer = encoder.write_header().map_err(|e| fail(&e))?;
    writer
        .write_image_data(picture.as_raw())
        .map_err(|e| fail(&e))?;
    writer.finish().map_err(|e| fail(&e))?;
    out.flush().map_err(|e| fail(&e))
}

/// Reads the model's `thumbnail.png`, BeamCraft's or a person's own, for the library. `None`
/// when there is none; an error when one is there but cannot be read as a picture.
pub fn load(store: &Path) -> Option<Result<egui::ColorImage, String>> {
    let file = path(store);
    if !file.is_file() {
        return None;
    }
    Some(read(&file))
}

fn read(file: &Path) -> Result<egui::ColorImage, String> {
    let picture = image::ImageReader::open(file)
        .map_err(|e| format!("thumbnail.png could not be read: {e}"))?
        .with_guessed_format()
        .map_err(|e| format!("thumbnail.png could not be read: {e}"))?
        .decode()
        .map_err(|e| format!("thumbnail.png is not a readable picture: {e}"))?;
    let picture = if picture.width() > MAX_READ_SIDE || picture.height() > MAX_READ_SIDE {
        picture.thumbnail(MAX_READ_SIDE, MAX_READ_SIDE)
    } else {
        picture
    };
    let rgba = picture.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_raw(),
    ))
}

/// The part of `rect` a picture of `size` pixels fills, centred: the picture
/// keeps its proportions and is letterboxed, never stretched.
pub fn fit(rect: egui::Rect, size: [usize; 2]) -> egui::Rect {
    let [w, h] = size;
    if w == 0 || h == 0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return rect;
    }
    let scale = (rect.width() / w as f32).min(rect.height() / h as f32);
    egui::Rect::from_center_size(rect.center(), egui::vec2(w as f32, h as f32) * scale)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "bc-thumbnail-{name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn shot(size: [usize; 2]) -> egui::ColorImage {
        let mut image = egui::ColorImage::new(size, vec![egui::Color32::BLACK; size[0] * size[1]]);
        image.pixels[size[0] + 1] = egui::Color32::from_rgb(200, 10, 20);
        image
    }

    #[test]
    fn a_saved_picture_is_marked_and_read_back() {
        let dir = scratch("saved");
        let picture = crop(&shot([8, 6]), [1, 1, 5, 4]).unwrap();
        assert_eq!(picture.dimensions(), (4, 3));
        assert_eq!(picture.get_pixel(0, 0).0, [200, 10, 20, 255]);
        assert_eq!(save(&dir, &picture), Ok(Saved::Written));
        assert!(is_generated(&path(&dir)));
        let back = load(&dir).unwrap().unwrap();
        assert_eq!(back.size, [4, 3]);
        // A later save replaces BeamCraft's own picture.
        let wider = crop(&shot([8, 6]), [0, 0, 8, 6]).unwrap();
        assert_eq!(save(&dir, &wider), Ok(Saved::Written));
        assert_eq!(load(&dir).unwrap().unwrap().size, [8, 6]);
        assert!(!dir.join(".thumbnail.png.partial").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_persons_own_picture_is_shown_and_never_replaced() {
        let dir = scratch("own");
        let own = image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255]));
        own.save(path(&dir)).unwrap();
        let before = std::fs::read(path(&dir)).unwrap();
        assert!(!is_generated(&path(&dir)));
        let picture = crop(&shot([8, 6]), [0, 0, 8, 6]).unwrap();
        assert_eq!(save(&dir, &picture), Ok(Saved::KeptOwn));
        assert_eq!(std::fs::read(path(&dir)).unwrap(), before);
        assert_eq!(load(&dir).unwrap().unwrap().size, [3, 2]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_picture_and_a_broken_picture_are_told_apart() {
        let dir = scratch("broken");
        assert!(load(&dir).is_none());
        std::fs::write(path(&dir), b"not a png").unwrap();
        assert!(load(&dir).unwrap().is_err());
        // A broken file is not BeamCraft's, so it is not replaced either.
        assert!(!is_generated(&path(&dir)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_large_view_is_scaled_to_fit_and_an_empty_cut_is_refused() {
        let picture = crop(&shot([1600, 900]), [0, 0, 1600, 900]).unwrap();
        assert!(picture.width() <= MAX_SIZE[0] && picture.height() <= MAX_SIZE[1]);
        assert_eq!(picture.width(), MAX_SIZE[0]);
        assert!(crop(&shot([8, 6]), [5, 5, 5, 6]).is_none());
        assert!(crop(&shot([8, 6]), [20, 20, 30, 30]).is_none());
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 100.0));
        let fitted = fit(rect, [100, 100]);
        assert_eq!(fitted.size(), egui::vec2(100.0, 100.0));
        assert_eq!(fitted.center(), rect.center());
    }
}
