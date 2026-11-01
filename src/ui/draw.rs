//! Cairo drawing for the morphing overlay.

use std::{f64::consts::PI, time::Instant};

use cairo::{Context, Filter, Format, ImageSurface, SurfacePattern};
use pango::{Alignment, EllipsizeMode, FontDescription, Weight};
use pangocairo::functions::{create_layout, layout_path, show_layout};

use crate::{
    animation,
    config::{Config, FontWeight, RowAlign, TextAlign, WidgetKind},
    media::metadata::{PlaybackStatus, PlayerSnapshot},
    ui::{
        artwork::{ArtworkLayer, ArtworkVisual},
        glass::{self, Palette, Rgb},
        layout::{self, CardLayout, Control, FrameLayout, Rect},
        timefmt,
    },
    utils::{format_duration, format_remaining},
};

pub struct ViewData<'a> {
    pub config: &'a Config,
    pub snapshot: Option<&'a PlayerSnapshot>,
    pub artwork: &'a ArtworkVisual,
    pub morph: f32,
    pub hover_control: Option<Control>,
    pub focused_control: Option<Control>,
    pub seeking_seconds: Option<f64>,
    pub now: Instant,
    pub system_dark: bool,
    pub high_contrast: bool,
}

pub fn draw(
    kind: WidgetKind,
    cr: &Context,
    width: f64,
    height: f64,
    view: &ViewData<'_>,
) {
    let _ = cr.save();
    cr.set_operator(cairo::Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    cr.rectangle(0.0, 0.0, width, height);
    cr.fill().ok();
    let _ = cr.restore();

    let layout = layout::compute(
        kind,
        view.morph,
        view.config.appearance.corner_radius,
        &view.config.widgets,
    );
    let palette = Palette::derive(
        &view.config.appearance,
        view.artwork.accent(),
        glass::is_dark_theme(view.config.appearance.theme, view.system_dark),
    );
    let palette = if view.high_contrast {
        Palette {
            text: if palette.dark {
                Rgb::WHITE
            } else {
                Rgb {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                }
            },
            secondary: if palette.dark {
                Rgb::WHITE
            } else {
                palette.text
            },
            tertiary: if palette.dark {
                Rgb::WHITE
            } else {
                palette.text
            },
            base_alpha: (palette.base_alpha + 0.06).min(1.0),
            ..palette
        }
    } else {
        palette
    };

    let card = layout.card;
    if view.config.widgets.has_background(kind) {
        paint_card(cr, &card, view, &palette);
    }

    match kind {
        WidgetKind::Media => {
            if view.config.widgets.media.artwork {
                draw_artwork(cr, &layout, view, &palette);
            }
            draw_text(cr, &layout, view, &palette);
            if view.config.widgets.media.show_controls {
                draw_controls(cr, &layout, view, &palette);
            }
            if view.config.widgets.media.show_progress {
                draw_progress(cr, &layout, view, &palette);
            }
        }
        WidgetKind::Clock => draw_clock(cr, &card, view, &palette),
        WidgetKind::Date => draw_date(cr, &card, view, &palette),
        WidgetKind::Text => draw_note(cr, &card, view, &palette),
    }
}

fn paint_card(cr: &Context, card: &CardLayout, view: &ViewData<'_>, palette: &Palette) {
    let rect = card.rect;
    let radius = card.radius;
    if view.config.appearance.glass {
        glass::paint_shadow(cr, rect, radius, *palette);
        glass::paint_glass(cr, rect, radius, *palette, view.config.appearance.blur);
    } else {
        glass::paint_shadow(cr, rect, radius, *palette);
        cr.set_source_rgba(
            palette.base_bottom.r,
            palette.base_bottom.g,
            palette.base_bottom.b,
            0.985,
        );
        glass::rounded_rect(cr, rect, radius);
        cr.fill().ok();
    }
}

/// Shared layout for a simple text card: an optional caption above a value.
struct TextCardLayout {
    label: Rect,
    value: Rect,
}

fn text_card_layout(
    card: &CardLayout,
    has_label: bool,
    lines: usize,
    align: RowAlign,
    label_size: f64,
    value_size: f64,
) -> TextCardLayout {
    let pad_x = 20.0;
    let inner_x = card.rect.x + pad_x;
    let inner_width = (card.rect.width - pad_x * 2.0).max(1.0);
    let lines = lines.max(1) as f64;

    let label_height = if has_label { label_size * 1.6 } else { 0.0 };
    let value_height = value_size * 1.35 * lines;
    let gap = if has_label { 2.0 } else { 0.0 };
    let content_height = label_height + gap + value_height;

    let available = (card.rect.height - content_height).max(0.0);
    let top = match align {
        RowAlign::Top => 12.0,
        RowAlign::Bottom => available - 12.0,
        RowAlign::Center => available * 0.5,
    }
    .max(2.0);

    let y = card.rect.y + top;
    TextCardLayout {
        label: Rect::new(inner_x, y, inner_width, label_height),
        value: Rect::new(
            inner_x,
            y + label_height + gap,
            inner_width,
            value_height.max(1.0),
        ),
    }
}

/// Draw one line of text vertically centred inside `rect` using real metrics.
/// Shrink `size_pt` until `text` fits inside `rect`, so a clock, date or note is
/// never truncated just because the card is narrow. Fitting beats ellipsising
/// for these single-value cards; the media title keeps ellipsis on purpose.
fn fit_size(
    cr: &Context,
    text: &str,
    rect: Rect,
    size_pt: f64,
    weight: Weight,
    single_line: bool,
) -> f64 {
    if text.is_empty() || rect.width <= 0.0 || size_pt <= 0.0 {
        return size_pt;
    }
    let measure = |size: f64| {
        let layout = create_layout(cr);
        let mut description = FontDescription::new();
        description.set_family("Noto Sans");
        description.set_style(pango::Style::Normal);
        description.set_weight(weight);
        description.set_size((size * f64::from(pango::SCALE)) as i32);
        layout.set_font_description(Some(&description));
        layout.set_text(text);
        if single_line {
            layout.set_single_paragraph_mode(true);
        }
        layout.pixel_size()
    };
    let (width, height) = measure(size_pt);
    let width = f64::from(width);
    let height = f64::from(height);
    if width <= rect.width && height <= rect.height {
        return size_pt;
    }
    let by_width = if width > 0.0 { rect.width / width } else { 1.0 };
    let by_height = if height > 0.0 { rect.height / height } else { 1.0 };
    let scale = by_width.min(by_height).min(1.0) * 0.99;
    (size_pt * scale).max(6.0)
}


fn draw_centred_line(
    cr: &Context,
    text: &str,
    rect: Rect,
    size_pt: f64,
    weight: Weight,
    colour: Rgb,
    alpha: f64,
    alignment: Alignment,
) {
    if text.is_empty() || rect.width <= 0.0 {
        return;
    }
    let size_pt = fit_size(cr, text, rect, size_pt, weight, true);
    let layout = create_layout(cr);
    let mut description = FontDescription::new();
    description.set_family("Noto Sans");
    description.set_style(pango::Style::Normal);
    description.set_weight(weight);
    description.set_size((size_pt * f64::from(pango::SCALE)) as i32);
    layout.set_font_description(Some(&description));
    layout.set_text(text);
    layout.set_single_paragraph_mode(true);
    layout.set_ellipsize(EllipsizeMode::End);
    layout.set_width((rect.width.max(1.0) * f64::from(pango::SCALE)) as i32);
    layout.set_alignment(alignment);
    let (_, height) = layout.pixel_size();
    let y = rect.y + (rect.height - f64::from(height)) * 0.5;
    let _ = cr.save();
    cr.move_to(rect.x, y);
    cr.set_source_rgba(colour.r, colour.g, colour.b, alpha.clamp(0.0, 1.0));
    show_layout(cr, &layout);
    let _ = cr.restore();
}

fn draw_card_label(
    cr: &Context,
    rect: Rect,
    text: &str,
    size_pt: f64,
    palette: &Palette,
    align: TextAlign,
) {
    draw_text_line(
        cr,
        text,
        rect,
        size_pt,
        Weight::Normal,
        palette.tertiary,
        0.72,
        pango_alignment(align),
        false,
    );
}

fn draw_clock(cr: &Context, card: &CardLayout, view: &ViewData<'_>, palette: &Palette) {
    let config = &view.config.widgets.clock;
    let text = timefmt::clock_text(config.hour_format, config.show_seconds);
    let layout = text_card_layout(
        card,
        config.show_label,
        1,
        view.config.widgets.align,
        config.label_size,
        config.font_size,
    );
    if config.show_label {
        draw_card_label(
            cr,
            layout.label,
            &config.label,
            config.label_size,
            palette,
            TextAlign::Left,
        );
    }
    let colour = configured_colour(config.colour.as_deref(), palette.text);
    if view.config.widgets.clock.background {
        draw_centred_line(
            cr,
            &text,
            layout.value,
            config.font_size,
            pango_weight(config.weight),
            colour,
            1.0,
            pango_alignment(TextAlign::Left),
        );
    } else {
        draw_glass_text(
            cr,
            &text,
            layout.value,
            config.font_size,
            pango_weight(config.weight),
            colour,
            pango_alignment(TextAlign::Left),
            palette,
        );
    }
}

fn draw_date(cr: &Context, card: &CardLayout, view: &ViewData<'_>, palette: &Palette) {
    let config = &view.config.widgets.date;
    let text = timefmt::date_text(config);
    let lines = if config.wrap {
        timefmt::wrap_date(&text)
    } else {
        vec![text]
    };
    let layout = text_card_layout(
        card,
        config.show_label,
        lines.len(),
        view.config.widgets.align,
        config.label_size,
        config.font_size,
    );
    if config.show_label {
        draw_card_label(
            cr,
            layout.label,
            &config.label,
            config.label_size,
            palette,
            TextAlign::Left,
        );
    }
    let colour = configured_colour(config.colour.as_deref(), palette.text);
    let line_height = config.font_size * 1.35;
    let total = line_height * lines.len() as f64;
    let mut y = layout.value.y + (layout.value.height - total).max(0.0) * 0.5;
    for line in &lines {
        let line_rect = Rect::new(layout.value.x, y, layout.value.width, line_height);
        if view.config.widgets.date.background {
            draw_centred_line(
                cr,
                line,
                line_rect,
                config.font_size,
                pango_weight(config.weight),
                colour,
                1.0,
                pango_alignment(TextAlign::Left),
            );
        } else {
            draw_glass_text(
                cr,
                line,
                line_rect,
                config.font_size,
                pango_weight(config.weight),
                colour,
                pango_alignment(TextAlign::Left),
                palette,
            );
        }
        y += line_height;
    }
}

fn draw_note(cr: &Context, card: &CardLayout, view: &ViewData<'_>, palette: &Palette) {
    let config = &view.config.widgets.text;
    let layout = text_card_layout(
        card,
        config.show_label,
        1,
        view.config.widgets.align,
        config.label_size,
        config.font_size,
    );
    if config.show_label {
        draw_card_label(
            cr,
            layout.label,
            &config.label,
            config.label_size,
            palette,
            config.align,
        );
    }
    let colour = configured_colour(config.colour.as_deref(), palette.text);
    if view.config.widgets.text.background {
        draw_centred_line(
            cr,
            &config.text,
            layout.value,
            config.font_size,
            pango_weight(config.weight),
            colour,
            1.0,
            pango_alignment(config.align),
        );
    } else {
        draw_glass_text(
            cr,
            &config.text,
            layout.value,
            config.font_size,
            pango_weight(config.weight),
            colour,
            pango_alignment(config.align),
            palette,
        );
    }
}

/// Glass lettering in the style of `iced_glass`'s MSDF text: each glyph is a
/// solid slab of glass, not a flat fill.
///
/// The look is built from four passes, in order: a soft contact shadow, a
/// darkened extrusion down-right for thickness, a wide chamfer stroked *around*
/// the letterform and graded from a lit upper-left facet to a shaded
/// lower-right one, and finally a bright face painted on top so only the outer
/// half of the chamfer stays visible. A rim highlight and a faint chromatic
/// fringe finish the edge. Nothing is drawn behind the text, so it reads as
/// glass floating on the wallpaper rather than as another card.
#[allow(clippy::too_many_arguments)]
fn draw_glass_text(
    cr: &Context,
    text: &str,
    rect: Rect,
    size_pt: f64,
    weight: Weight,
    colour: Rgb,
    alignment: Alignment,
    palette: &Palette,
) {
    if text.is_empty() || rect.width <= 0.0 {
        return;
    }
    let size_pt = fit_size(cr, text, rect, size_pt, weight, true);
    let layout = create_layout(cr);
    let mut description = FontDescription::new();
    description.set_family("Noto Sans");
    description.set_style(pango::Style::Normal);
    description.set_weight(weight);
    description.set_size((size_pt * f64::from(pango::SCALE)) as i32);
    layout.set_font_description(Some(&description));
    layout.set_text(text);
    layout.set_single_paragraph_mode(true);
    layout.set_ellipsize(EllipsizeMode::End);
    layout.set_width((rect.width.max(1.0) * f64::from(pango::SCALE)) as i32);
    layout.set_alignment(alignment);
    let (width, height) = layout.pixel_size();
    let width = f64::from(width);
    let height = f64::from(height);
    let x = match alignment {
        Alignment::Center => rect.x + (rect.width - width) * 0.5,
        Alignment::Right => rect.x + rect.width - width,
        _ => rect.x,
    };
    let y = rect.y + (rect.height - height) * 0.5;

    // Every dimension scales with the type size, so a 12 pt note and a 58 pt
    // clock read as the same material.
    let depth = (size_pt * 0.070).clamp(1.0, 7.0);
    let chamfer = (size_pt * 0.105).clamp(1.4, 11.0);
    let rim = (size_pt * 0.020).clamp(0.5, 1.9);
    let fringe = (size_pt * 0.018).clamp(0.4, 1.5);
    let glyphs = |cr: &Context| {
        cr.move_to(x, y);
        layout_path(cr, &layout);
    };

    // 1. Contact shadow: three progressively wider, fainter, lower copies of
    //    the silhouette. A cheap stand-in for a blurred shadow.
    for (dy, alpha, widen) in [
        (depth * 0.55, 0.34, 0.0),
        (depth * 1.15, 0.18, rim * 1.6),
        (depth * 1.95, 0.08, rim * 3.4),
    ] {
        let _ = cr.save();
        cr.translate(0.0, dy);
        glyphs(cr);
        cr.set_source_rgba(0.0, 0.0, 0.0, alpha);
        if widen > 0.0 {
            cr.set_line_width(widen);
            cr.fill_preserve().ok();
            cr.stroke().ok();
        } else {
            cr.fill().ok();
        }
        let _ = cr.restore();
    }

    // 2. Extrusion: the silhouette repeated down-right, each slice a little
    //    darker, which builds the shaded side wall of the slab.
    let steps = 12;
    for step in (1..=steps).rev() {
        let t = step as f64 / steps as f64;
        let shade = colour.lighten(0.12).darken(0.08 + 0.62 * t);
        let _ = cr.save();
        cr.translate(depth * t, depth * 1.30 * t);
        glyphs(cr);
        cr.set_source_rgba(shade.r, shade.g, shade.b, 0.96);
        cr.fill().ok();
        let _ = cr.restore();
    }

    // 3. Chamfer: a wide stroke centred on the outline, painted twice from
    //    offset copies of the glyph. The copy shifted down-right is stroked in
    //    shadow, the copy shifted up-left in light. Painting the face on top
    //    afterwards covers the inner half of both, leaving a bevel ring that is
    //    lit on the upper-left and shaded on the lower-right — per glyph, so a
    //    long string stays evenly lit instead of fading left to right.
    let shift = chamfer * 0.62;
    for (dx, dy, r, g, b, alpha) in [
        (shift, shift, 0.26, 0.24, 0.32, 0.55),
        (shift * 0.55, shift * 0.55, 0.30, 0.28, 0.36, 0.30),
        (-shift, -shift, 1.0, 1.0, 1.0, 0.90),
        (-shift * 0.5, -shift * 0.5, 1.0, 1.0, 1.0, 0.55),
    ] {
        let _ = cr.save();
        cr.translate(dx, dy);
        glyphs(cr);
        cr.set_line_width(chamfer * 2.1);
        cr.set_line_join(cairo::LineJoin::Round);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_source_rgba(r, g, b, alpha);
        cr.stroke().ok();
        let _ = cr.restore();
    }

    // 4. Face: bright at the top, settling into the configured colour at the
    //    bottom, like light falling through a thick slab. Opaque enough to stay
    //    legible over any wallpaper.
    let face_top = colour.lighten(if palette.dark { 0.86 } else { 0.90 });
    let face_low = colour.lighten(0.34).mix(palette.base_bottom, 0.20);
    let _ = cr.save();
    glyphs(cr);
    let body = cairo::LinearGradient::new(0.0, y, 0.0, y + height);
    body.add_color_stop_rgba(0.00, face_top.r, face_top.g, face_top.b, 0.97);
    body.add_color_stop_rgba(0.36, face_top.r, face_top.g, face_top.b, 0.93);
    body.add_color_stop_rgba(0.74, colour.lighten(0.62).r, colour.lighten(0.62).g, colour.lighten(0.62).b, 0.90);
    body.add_color_stop_rgba(1.00, face_low.r, face_low.g, face_low.b, 0.92);
    cr.set_source(&body).ok();
    cr.fill().ok();
    let _ = cr.restore();

    // 4b. Sheen: a soft pool of light in the upper part of the face, as if the
    //     slab were catching the sky. Clipped to the glyph.
    let _ = cr.save();
    glyphs(cr);
    cr.clip();
    let sheen = cairo::RadialGradient::new(
        x + width * 0.32,
        y + height * 0.06,
        size_pt * 0.04,
        x + width * 0.32,
        y + height * 0.06,
        width.max(height) * 0.78,
    );
    sheen.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, if palette.dark { 0.40 } else { 0.52 });
    sheen.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
    cr.set_source(&sheen).ok();
    cr.paint().ok();
    let _ = cr.restore();

    // 5. Chromatic fringe: red and blue outlines nudged apart by a fraction of
    //    a pixel, the dispersion a real refractive edge shows.
    for (dx, dy, red, green, blue, alpha) in [
        (-fringe, 0.0, 1.0, 0.40, 0.34, 0.30),
        (fringe, 0.0, 0.34, 0.62, 1.0, 0.30),
    ] {
        let _ = cr.save();
        cr.translate(dx, dy);
        glyphs(cr);
        cr.set_line_width(rim * 0.9);
        cr.set_source_rgba(red, green, blue, alpha);
        cr.stroke().ok();
        let _ = cr.restore();
    }

    // 6. Rim: the specular hairline along the silhouette, strongest at the top
    //    where the light lands.
    let _ = cr.save();
    glyphs(cr);
    cr.set_line_width(rim * 1.3);
    let rim_gradient = cairo::LinearGradient::new(x, y, x, y + height);
    rim_gradient.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.98);
    rim_gradient.add_color_stop_rgba(0.55, 1.0, 1.0, 1.0, 0.52);
    rim_gradient.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.24);
    cr.set_source(&rim_gradient).ok();
    cr.stroke().ok();
    let _ = cr.restore();
}

