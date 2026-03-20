//! Perceptual hashing (dHash) for near-duplicate image detection.

use std::path::Path;

/// Compute a 64-bit difference hash (dHash) for an image file.
///
/// The image is decoded, resized to 9x8 grayscale, and each pixel is
/// compared to its right neighbor. The result is a 64-bit hash in
/// row-major order (bit index = y * 8 + x).
///
/// Returns `None` if the image cannot be decoded (unsupported format,
/// corrupt file, or file not found).
pub fn compute_dhash(path: &Path) -> Option<u64> {
    let img = image::open(path).ok()?;
    let gray = img
        .resize_exact(9, 8, image::imageops::FilterType::Lanczos3)
        .to_luma8();

    let mut hash: u64 = 0;
    for y in 0..8 {
        for x in 0..8 {
            if gray.get_pixel(x, y)[0] > gray.get_pixel(x + 1, y)[0] {
                hash |= 1 << (y * 8 + x);
            }
        }
    }
    Some(hash)
}

/// Hamming distance between two 64-bit hashes (number of differing bits).
pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}
