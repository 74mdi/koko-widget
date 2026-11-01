//! Album-artwork representation for the Cairo renderer.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use cairo::{Format, ImageSurface};

use crate::media::artwork::DecodedArtwork;

#[derive(Clone)]
pub struct ArtworkLayer {
    pub surface: Arc<ImageSurface>,
    pub accent: [f32; 3],
}

#[derive(Default)]
pub struct ArtworkVisual {
    pub current: Option<ArtworkLayer>,
    pub previous: Option<ArtworkLayer>,
    pub transition_started: Option<Instant>,
    pub transition_duration: Duration,
    pub url: Option<String>,
    pub failed: bool,
}

impl ArtworkVisual {
    pub fn new() -> Self {
        Self {
            transition_duration: Duration::from_millis(220),
            ..Self::default()
        }
    }

    pub fn update(&mut self, decoded: &DecodedArtwork, url: &str, now: Instant) {
        match layer_from_decoded(decoded) {
            Ok(layer) => {
                self.previous = self.current.take();
                self.current = Some(layer);
                self.url = Some(url.to_owned());
                self.failed = false;
                self.transition_started = Some(now);
            }
            Err(error) => {
                tracing::warn!(%error, "could not upload artwork to Cairo");
                self.show_fallback(now);
            }
        }
    }

    pub fn show_fallback(&mut self, now: Instant) {
        if self.current.is_some() {
            self.previous = self.current.take();
            self.transition_started = Some(now);
        } else {
            self.previous = None;
            self.transition_started = None;
        }
        self.url = None;
        self.failed = false;
    }

    pub fn invalidate_transition_if_complete(&mut self, now: Instant) {
        if let Some(started) = self.transition_started
            && now.saturating_duration_since(started) >= self.transition_duration
        {
            self.previous = None;
            self.transition_started = None;
        }
    }

    pub fn transition_progress(&self, now: Instant) -> f32 {
        let Some(started) = self.transition_started else {
            return 1.0;
        };
        if self.transition_duration.is_zero() {
            return 1.0;
        }
        let elapsed = now.saturating_duration_since(started).as_secs_f32();
        (elapsed / self.transition_duration.as_secs_f32()).clamp(0.0, 1.0)
    }

    pub fn accent(&self) -> Option<[f32; 3]> {
        self.current.as_ref().map(|layer| layer.accent)
    }

    pub fn has_pending_transition(&self) -> bool {
        self.transition_started.is_some()
    }
}

fn layer_from_decoded(decoded: &DecodedArtwork) -> cairo::Result<ArtworkLayer> {
    let width = decoded.width.max(1);
    let height = decoded.height.max(1);
    let mut surface = ImageSurface::create(Format::ARgb32, width as i32, height as i32)?;
    {
        let stride = surface.stride() as usize;
        let mut data = surface.data().map_err(|_| cairo::Error::InvalidStatus)?;
        for row in 0..height as usize {
            let source_row = row * width as usize * 4;
            let dest_row = row * stride;
            for column in 0..width as usize {
                let s = source_row + column * 4;
                let d = dest_row + column * 4;
                let alpha = decoded.rgba[s + 3];
                let premultiply = |channel: u8| -> u8 {
                    if alpha == 255 {
                        channel
                    } else {
                        ((u16::from(channel) * u16::from(alpha)) / 255) as u8
                    }
                };
                // Cairo's ARGB32 is premultiplied and stored native-endian.
                data[d] = premultiply(decoded.rgba[s + 2]);
                data[d + 1] = premultiply(decoded.rgba[s + 1]);
                data[d + 2] = premultiply(decoded.rgba[s]);
                data[d + 3] = alpha;
            }
        }
    }
    surface.mark_dirty();
    surface.flush();
    Ok(ArtworkLayer {
        surface: Arc::new(surface),
        accent: decoded.accent,
    })
}