fn configured_colour(value: Option<&str>, fallback: Rgb) -> Rgb {
    value.and_then(Rgb::hex).unwrap_or(fallback)
}

fn pango_weight(weight: FontWeight) -> Weight {
    match weight {
        FontWeight::Light => Weight::Light,
        FontWeight::Normal => Weight::Normal,
        FontWeight::Medium => Weight::Medium,
        FontWeight::Semibold => Weight::Semibold,
        FontWeight::Bold => Weight::Bold,
    }
}

fn pango_alignment(align: TextAlign) -> Alignment {
    match align {
        TextAlign::Left => Alignment::Left,
        TextAlign::Center => Alignment::Center,
        TextAlign::Right => Alignment::Right,
    }
}

fn draw_artwork(cr: &Context, layout: &FrameLayout, view: &ViewData<'_>, palette: &Palette) {
    let art = layout.art;
    let _ = cr.save();
    glass::clip_rounded(cr, art, layout.art_radius);

    let accent = view.artwork.accent().unwrap_or([0.55, 0.49, 0.96]);
    draw_fallback_art(
        cr,
        art,
        accent,
        palette,
        view.config.appearance.adaptive_accent,
    );

    let progress = view.artwork.transition_progress(view.now);
    if let Some(previous) = &view.artwork.previous {
        paint_cover(cr, art, previous, layout.art_radius);
    }
    if let Some(current) = &view.artwork.current {
        let alpha = if view.artwork.transition_started.is_some() {
            animation::smoothstep(0.0, 1.0, progress)
        } else {
            1.0
        };
        paint_cover_alpha(cr, art, current, layout.art_radius, alpha);
    }

    // A glassy sheen across the top of the art keeps it feeling inset in the
    // surface rather than pasted on top.
    let sheen = cairo::LinearGradient::new(art.x, art.y, art.x, art.y + art.height * 0.55);
    sheen.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.09);
    sheen.add_color_stop_rgba(0.46, 1.0, 1.0, 1.0, 0.012);
    sheen.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
    cr.set_source(&sheen).ok();
    cr.paint().ok();
    let _ = cr.restore();

    // Fine inset rim.
    cr.set_line_width(1.0);
    cr.set_source_rgba(
        if palette.dark { 1.0 } else { 0.1 },
        if palette.dark { 1.0 } else { 0.08 },
        if palette.dark { 1.0 } else { 0.15 },
        if palette.dark { 0.13 } else { 0.15 },
    );
    glass::rounded_rect(cr, art.inflate(-0.5), layout.art_radius - 0.5);
    cr.stroke().ok();
}

