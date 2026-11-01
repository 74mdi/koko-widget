//! Background artwork fetching, decoding and bounded on-disk caching.

use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result, bail};
use image::{ImageReader, imageops::FilterType};
use sha2::{Digest, Sha256};

use crate::utils::{ArtworkSource, artwork_source, cache_dir, write_atomic};

pub const MAX_REMOTE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_LOCAL_BYTES: u64 = 48 * 1024 * 1024;
pub const MAX_SOURCE_DIMENSION: u32 = 8192;
pub const MAX_CACHED_DIMENSION: u32 = 1024;
pub const MAX_CACHE_FILES: usize = 128;

#[derive(Debug, Clone)]
pub struct DecodedArtwork {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Restrained accent sampled from vivid, non-extreme pixels.
    pub accent: [f32; 3],
    pub from_cache: bool,
}

#[derive(Debug, Clone)]
pub struct ArtworkRequest {
    pub id: u64,
    pub url: String,
    pub allow_remote: bool,
}

#[derive(Debug)]
pub enum ArtworkResponse {
    Loaded { id: u64, artwork: DecodedArtwork },
    Failed { id: u64, error: String },
}

pub type ArtworkCallback = Arc<dyn Fn(ArtworkResponse) + Send + Sync + 'static>;

/// A single-threaded loader. Requests are serialised through one worker, which
/// keeps memory bounded and avoids duplicate downloads for the same cover.
pub struct ArtworkLoader {
    request_tx: std::sync::mpsc::Sender<ArtworkRequest>,
    next_id: Arc<AtomicU64>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl ArtworkLoader {
    pub fn spawn(callback: ArtworkCallback) -> Self {
        let (request_tx, request_rx) = std::sync::mpsc::channel::<ArtworkRequest>();
        let next_id = Arc::new(AtomicU64::new(1));
        let worker = std::thread::Builder::new()
            .name("koko-artwork".to_owned())
            .spawn(move || artwork_worker(request_rx, callback))
            .expect("could not start artwork loader thread");
        Self {
            request_tx,
            next_id,
            worker: Some(worker),
        }
    }

    pub fn request(&self, url: &str, allow_remote: bool) -> Option<u64> {
        if url.trim().is_empty() {
            return None;
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = ArtworkRequest {
            id,
            url: url.to_owned(),
            allow_remote,
        };
        if self.request_tx.send(request).is_err() {
            tracing::warn!("artwork worker is no longer available");
            return None;
        }
        Some(id)
    }

    pub fn next_id(&self) -> u64 {
        self.next_id.load(Ordering::Relaxed)
    }
}

impl Drop for ArtworkLoader {
    fn drop(&mut self) {
        // Dropping the sender makes the worker exit after its in-flight request.
        // Joining without a timeout could delay shutdown on a slow network, so
        // the process simply lets the short-lived worker finish.
        if let Some(worker) = self.worker.take()
            && worker.is_finished()
        {
            let _ = worker.join();
        }
    }
}

fn artwork_worker(requests: std::sync::mpsc::Receiver<ArtworkRequest>, callback: ArtworkCallback) {
    let cache_root = cache_dir().join("artwork");
    if let Err(error) = prune_cache(&cache_root, MAX_CACHE_FILES) {
        tracing::debug!(%error, "artwork cache pruning skipped");
    }
    while let Ok(request) = requests.recv() {
        let response = match load_artwork(&request, &cache_root) {
            Ok(artwork) => ArtworkResponse::Loaded {
                id: request.id,
                artwork,
            },
            Err(error) => ArtworkResponse::Failed {
                id: request.id,
                error: format!("{error:#}"),
            },
        };
        callback(response);
    }
}

fn load_artwork(request: &ArtworkRequest, cache_root: &Path) -> Result<DecodedArtwork> {
    let source = artwork_source(&request.url)?;
    match source {
        ArtworkSource::Local(path) => {
            let metadata = std::fs::metadata(&path)
                .with_context(|| format!("could not stat {}", path.display()))?;
            if metadata.len() > MAX_LOCAL_BYTES {
                bail!("local artwork is larger than the safety limit");
            }
            let bytes = std::fs::read(&path)
                .with_context(|| format!("could not read {}", path.display()))?;
            decode_artwork(&bytes)
        }
        ArtworkSource::Remote(url) => {
            if !request.allow_remote {
                bail!("remote artwork is disabled");
            }
            let cache_path = remote_cache_path(cache_root, &url);
            if let Ok(bytes) = std::fs::read(&cache_path)
                && bytes.len() as u64 <= MAX_REMOTE_BYTES
            {
                match decode_artwork(&bytes) {
                    Ok(mut artwork) => {
                        artwork.from_cache = true;
                        return Ok(artwork);
                    }
                    Err(error) => {
                        tracing::debug!(%error, "cached artwork was invalid; refreshing");
                        let _ = std::fs::remove_file(&cache_path);
                    }
                }
            }

            let bytes = download(&url)?;
            let artwork = decode_artwork(&bytes)?;
            if let Err(error) = write_atomic(&cache_path, &bytes) {
                tracing::debug!(%error, "could not cache remote artwork");
            }
            Ok(artwork)
        }
    }
}

fn download(url: &str) -> Result<Vec<u8>> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .header(
            "User-Agent",
            concat!("KOKO-WIDGET/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .with_context(|| format!("artwork request failed for {url}"))?;
    if !response.status().is_success() {
        bail!("artwork server returned {}", response.status());
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_REMOTE_BYTES)
        .read_to_vec()
        .context("could not read remote artwork")?;
    if bytes.is_empty() {
        bail!("remote artwork was empty");
    }
    Ok(bytes)
}

pub fn decode_artwork(bytes: &[u8]) -> Result<DecodedArtwork> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_DIMENSION);
    limits.max_image_height = Some(MAX_SOURCE_DIMENSION);
    limits.max_alloc = Some(128 * 1024 * 1024);

    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("could not identify artwork format")?;
    let mut reader = reader;
    reader.limits(limits);
    let image = reader.decode().context("could not decode artwork")?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    if width == 0 || height == 0 {
        bail!("artwork has zero dimensions");
    }

    let (width, height, rgba, accent) = if width > MAX_CACHED_DIMENSION
        || height > MAX_CACHED_DIMENSION
    {
        let scale = (MAX_CACHED_DIMENSION as f64 / f64::from(width))
            .min(MAX_CACHED_DIMENSION as f64 / f64::from(height));
        let new_width = (f64::from(width) * scale).round().max(1.0) as u32;
        let new_height = (f64::from(height) * scale).round().max(1.0) as u32;
        let resized = image::imageops::resize(&rgba, new_width, new_height, FilterType::Lanczos3);
        let accent = dominant_accent(resized.as_raw());
        (new_width, new_height, resized.into_raw(), accent)
    } else {
        let accent = dominant_accent(rgba.as_raw());
        (width, height, rgba.into_raw(), accent)
    };

    Ok(DecodedArtwork {
        width,
        height,
        rgba,
        accent,
        from_cache: false,
    })
}

fn remote_cache_path(cache_root: &Path, url: &str) -> PathBuf {
    let digest = Sha256::digest(url.as_bytes());
    let hash = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    cache_root.join(format!("{hash}.art"))
}

fn prune_cache(cache_root: &Path, max_files: usize) -> Result<()> {
    if !cache_root.exists() {
        return Ok(());
    }
    let mut files = std::fs::read_dir(cache_root)
        .with_context(|| format!("could not read {}", cache_root.display()))?
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            Some((
                entry.path(),
                metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ))
        })
        .collect::<Vec<_>>();
    if files.len() <= max_files {
        return Ok(());
    }
    files.sort_by_key(|(_, modified)| *modified);
    let remove_count = files.len() - max_files;
    for (path, _) in files.into_iter().take(remove_count) {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

fn dominant_accent(rgba: &[u8]) -> [f32; 3] {
    let mut weight_sum = 0.0_f64;
    let mut sums = [0.0_f64; 3];
    // Sampling every 7th pixel is enough for a tint and keeps the worker cheap.
    for pixel in rgba.as_chunks::<4>().0.iter().step_by(7) {
        let alpha = f64::from(pixel[3]) / 255.0;
        if alpha < 0.2 {
            continue;
        }
        let rgb = [
            f64::from(pixel[0]) / 255.0,
            f64::from(pixel[1]) / 255.0,
            f64::from(pixel[2]) / 255.0,
        ];
        let max = rgb[0].max(rgb[1]).max(rgb[2]);
        let min = rgb[0].min(rgb[1]).min(rgb[2]);
        let luminance = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
        let saturation = if max > 0.0 { (max - min) / max } else { 0.0 };
        if luminance < 0.04 || luminance > 0.94 {
            continue;
        }
        // Vivid mid-tones dominate; near-greyscale pixels still contribute a
        // little so monochrome covers yield a tasteful neutral tint.
        let weight = (0.18 + saturation * 1.6) * (0.55 + (1.0 - (luminance - 0.5).abs()));
        weight_sum += weight;
        for channel in 0..3 {
            sums[channel] += rgb[channel] * weight;
        }
    }
    if weight_sum <= f64::EPSILON {
        return [0.55, 0.49, 0.96];
    }
    let average = [
        (sums[0] / weight_sum) as f32,
        (sums[1] / weight_sum) as f32,
        (sums[2] / weight_sum) as f32,
    ];
    boost_saturation(average)
}

fn boost_saturation(accent: [f32; 3]) -> [f32; 3] {
    let luminance = 0.2126 * accent[0] + 0.7152 * accent[1] + 0.0722 * accent[2];
    let boosted = accent.map(|channel| luminance + (channel - luminance) * 1.22);
    // Keep the tint usable as a subtle highlight. Extremely bright artwork
    // should not turn the glass white.
    let max = boosted[0].max(boosted[1]).max(boosted[2]).max(0.001);
    if max > 0.82 {
        let scale = 0.82 / max;
        boosted.map(|channel| (channel * scale).clamp(0.0, 1.0))
    } else {
        boosted.map(|channel| channel.clamp(0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let image = ImageBuffer::from_fn(width, height, |x, y| {
            Rgba([(x % 256) as u8, (y % 256) as u8, 180, 255])
        });
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn decodes_png_into_rgba() {
        let bytes = png_bytes(64, 32);
        let artwork = decode_artwork(&bytes).unwrap();
        assert_eq!(artwork.width, 64);
        assert_eq!(artwork.height, 32);
        assert_eq!(artwork.rgba.len(), 64 * 32 * 4);
        assert!(
            artwork
                .accent
                .iter()
                .all(|value| (0.0..=1.0).contains(value))
        );
    }

    #[test]
    fn resizes_large_artwork_to_bounded_dimensions() {
        let bytes = png_bytes(1400, 700);
        let artwork = decode_artwork(&bytes).unwrap();
        assert!(artwork.width <= MAX_CACHED_DIMENSION);
        assert!(artwork.height <= MAX_CACHED_DIMENSION);
        assert_eq!(
            artwork.rgba.len() as u32,
            artwork.width * artwork.height * 4
        );
    }

    #[test]
    fn rejects_non_image_data() {
        assert!(decode_artwork(b"not an image").is_err());
        assert!(decode_artwork(&[]).is_err());
    }

    #[test]
    fn cache_key_is_stable_and_url_specific() {
        let root = Path::new("/tmp/cache");
        let one = remote_cache_path(root, "https://example.invalid/a.jpg");
        let two = remote_cache_path(root, "https://example.invalid/a.jpg");
        let three = remote_cache_path(root, "https://example.invalid/b.jpg");
        assert_eq!(one, two);
        assert_ne!(one, three);
        assert!(one.to_string_lossy().ends_with(".art"));
    }

    #[test]
    fn cache_pruning_respects_limit() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..10 {
            std::fs::write(dir.path().join(format!("{index}.art")), b"x").unwrap();
        }
        prune_cache(dir.path(), 3).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 3);
    }

    #[test]
    fn saturation_boost_keeps_values_in_range() {
        for colour in [
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [1.0, 0.0, 0.0],
            [0.2, 0.7, 0.3],
        ] {
            let value = boost_saturation(colour);
            assert!(value.iter().all(|channel| (0.0..=1.0).contains(channel)));
        }
    }
}
