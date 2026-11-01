//! Layered Liquid Glass-inspired rendering.
//!
//! This is intentionally compositor-agnostic. Real background blur is provided
//! by the compositor when available (for example a niri layer rule); everything
//! here is a legible fallback made from translucent gradients, highlights and a
//! restrained grain.

use std::cell::RefCell;

use cairo::{Context, Filter, Format, ImageSurface, RectangleInt, SurfacePattern};

use crate::{
    config::{Appearance, Theme},
    ui::layout::Rect,
    utils::parse_hex_color,
};

const COMPACT_RADIUS: f64 = 34.0;

#[derive(Debug, Clone, Copy)]
pub struct Rgb {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

impl Rgb {
    pub const WHITE: Self = Self {
        r: 1.0,
        g: 1.0,
        b: 1.0,
    };

    pub fn from_slice(value: [f32; 3]) -> Self {
        Self {
            r: f64::from(value[0]).clamp(0.0, 1.0),
            g: f64::from(value[1]).clamp(0.0, 1.0),
            b: f64::from(value[2]).clamp(0.0, 1.0),
        }
    }

    pub fn hex(value: &str) -> Option<Self> {
        let [r, g, b, _] = parse_hex_color(value)?;
        Some(Self { r, g, b })
    }

    pub fn mix(self, other: Self, amount: f64) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        Self {
            r: self.r + (other.r - self.r) * amount,
            g: self.g + (other.g - self.g) * amount,
            b: self.b + (other.b - self.b) * amount,
        }
    }

    pub fn lighten(self, amount: f64) -> Self {
        self.mix(Self::WHITE, amount)
    }

    pub fn darken(self, amount: f64) -> Self {
        self.mix(
            Self {
                r: 0.0,
                g: 0.0,
                b: 0.0,
            },
            amount,
        )
    }

    pub fn luminance(self) -> f64 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub base_top: Rgb,
    pub base_bottom: Rgb,
    pub accent: Rgb,
    pub text: Rgb,
    pub secondary: Rgb,
    pub tertiary: Rgb,
    pub border: Rgb,
    pub progress: Rgb,
    pub track: Rgb,
    pub shadow: Rgb,
    pub base_alpha: f64,
}

impl Palette {
    pub fn derive(
        appearance: &Appearance,
        artwork_accent: Option<[f32; 3]>,
        is_dark: bool,
    ) -> Self {
        let configured = Rgb::hex(&appearance.accent).unwrap_or(Rgb {
            r: 0.545,
            g: 0.486,
            b: 0.965,
        });
        let artwork = artwork_accent.map(Rgb::from_slice);
        let raw_accent = if appearance.adaptive_accent {
            artwork.unwrap_or(configured)
        } else {
            configured
        };
        // Keep the tint from becoming fluorescent or nearly black.
        let accent = if raw_accent.luminance() < 0.12 {
            raw_accent.lighten(0.28)
        } else if raw_accent.luminance() > 0.82 {
            raw_accent.darken(0.24)
        } else {
            raw_accent
        };

        let base_alpha = if !appearance.glass {
            0.985
        } else if appearance.blur {
            (0.53 + appearance.opacity * 0.44).clamp(0.62, 0.955)
        } else {
            (0.62 + appearance.opacity * 0.40).clamp(0.70, 0.975)
        };

        if is_dark {
            let base = Rgb {
                r: 0.075,
                g: 0.073,
                b: 0.105,
            }
            .mix(accent, 0.085);
            Self {
                dark: true,
                base_top: base.lighten(0.05),
                base_bottom: base.darken(0.34),
                accent,
                text: Rgb {
                    r: 0.985,
                    g: 0.985,
                    b: 1.0,
                },
                secondary: Rgb {
                    r: 0.80,
                    g: 0.79,
                    b: 0.86,
                },
                tertiary: Rgb {
                    r: 0.70,
                    g: 0.69,
                    b: 0.76,
                },
                border: Rgb::WHITE,
                progress: Rgb {
                    r: 0.96,
                    g: 0.95,
                    b: 1.0,
                },
                track: Rgb::WHITE,
                shadow: Rgb {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                },
                base_alpha,
            }
        } else {
            let base = Rgb {
                r: 0.91,
                g: 0.905,
                b: 0.94,
            }
            .mix(accent, 0.05);
            Self {
                dark: false,
                base_top: base.lighten(0.34),
                base_bottom: base.darken(0.10),
                accent,
                text: Rgb {
                    r: 0.075,
                    g: 0.068,
                    b: 0.105,
                },
                secondary: Rgb {
                    r: 0.24,
                    g: 0.23,
                    b: 0.30,
                },
                tertiary: Rgb {
                    r: 0.35,
                    g: 0.34,
                    b: 0.41,
                },
                border: Rgb {
                    r: 0.1,
                    g: 0.08,
                    b: 0.16,
                },
                progress: Rgb {
                    r: 0.16,
                    g: 0.14,
                    b: 0.23,
                },
                track: Rgb {
                    r: 0.14,
                    g: 0.12,
                    b: 0.20,
                },
                shadow: Rgb {
                    r: 0.05,
                    g: 0.03,
                    b: 0.09,
                },
                base_alpha: base_alpha * 0.92,
            }
        }
    }