fn draw_fallback_art(cr: &Context, art: Rect, accent: [f32; 3], palette: &Palette, adaptive: bool) {
    let accent = Rgb::from_slice(accent);
    let base = Rgb {
        r: 0.055,
        g: 0.052,
        b: 0.09,
    }
    .mix(accent, if adaptive { 0.3 } else { 0.16 });
    let gradient = cairo::LinearGradient::new(art.x, art.y, art.right(), art.bottom());
    gradient.add_color_stop_rgba(0.0, base.r, base.g, base.b, 1.0);
    gradient.add_color_stop_rgba(0.55, base.r * 0.62, base.g * 0.62, base.b * 0.72, 1.0);
    gradient.add_color_stop_rgba(1.0, 0.018, 0.016, 0.030, 1.0);
    let _ = cr.set_source(&gradient);
    let _ = cr.paint();

    // A soft, off-centre light mass.
    let glow = cairo::RadialGradient::new(
        art.x + art.width * 0.24,
        art.y + art.height * 0.2,
        0.0,
        art.x + art.width * 0.26,
        art.y + art.height * 0.24,
        art.width * 0.8,
    );
    glow.add_color_stop_rgba(
        0.0,
        accent.r,
        accent.g,
        accent.b,
        if adaptive { 0.58 } else { 0.3 },
    );
    glow.add_color_stop_rgba(0.55, accent.r, accent.g, accent.b, 0.10);
    glow.add_color_stop_rgba(1.0, accent.r, accent.g, accent.b, 0.0);
    let _ = cr.set_source(&glow);
    let _ = cr.paint();

    // Abstract record grooves and a minimal note.
    let cx = art.x + art.width * 0.67;
    let cy = art.y + art.height * 0.66;
    let radius = art.width * 0.42;
    for ring in 0..5 {
        let r = radius * (0.46 + ring as f64 * 0.13);
        cr.set_line_width(0.8);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.035 + ring as f64 * 0.006);
        cr.arc(cx, cy, r, 0.0, 2.0 * PI);
        cr.stroke().ok();
    }
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.14);
    cr.arc(cx, cy, radius * 0.40, 0.0, 2.0 * PI);
    cr.fill().ok();
    cr.set_source_rgba(base.r, base.g, base.b, 0.8);
    cr.arc(cx, cy, radius * 0.095, 0.0, 2.0 * PI);
    cr.fill().ok();

    let note_x = art.x + art.width * 0.27;
    let note_y = art.y + art.height * 0.67;
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.20);
    cr.set_line_width(art.width * 0.026);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.move_to(note_x + art.width * 0.12, note_y - art.height * 0.16);
    cr.line_to(note_x + art.width * 0.12, note_y + art.height * 0.14);
    cr.stroke().ok();
    cr.arc(
        note_x + art.width * 0.055,
        note_y + art.height * 0.14,
        art.width * 0.07,
        0.0,
        2.0 * PI,
    );
    cr.fill().ok();
    let _ = cr.save();
    cr.translate(note_x + art.width * 0.19, note_y - art.height * 0.12);
    cr.rotate(0.3);
    cr.rectangle(
        -art.width * 0.15,
        -art.height * 0.03,
        art.width * 0.15,
        art.height * 0.06,
    );
    cr.fill().ok();
    let _ = cr.restore();

    // Vignette for depth and text-free shading.
    let vignette = cairo::RadialGradient::new(
        art.x + art.width * 0.5,
        art.y + art.height * 0.46,
        art.width * 0.3,
        art.x + art.width * 0.5,
        art.y + art.height * 0.46,
        art.width * 0.82,
    );
    vignette.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.0);
    vignette.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, if palette.dark { 0.34 } else { 0.18 });
    let _ = cr.set_source(&vignette);
    let _ = cr.paint();
}

