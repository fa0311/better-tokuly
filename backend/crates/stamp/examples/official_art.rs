//! Renders the official stamp art to PNG files, to look at while drawing.
//!
//! ```sh
//! cargo run -p tokuly-stamp --example official_art -- /tmp/stamp-art
//! ```
//!
//! Writes one `<pack>-<stamp>.png` per stamp plus `<pack>.png`, a contact
//! sheet of the whole pack on a grey background (so white parts show).
use std::path::PathBuf;

use image::{Rgba, RgbaImage, imageops};
use tokuly_stamp::official::art;

const STAMP_SIZE: u32 = 256;
const COLUMNS: u32 = 4;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(std::env::args().nth(1).ok_or("usage: official_art <output directory>")?);
    std::fs::create_dir_all(&dir)?;
    for pack in art::packs() {
        let rows = (pack.stamps.len() as u32).div_ceil(COLUMNS);
        let mut sheet = RgbaImage::from_pixel(COLUMNS * STAMP_SIZE, rows * STAMP_SIZE, Rgba([128, 128, 128, 255]));
        for (index, stamp) in pack.stamps.iter().enumerate() {
            let pixels = stamp.scene.render(STAMP_SIZE);
            let picture = RgbaImage::from_raw(STAMP_SIZE, STAMP_SIZE, pixels).ok_or("render size mismatch")?;
            picture.save(dir.join(format!("{}-{}.png", pack.key, stamp.key)))?;
            let (column, row) = (index as u32 % COLUMNS, index as u32 / COLUMNS);
            imageops::overlay(&mut sheet, &picture, (column * STAMP_SIZE) as i64, (row * STAMP_SIZE) as i64);
        }
        let path = dir.join(format!("{}.png", pack.key));
        sheet.save(&path)?;
        println!("{} ({} stamps): {}", pack.name, pack.stamps.len(), path.display());
    }
    Ok(())
}