    pub fn radius(self, configured: f64, expanded: bool) -> f64 {
        let _ = self;
        configured.clamp(18.0, 48.0)
            + if expanded {
                0.0
            } else {
                (COMPACT_RADIUS - 34.0).min(1.0)
            }
    }
}

pub fn is_dark_theme(theme: Theme, system_dark: bool) -> bool {
    match theme {
        Theme::Dark => true,
        Theme::Light => false,
        Theme::Auto => system_dark,
    }
}

pub fn rounded_rect(cr: &Context, rect: Rect, radius: f64) {
    let radius = radius.max(0.0).min(rect.width.min(rect.height) * 0.5);
    let x = rect.x;
    let y = rect.y;
    let w = rect.width;
    let h = rect.height;
    let right = x + w;
    let bottom = y + h;
    let k = 0.552_284_749_8;

    cr.new_sub_path();
    cr.arc(
        right - radius,
        y + radius,
        radius,
        -std::f64::consts::FRAC_PI_2,
        0.0,
    );
    cr.arc(
        right - radius,
        bottom - radius,
        radius,
        0.0,
        std::f64::consts::FRAC_PI_2,
    );
    cr.arc(
        x + radius,
        bottom - radius,
        radius,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    cr.arc(
        x + radius,
        y + radius,
        radius,
        std::f64::consts::PI,
        3.0 * std::f64::consts::FRAC_PI_2,
    );
    cr.close_path();
    let _ = k;
}

pub fn paint_shadow(cr: &Context, pill: Rect, radius: f64, palette: Palette) {
    let layers = 9;
    for index in (0..layers).rev() {
        let spread = 1.0 + index as f64 * 1.65;
        let y_offset = 5.0 + index as f64 * 0.30;
        let alpha = if palette.dark { 0.028 } else { 0.016 };
        cr.set_source_rgba(palette.shadow.r, palette.shadow.g, palette.shadow.b, alpha);
        let shadow = Rect::new(
            pill.x - spread,
            pill.y - spread + y_offset,
            pill.width + spread * 2.0,
            pill.height + spread * 2.0,
        );
        rounded_rect(cr, shadow, radius + spread * 0.6);
        let _ = cr.fill();
    }
}

pub fn paint_glass(cr: &Context, pill: Rect, radius: f64, palette: Palette, frosted: bool) {
    let _ = cr.save();
    rounded_rect(cr, pill, radius);
    cr.clip();

    // A deep, softly graded base.
    let base =
        cairo::LinearGradient::new(pill.x, pill.y, pill.x + pill.width * 0.25, pill.bottom());
    let top_alpha = if frosted {
        palette.base_alpha * 0.94
    } else {
        palette.base_alpha
    };
    base.add_color_stop_rgba(
        0.0,
        palette.base_top.r,
        palette.base_top.g,
        palette.base_top.b,
        top_alpha,
    );
    base.add_color_stop_rgba(
        0.58,
        palette.base_top.mix(palette.base_bottom, 0.35).r,
        palette.base_top.mix(palette.base_bottom, 0.35).g,
        palette.base_top.mix(palette.base_bottom, 0.35).b,
        palette.base_alpha,
    );
    base.add_color_stop_rgba(
        1.0,
        palette.base_bottom.r,
        palette.base_bottom.g,
        palette.base_bottom.b,
        (palette.base_alpha + 0.035).min(0.98),
    );
    set_linear(cr, &base);
    cr.paint().ok();

    // Accent wash: strongest behind the artwork and along the upper-right edge,
    // never enough to tint the text or controls.
    let wash = cairo::RadialGradient::new(
        pill.x + pill.width * 0.30,
        pill.y + pill.height * 0.14,
        pill.width * 0.02,
        pill.x + pill.width * 0.46,
        pill.y + pill.height * 0.44,
        pill.width * 0.66,
    );
    wash.add_color_stop_rgba(
        0.0,
        palette.accent.r,
        palette.accent.g,
        palette.accent.b,
        if palette.dark { 0.26 } else { 0.16 },
    );
    wash.add_color_stop_rgba(
        0.55,
        palette.accent.r,
        palette.accent.g,
        palette.accent.b,
        if palette.dark { 0.085 } else { 0.055 },
    );
    wash.add_color_stop_rgba(
        1.0,
        palette.accent.r,
        palette.accent.g,
        palette.accent.b,
        0.0,
    );
    set_radial(cr, &wash);
    cr.paint().ok();

    // A second, cooler reflection on the right keeps the glass dimensional.
    let reflection = cairo::LinearGradient::new(pill.right(), pill.y, pill.x, pill.bottom());
    reflection.add_color_stop_rgba(
        0.0,
        palette.accent.r,
        palette.accent.g,
        palette.accent.b,
        if palette.dark { 0.115 } else { 0.06 },
    );
    reflection.add_color_stop_rgba(
        0.42,
        palette.accent.r * 0.7,
        palette.accent.g * 0.7,
        palette.accent.b,
        0.018,
    );
    reflection.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 0.0);
    set_linear(cr, &reflection);
    cr.paint().ok();

    // Top specular strip.
    let highlight = cairo::LinearGradient::new(0.0, pill.y, 0.0, pill.y + pill.height * 0.52);
    highlight.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, if palette.dark { 0.105 } else { 0.34 });
    highlight.add_color_stop_rgba(0.32, 1.0, 1.0, 1.0, 0.023);
    highlight.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
    set_linear(cr, &highlight);
    cr.paint().ok();

    let shine = cairo::RadialGradient::new(
        pill.x + pill.width * 0.16,
        pill.y + pill.height * 0.02,
        0.0,
        pill.x + pill.width * 0.20,
        pill.y + pill.height * 0.04,
        pill.width * 0.55,
    );
    shine.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, if palette.dark { 0.06 } else { 0.22 });
    shine.add_color_stop_rgba(0.6, 1.0, 1.0, 1.0, 0.012);
    shine.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
    set_radial(cr, &shine);
    cr.paint().ok();

    // Very fine grain prevents visible banding on 8-bit surfaces.
    if palette.dark {
        paint_noise(cr, pill);
    }

    let _ = cr.restore();

    // A thin, bright rim with a stronger upper edge.
    let rim = cairo::LinearGradient::new(pill.x, pill.y, pill.x, pill.bottom());
    let border_strength = if palette.dark { 0.23 } else { 0.14 };
    rim.add_color_stop_rgba(
        0.0,
        palette.border.r,
        palette.border.g,
        palette.border.b,
        border_strength,
    );
    rim.add_color_stop_rgba(
        0.45,
        palette.border.r,
        palette.border.g,
        palette.border.b,
        border_strength * 0.42,
    );
    rim.add_color_stop_rgba(
        1.0,
        palette.border.r,
        palette.border.g,
        palette.border.b,
        border_strength * 0.22,
    );
    set_linear(cr, &rim);
    cr.set_line_width(1.0);
    rounded_rect(cr, pill.inflate(-0.5), radius - 0.5);
    let _ = cr.stroke();

    // Inner shadow at the lower edge and a hairline bevel at the top.
    let _ = cr.save();
    rounded_rect(cr, pill, radius);
    cr.clip();
    let bevel = cairo::LinearGradient::new(0.0, pill.y, 0.0, pill.y + 10.0);
    bevel.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, if palette.dark { 0.12 } else { 0.38 });
    bevel.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
    set_linear(cr, &bevel);
    cr.paint().ok();

    let inner_shadow = cairo::LinearGradient::new(0.0, pill.bottom() - 18.0, 0.0, pill.bottom());
    inner_shadow.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.0);
    inner_shadow.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, if palette.dark { 0.34 } else { 0.12 });
    set_linear(cr, &inner_shadow);
    cr.paint().ok();
    let _ = cr.restore();
}