fn paint_cover(cr: &Context, art: Rect, layer: &ArtworkLayer, radius: f64) {
    paint_cover_alpha(cr, art, layer, radius, 1.0);
}

fn paint_cover_alpha(cr: &Context, art: Rect, layer: &ArtworkLayer, radius: f64, alpha: f32) {
    if alpha <= 0.001 {
        return;
    }
    let surface = &layer.surface;
    let surface_width = f64::from(surface.width());
    let surface_height = f64::from(surface.height());
    if surface_width <= 0.0 || surface_height <= 0.0 {
        return;
    }
    let scale = (art.width / surface_width).max(art.height / surface_height);
    let draw_width = surface_width * scale;
    let draw_height = surface_height * scale;
    let _ = cr.save();
    glass::clip_rounded(cr, art, radius);
    cr.translate(
        art.x + (art.width - draw_width) * 0.5,
        art.y + (art.height - draw_height) * 0.5,
    );
    cr.scale(scale, scale);
    let pattern = SurfacePattern::create(surface.as_ref());
    pattern.set_filter(Filter::Good);
    cr.set_source(&pattern).ok();
    cr.paint_with_alpha(f64::from(alpha)).ok();
    let _ = cr.restore();
}

fn draw_text(cr: &Context, layout: &FrameLayout, view: &ViewData<'_>, palette: &Palette) {
    let (title, artist, duration, position) = match view.snapshot {
        Some(snapshot) => (
            snapshot.track.title.clone(),
            snapshot.track.artist_line(),
            snapshot.duration_seconds(),
            view.seeking_seconds
                .unwrap_or_else(|| snapshot.position_seconds_at(view.now)),
        ),
        None => (
            "Nothing playing".to_owned(),
            "Start a media player".to_owned(),
            None,
            0.0,
        ),
    };

    let media = &view.config.widgets.media;
    draw_text_line(
        cr,
        &title,
        layout.title,
        layout.title_size_pt + (media.title_size - 13.0),
        Weight::Semibold,
        palette.text,
        1.0,
        Alignment::Left,
        view.high_contrast,
    );
    if !media.show_artist {
        // Skipped: the artist line is disabled in the widget settings.
    } else {
        draw_text_line(
            cr,
            &artist,
            layout.artist,
            layout.artist_size_pt + (media.artist_size - 11.0),
            Weight::Normal,
            palette.secondary,
            if view.high_contrast { 0.96 } else { 0.80 },
            Alignment::Left,
            view.high_contrast,
        );
    }

    if let Some(duration) = duration
        && layout.time_alpha > 0.01
    {
        let elapsed = position.clamp(0.0, duration.max(position));
        draw_text_line(
            cr,
            &format_duration(elapsed),
            Rect::new(
                layout.elapsed_anchor.0,
                layout.elapsed_anchor.1,
                layout.progress_track.width * 0.5,
                layout.time_size_pt * 1.5,
            ),
            layout.time_size_pt,
            Weight::Normal,
            palette.tertiary,
            f64::from(layout.time_alpha) * 0.86,
            Alignment::Left,
            view.high_contrast,
        );
        draw_text_line(
            cr,
            &format_remaining(elapsed, duration),
            Rect::new(
                layout.elapsed_anchor.0 + layout.progress_track.width * 0.5,
                layout.elapsed_anchor.1,
                layout.progress_track.width * 0.5,
                layout.time_size_pt * 1.5,
            ),
            layout.time_size_pt,
            Weight::Normal,
            palette.tertiary,
            f64::from(layout.time_alpha) * 0.86,
            Alignment::Right,
            view.high_contrast,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_text_line(
    cr: &Context,
    text: &str,
    rect: Rect,
    size_pt: f64,
    weight: Weight,
    colour: Rgb,
    alpha: f64,
    alignment: Alignment,
    high_contrast: bool,
) {
    if text.is_empty() || rect.width <= 0.0 {
        return;
    }
    let layout = create_layout(cr);
    let mut description = FontDescription::new();
    description.set_family("Noto Sans");
    description.set_style(pango::Style::Normal);
    let _ = high_contrast;
    description.set_weight(weight);
    description.set_size((size_pt * f64::from(pango::SCALE)) as i32);
    layout.set_font_description(Some(&description));
    layout.set_text(text);
    layout.set_single_paragraph_mode(true);
    layout.set_ellipsize(EllipsizeMode::End);
    layout.set_width((rect.width.max(1.0) * f64::from(pango::SCALE)) as i32);
    layout.set_alignment(alignment);
    let _ = cr.save();
    cr.move_to(rect.x, rect.y);
    cr.set_source_rgba(colour.r, colour.g, colour.b, alpha.clamp(0.0, 1.0));
    show_layout(cr, &layout);
    let _ = cr.restore();
}

fn draw_controls(cr: &Context, layout: &FrameLayout, view: &ViewData<'_>, palette: &Palette) {
    let capabilities = view.snapshot.map(|snapshot| snapshot.capabilities);
    let playing = view
        .snapshot
        .map(|snapshot| snapshot.status == PlaybackStatus::Playing)
        .unwrap_or(false);
    let no_media = view.snapshot.is_none();

    let previous_enabled = capabilities.is_some_and(|caps| caps.can_go_previous);
    let play_enabled =
        capabilities.is_some_and(|caps| caps.can_control && (caps.can_play || caps.can_pause));
    let next_enabled = capabilities.is_some_and(|caps| caps.can_go_next);

    for control in [Control::Previous, Control::PlayPause, Control::Next] {
        let alpha = layout.controls_alpha(control);
        if alpha <= 0.01 {
            continue;
        }
        let enabled = match control {
            Control::Previous => previous_enabled,
            Control::PlayPause => play_enabled && !no_media,
            Control::Next => next_enabled,
        };
        let hovered = view.hover_control == Some(control) && enabled;
        let focused = view.focused_control == Some(control);
        draw_control(
            cr,
            layout,
            palette,
            control,
            alpha,
            enabled,
            hovered,
            focused,
            playing,
            view.high_contrast,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_control(
    cr: &Context,
    layout: &FrameLayout,
    palette: &Palette,
    control: Control,
    alpha: f32,
    enabled: bool,
    hovered: bool,
    focused: bool,
    playing: bool,
    high_contrast: bool,
) {
    let (cx, cy) = layout.control_center(control);
    let icon = layout.icon_size * if hovered { 1.035 } else { 1.0 };
    let control_alpha = if enabled {
        f64::from(alpha)
    } else {
        f64::from(alpha) * 0.26
    };

    if hovered || focused {
        let halo = if high_contrast { 19.0 } else { 18.0 };
        cr.set_source_rgba(
            if palette.dark { 1.0 } else { 0.08 },
            if palette.dark { 1.0 } else { 0.07 },
            if palette.dark { 1.0 } else { 0.12 },
            if focused { 0.14 } else { 0.075 },
        );
        cr.arc(cx, cy, halo, 0.0, 2.0 * PI);
        cr.fill().ok();
        if focused {
            cr.set_line_width(1.2);
            cr.set_source_rgba(palette.accent.r, palette.accent.g, palette.accent.b, 0.85);
            cr.arc(cx, cy, halo + 1.2, 0.0, 2.0 * PI);
            cr.stroke().ok();
        }
    }

    let colour = if palette.dark {
        [1.0, 1.0, 1.0]
    } else {
        [0.08, 0.065, 0.12]
    };
    cr.set_source_rgba(colour[0], colour[1], colour[2], control_alpha);
    match control {
        Control::PlayPause => {
            if playing {
                draw_pause(cr, cx, cy, icon);
            } else {
                draw_play(cr, cx, cy, icon);
            }
        }
        Control::Previous => draw_skip(cr, cx, cy, icon, true),
        Control::Next => draw_skip(cr, cx, cy, icon, false),
    }
}

fn draw_pause(cr: &Context, cx: f64, cy: f64, size: f64) {
    let bar_width = size * 0.24;
    let bar_height = size * 1.03;
    let gap = size * 0.27;
    for offset in [-1.0_f64, 1.0] {
        let x = cx + offset * (gap * 0.5 + bar_width * 0.5) - bar_width * 0.5;
        rounded_rect_simple(
            cr,
            x,
            cy - bar_height * 0.5,
            bar_width,
            bar_height,
            bar_width * 0.38,
        );
        cr.fill().ok();
    }
}

fn draw_play(cr: &Context, cx: f64, cy: f64, size: f64) {
    let width = size * 0.78;
    let height = size * 1.0;
    let x = cx - width * 0.34;
    let y = cy - height * 0.5;
    cr.new_sub_path();
    cr.move_to(x, y);
    cr.line_to(x + width, cy);
    cr.line_to(x, y + height);
    cr.close_path();
    cr.set_line_width(1.6);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.fill_preserve().ok();
    cr.stroke().ok();
}

fn draw_skip(cr: &Context, cx: f64, cy: f64, size: f64, reverse: bool) {
    let direction = if reverse { -1.0 } else { 1.0 };
    let triangle_width = size * 0.48;
    let height = size * 0.96;
    let gap = size * 0.14;
    let centre_shift = -triangle_width * 0.54;
    for index in 0..2 {
        let base_x = cx + direction * (centre_shift + index as f64 * (triangle_width + gap));
        let tip_x = base_x + direction * triangle_width;
        let left_x = if reverse {
            base_x.max(tip_x)
        } else {
            base_x.min(tip_x)
        };
        let right_x = if reverse {
            base_x.min(tip_x)
        } else {
            base_x.max(tip_x)
        };
        cr.new_sub_path();
        cr.move_to(if reverse { right_x } else { left_x }, cy - height * 0.5);
        cr.line_to(if reverse { left_x } else { right_x }, cy);
        cr.line_to(if reverse { right_x } else { left_x }, cy + height * 0.5);
        cr.close_path();
        cr.set_line_width(1.4);
        cr.set_line_join(cairo::LineJoin::Round);
        cr.fill_preserve().ok();
        cr.stroke().ok();
    }
}

fn rounded_rect_simple(cr: &Context, x: f64, y: f64, width: f64, height: f64, radius: f64) {
    let radius = radius.min(width * 0.5).min(height * 0.5);
    cr.new_sub_path();
    cr.arc(x + width - radius, y + radius, radius, -PI / 2.0, 0.0);
    cr.arc(
        x + width - radius,
        y + height - radius,
        radius,
        0.0,
        PI / 2.0,
    );
    cr.arc(x + radius, y + height - radius, radius, PI / 2.0, PI);
    cr.arc(x + radius, y + radius, radius, PI, 3.0 * PI / 2.0);
    cr.close_path();
}

fn draw_progress(cr: &Context, layout: &FrameLayout, view: &ViewData<'_>, palette: &Palette) {
    let Some(snapshot) = view.snapshot else {
        return;
    };
    let Some(duration) = snapshot.duration_seconds() else {
        return;
    };
    if duration <= 0.01 || layout.progress_alpha <= 0.01 {
        return;
    }
    let position = view
        .seeking_seconds
        .unwrap_or_else(|| snapshot.position_seconds_at(view.now))
        .clamp(0.0, duration);
    let fraction = (position / duration).clamp(0.0, 1.0);
    let track = layout.progress_track;
    let alpha = f64::from(layout.progress_alpha);
    let y = track.y + track.height * 0.5;
    let radius = track.height * 0.5;

    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_width(track.height);
    cr.set_source_rgba(
        if palette.dark { 1.0 } else { 0.08 },
        if palette.dark { 1.0 } else { 0.07 },
        if palette.dark { 1.0 } else { 0.12 },
        if palette.dark { 0.14 } else { 0.12 } * alpha,
    );
    cr.move_to(track.x + radius, y);
    cr.line_to(track.right() - radius, y);
    cr.stroke().ok();

    let fill_end = track.x + track.width * fraction;
    if fraction > 0.002 {
        // Soft glow under the filled range.
        cr.set_source_rgba(
            palette.progress.r,
            palette.progress.g,
            palette.progress.b,
            0.13 * alpha,
        );
        cr.set_line_width(track.height + 3.0);
        cr.move_to(track.x + radius, y);
        cr.line_to((fill_end - radius).max(track.x + radius), y);
        cr.stroke().ok();

        cr.set_source_rgba(
            palette.progress.r,
            palette.progress.g,
            palette.progress.b,
            0.98 * alpha,
        );
        cr.set_line_width(track.height);
        cr.move_to(track.x + radius, y);
        cr.line_to((fill_end - radius).max(track.x + radius), y);
        cr.stroke().ok();
    }

    if view.seeking_seconds.is_some() {
        cr.set_source_rgba(
            palette.progress.r,
            palette.progress.g,
            palette.progress.b,
            0.98 * alpha,
        );
        cr.arc(fill_end, y, track.height * 1.5, 0.0, 2.0 * PI);
        cr.fill().ok();
    }

    // A faint inset line gives the bar a physical groove.
    cr.set_source_rgba(
        0.0,
        0.0,
        0.0,
        if palette.dark { 0.16 } else { 0.06 } * alpha,
    );
    cr.set_line_width(0.6);
    cr.move_to(track.x + radius, y + track.height * 0.42);
    cr.line_to(track.right() - radius, y + track.height * 0.42);
    cr.stroke().ok();
}

/// Render a standalone preview surface for diagnostics and visual regression
/// checks. This never participates in the live widget path.
pub fn render_preview(
    kind: WidgetKind,
    config: &Config,
    snapshot: Option<&PlayerSnapshot>,
    artwork: &ArtworkVisual,
    morph: f32,
    width: i32,
    height: i32,
) -> Option<ImageSurface> {
    let surface = ImageSurface::create(Format::ARgb32, width, height).ok()?;
    let cr = Context::new(&surface).ok()?;
    let view = ViewData {
        config,
        snapshot,
        artwork,
        morph,
        hover_control: None,
        focused_control: None,
        seeking_seconds: None,
        now: Instant::now(),
        system_dark: true,
        high_contrast: false,
    };
    draw(kind, &cr, f64::from(width), f64::from(height), &view);
    drop(cr);
    surface.flush();
    Some(surface)
}