fn set_linear(cr: &Context, gradient: &cairo::LinearGradient) {
    cr.set_source(gradient).ok();
}

fn set_radial(cr: &Context, gradient: &cairo::RadialGradient) {
    cr.set_source(gradient).ok();
}

thread_local! {
    static NOISE: RefCell<Option<ImageSurface>> = const { RefCell::new(None) };
}

fn paint_noise(cr: &Context, pill: Rect) {
    let surface = NOISE.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.is_none() {
            *cell = build_noise_surface().ok();
        }
        cell.as_ref().cloned()
    });
    let Some(surface) = surface else {
        return;
    };
    let pattern = SurfacePattern::create(surface);
    pattern.set_extend(cairo::Extend::Repeat);
    pattern.set_filter(Filter::Fast);
    let _ = cr.save();
    cr.rectangle(pill.x, pill.y, pill.width, pill.height);
    cr.clip();
    let _ = cr.set_source(&pattern);
    let _ = cr.paint_with_alpha(0.22);
    let _ = cr.restore();
}

fn build_noise_surface() -> cairo::Result<ImageSurface> {
    let size = 128;
    let mut surface = ImageSurface::create(Format::ARgb32, size, size)?;
    {
        let stride = surface.stride() as usize;
        let mut data = surface.data().map_err(|_| cairo::Error::InvalidStatus)?;
        let mut state = 0x9E37_79B9_u32;
        for y in 0..size as usize {
            for x in 0..size as usize {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let value = (state & 0xff) as u8;
                // Cairo's ARGB32 surfaces are premultiplied. A white grain dot
                // must therefore have R=G=B=A; storing 255 here would create an
                // invalid premultiplied value and wash out the glass.
                let alpha = (value / 32).min(6);
                let offset = y * stride + x * 4;
                data[offset] = alpha;
                data[offset + 1] = alpha;
                data[offset + 2] = alpha;
                data[offset + 3] = alpha;
            }
        }
    }
    surface.mark_dirty();
    surface.flush();
    Ok(surface)
}

/// Clip a rounded rectangle for album artwork rendering.
pub fn clip_rounded(cr: &Context, rect: Rect, radius: f64) {
    rounded_rect(cr, rect, radius);
    cr.clip();
}

/// Fill the area outside the rounded pill with transparent black before painting.
/// GTK clears the surface, but doing this explicitly keeps the Cairo path
/// deterministic when rendering into offscreen surfaces for screenshots.
pub fn clear(cr: &Context, width: f64, height: f64) {
    let _ = cr.save();
    cr.set_operator(cairo::Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    rounded_rect(cr, Rect::new(0.0, 0.0, width, height), 0.0);
    cr.paint().ok();
    let _ = cr.restore();
}

/// Clip a surface to an integer-aligned rectangle for the GDK input region.
pub fn input_region_rects(pill: Rect) -> RectangleInt {
    let x = pill.x.floor() as i32;
    let y = pill.y.floor() as i32;
    let width = pill.width.ceil() as i32;
    let height = pill.height.ceil() as i32;
    RectangleInt::new(x, y, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_and_light_palettes_keep_contrast() {
        let appearance = Appearance::default();
        let dark = Palette::derive(&appearance, Some([0.2, 0.5, 0.8]), true);
        let light = Palette::derive(&appearance, Some([0.2, 0.5, 0.8]), false);
        assert!(dark.dark);
        assert!(!light.dark);
        let dark_contrast = (dark.text.luminance() + 0.05) / (dark.base_top.luminance() + 0.05);
        assert!(dark_contrast > 5.0);
        let light_contrast = (light.base_top.luminance() + 0.05) / (light.text.luminance() + 0.05);
        assert!(light_contrast > 7.0);
    }

    #[test]
    fn accent_adapts_to_artwork_only_when_enabled() {
        let mut appearance = Appearance {
            accent: "#ff0000".into(),
            adaptive_accent: true,
            ..Appearance::default()
        };
        let adaptive = Palette::derive(&appearance, Some([0.0, 1.0, 0.0]), true);
        assert!(adaptive.accent.g > adaptive.accent.r);
        appearance.adaptive_accent = false;
        let fixed = Palette::derive(&appearance, Some([0.0, 1.0, 0.0]), true);
        assert!(fixed.accent.r > fixed.accent.g);
    }

    #[test]
    fn extreme_accents_are_brought_into_a_usable_range() {
        let appearance = Appearance::default();
        let black = Palette::derive(&appearance, Some([0.0, 0.0, 0.0]), true);
        let white = Palette::derive(&appearance, Some([1.0, 1.0, 1.0]), true);
        assert!(black.accent.luminance() > 0.1);
        assert!(white.accent.luminance() < 0.82);
    }

    #[test]
    fn opacity_respects_user_setting_and_glass_toggle() {
        let mut appearance = Appearance {
            opacity: 0.84,
            ..Appearance::default()
        };
        let blur = Palette::derive(&appearance, None, true);
        assert!(blur.base_alpha < 0.93);
        appearance.glass = false;
        let solid = Palette::derive(&appearance, None, true);
        assert!(solid.base_alpha > 0.95);
    }

    #[test]
    fn input_region_matches_pill_bounds() {
        let region = input_region_rects(Rect::new(10.2, 20.8, 100.4, 40.1));
        assert_eq!(region.x(), 10);
        assert_eq!(region.y(), 20);
        assert_eq!(region.width(), 101);
        assert_eq!(region.height(), 41);
    }
}
