//! Geometry for the resting forms, every point in between, and one card.
//!
//! Every widget lives in its own window, so this module computes the canvas for
//! exactly one card at a time. The canvas is the card plus a transparent margin
//! for its shadow; the card itself is always centred in it. The same computed
//! frame drives Cairo drawing, native control placement and the input region.

use crate::{
    animation::{aux_alpha, progress_alpha, time_alpha},
    config::{Config, RowAlign, WidgetKind, Widgets},
};

pub const MEDIA_WIDTH: f64 = 460.0;
pub const PILL_WIDTH: f64 = MEDIA_WIDTH;
pub const COMPACT_HEIGHT: f64 = 112.0;
pub const EXPANDED_HEIGHT: f64 = 236.0;
pub const HORIZONTAL_MARGIN: f64 = 32.0;
pub const VERTICAL_MARGIN: f64 = 32.0;
/// Canvas height of the media card: it must fit the expanded form.
pub const CANVAS_HEIGHT: f64 = EXPANDED_HEIGHT + VERTICAL_MARGIN * 2.0;
/// Canvas width of the media card.
pub const CANVAS_WIDTH: f64 = MEDIA_WIDTH + HORIZONTAL_MARGIN * 2.0;
pub const CONTROL_SIZE: f64 = 44.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width * 0.5, self.y + self.height * 0.5)
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.right() && y >= self.y && y <= self.bottom()
    }

    pub fn inflate(&self, amount: f64) -> Self {
        Self::new(
            self.x - amount,
            self.y - amount,
            self.width + amount * 2.0,
            self.height + amount * 2.0,
        )
    }

    pub fn from_center(cx: f64, cy: f64, width: f64, height: f64) -> Self {
        Self::new(cx - width * 0.5, cy - height * 0.5, width, height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Previous,
    PlayPause,
    Next,
}

/// A glass card. Every window contains exactly one of these.
#[derive(Debug, Clone, Copy)]
pub struct CardLayout {
    pub kind: WidgetKind,
    pub rect: Rect,
    pub radius: f64,
}

#[derive(Debug, Clone)]
pub struct FrameLayout {
    pub progress: f32,
    pub card: CardLayout,
    /// Resting size of this window's canvas.
    pub canvas: (f64, f64),
    pub art: Rect,
    pub art_radius: f64,
    pub title: Rect,
    pub artist: Rect,
    pub title_size_pt: f64,
    pub artist_size_pt: f64,
    pub time_size_pt: f64,
    pub previous_hit: Rect,
    pub play_hit: Rect,
    pub next_hit: Rect,
    pub previous_center: (f64, f64),
    pub play_center: (f64, f64),
    pub next_center: (f64, f64),
    pub icon_size: f64,
    pub progress_track: Rect,
    pub progress_hit: Rect,
    pub elapsed_anchor: (f64, f64),
    pub remaining_anchor: (f64, f64),
    pub previous_alpha: f32,
    pub progress_alpha: f32,
    pub time_alpha: f32,
    pub aux_alpha: f32,
    pub expanded: bool,
}

impl FrameLayout {
    /// The media pill, or the whole card for other widgets.
    pub fn pill(&self) -> Rect {
        self.card.rect
    }

    pub fn control_hit(&self, control: Control) -> Rect {
        match control {
            Control::Previous => self.previous_hit,
            Control::PlayPause => self.play_hit,
            Control::Next => self.next_hit,
        }
    }

    pub fn control_center(&self, control: Control) -> (f64, f64) {
        match control {
            Control::Previous => self.previous_center,
            Control::PlayPause => self.play_center,
            Control::Next => self.next_center,
        }
    }

    pub fn controls_alpha(&self, control: Control) -> f32 {
        match control {
            Control::Previous => self.previous_alpha,
            Control::PlayPause | Control::Next => 1.0,
        }
    }

    pub fn progress_fraction_at(&self, x: f64) -> Option<f64> {
        if self.progress_alpha < 0.4 || !self.progress_hit.contains(x, self.progress_hit.y + 1.0) {
            return None;
        }
        Some(((x - self.progress_track.x) / self.progress_track.width).clamp(0.0, 1.0))
    }

    /// Rectangle that should accept pointer input in this window.
    pub fn input_rect(&self) -> Rect {
        self.card.rect.inflate(1.0)
    }
}

/// Size of a card at rest.
pub fn card_size(kind: WidgetKind, widgets: &Widgets) -> (f64, f64) {
    match kind {
        WidgetKind::Media => (MEDIA_WIDTH, EXPANDED_HEIGHT),
        WidgetKind::Clock => (
            widgets.clock.width.clamp(96.0, 420.0),
            widgets.clock.height.clamp(72.0, 320.0),
        ),
        WidgetKind::Date => (
            widgets.date.width.clamp(96.0, 420.0),
            widgets.date.height.clamp(72.0, 320.0),
        ),
        WidgetKind::Text => (
            widgets.text.width.clamp(96.0, 420.0),
            widgets.text.height.clamp(72.0, 320.0),
        ),
    }
}

/// Canvas size for one card: the card plus its transparent shadow margin.
pub fn canvas_size(kind: WidgetKind, widgets: &Widgets) -> (f64, f64) {
    let (width, height) = card_size(kind, widgets);
    (
        width + HORIZONTAL_MARGIN * 2.0,
        height + VERTICAL_MARGIN * 2.0,
    )
}

/// Canvas size for a whole configuration, used for coarse placement maths.
pub fn row_canvas_size(config: &Config) -> (f64, f64) {
    let (width, height) = config
        .widgets
        .active()
        .iter()
        .map(|kind| card_size(*kind, &config.widgets))
        .fold((0.0_f64, 0.0_f64), |(w, h), (cw, ch)| (w + cw, h.max(ch)));
    (
        width + HORIZONTAL_MARGIN * 2.0,
        height + VERTICAL_MARGIN * 2.0,
    )
}

/// How many logical pixels a card occupies in the row, used for auto-placement.
pub fn row_offset(kind: WidgetKind, widgets: &Widgets) -> f64 {
    let mut offset = 0.0;
    for entry in widgets.active() {
        if entry == kind {
            break;
        }
        offset += card_size(entry, widgets).0 + widgets.gap.max(0.0);
    }
    offset
}

pub fn compute(kind: WidgetKind, morph: f32, corner_radius: f64, widgets: &Widgets) -> FrameLayout {
    let t = f64::from(morph.clamp(0.0, 1.0));
    let (canvas_width, canvas_height) = canvas_size(kind, widgets);
    let (card_width, resting_height) = card_size(kind, widgets);
    let card_height = if kind == WidgetKind::Media {
        lerp(COMPACT_HEIGHT, EXPANDED_HEIGHT, t)
    } else {
        resting_height
    };

    // The media pill is centred vertically in its tall canvas so the morph grows
    // from the middle. Other cards fill their canvas exactly.
    let card = Rect::new(
        HORIZONTAL_MARGIN,
        (canvas_height - card_height) * 0.5,
        card_width,
        card_height,
    );

    let pill = card;
    let pill_y = pill.y;
    let pill_height = pill.height;

    let art_size = lerp(84.0, 208.0, smoothstep_f64(0.0, 0.96, t));
    let art_x = lerp(14.0, 16.0, t) + pill.x;
    let art_y = pill_y + (pill_height - art_size) * 0.5;
    let art = Rect::new(art_x, art_y, art_size, art_size);
    let art_radius = lerp(16.0, 22.0, t);

    let title_x = lerp(116.0, 248.0, t) + pill.x;
    let title_y = pill_y + lerp(32.0, 32.0, t);
    let title_height = lerp(21.0, 24.0, t);
    let title_width = lerp(224.0, 194.0, t);
    let title = Rect::new(title_x, title_y, title_width, title_height);
    let artist_y = pill_y + lerp(54.0, 58.0, t);
    let artist = Rect::new(
        title_x,
        artist_y,
        lerp(224.0, 194.0, t),
        lerp(18.0, 21.0, t),
    );

    let previous_x = lerp(334.0, 274.0, t) + pill.x;
    let play_x = lerp(374.0, 346.0, t) + pill.x;
    let next_x = lerp(418.0, 418.0, t) + pill.x;
    let controls_y = pill_y + lerp(56.0, 130.0, smoothstep_f64(0.0, 1.0, t));
    let previous_center = (previous_x, controls_y);
    let play_center = (play_x, controls_y);
    let next_center = (next_x, controls_y);
    let previous_hit = Rect::from_center(previous_x, controls_y, CONTROL_SIZE, CONTROL_SIZE);
    let play_hit = Rect::from_center(play_x, controls_y, CONTROL_SIZE, CONTROL_SIZE);
    let next_hit = Rect::from_center(next_x, controls_y, CONTROL_SIZE, CONTROL_SIZE);

    let progress_y = pill_y + pill_height - 62.0;
    let progress_x = title_x;
    let progress_width = lerp(224.0, 194.0, t);
    let progress_track = Rect::new(progress_x, progress_y, progress_width, 4.5);
    let progress_hit = Rect::new(progress_x, progress_y - 8.0, progress_width, 22.0);
    let elapsed_anchor = (progress_x, progress_y + 24.0);
    let remaining_anchor = (progress_x + progress_width, progress_y + 24.0);

    let icon_size = lerp(20.0, 26.0, t);
    let aux = aux_alpha(morph);
    let progress_alpha = progress_alpha(morph);
    let time_alpha = time_alpha(morph);

    FrameLayout {
        progress: morph,
        card: CardLayout {
            kind,
            rect: card,
            radius: corner_radius,
        },
        canvas: (canvas_width, canvas_height),
        art,
        art_radius,
        title,
        artist,
        title_size_pt: lerp(12.5, 13.5, t),
        artist_size_pt: lerp(10.5, 11.0, t),
        time_size_pt: 9.5,
        previous_hit,
        play_hit,
        next_hit,
        previous_center,
        play_center,
        next_center,
        icon_size,
        progress_track,
        progress_hit,
        elapsed_anchor,
        remaining_anchor,
        previous_alpha: aux,
        progress_alpha,
        time_alpha,
        aux_alpha: aux,
        expanded: morph > 0.5,
    }
}

/// Vertical alignment helper kept for callers that lay out a mixed row.
pub fn align_offset(align: RowAlign, available: f64, height: f64) -> f64 {
    match align {
        RowAlign::Top => 0.0,
        RowAlign::Bottom => (available - height).max(0.0),
        RowAlign::Center => ((available - height) * 0.5).max(0.0),
    }
}

fn lerp(from: f64, to: f64, t: f64) -> f64 {
    from + (to - from) * t
}

fn smoothstep_f64(edge0: f64, edge1: f64, value: f64) -> f64 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widgets() -> Widgets {
        Widgets::default()
    }

    #[test]
    fn compact_media_fits_inside_its_canvas() {
        let layout = compute(WidgetKind::Media, 0.0, 34.0, &widgets());
        assert_eq!(layout.card.rect.width, PILL_WIDTH);
        assert!((layout.card.rect.height - COMPACT_HEIGHT).abs() < 0.001);
        assert!(layout.card.rect.x >= 0.0);
        assert!(layout.card.rect.bottom() <= layout.canvas.1);
        assert!(layout.art.y >= layout.card.rect.y);
        assert!(layout.art.bottom() <= layout.card.rect.bottom());
        assert!(layout.play_hit.right() <= layout.card.rect.right());
        assert!(layout.next_hit.right() <= layout.card.rect.right());
        assert!(layout.title.right() < layout.play_hit.x);
        assert_eq!(layout.previous_alpha, 0.0);
        assert_eq!(layout.progress_alpha, 0.0);
    }

    #[test]
    fn expanded_media_has_every_control_inside_the_pill() {
        let layout = compute(WidgetKind::Media, 1.0, 34.0, &widgets());
        assert!((layout.card.rect.height - EXPANDED_HEIGHT).abs() < 0.001);
        assert!((layout.art.width - 208.0).abs() < 0.001);
        assert!(layout.art.bottom() <= layout.card.rect.bottom());
        for control in [Control::Previous, Control::PlayPause, Control::Next] {
            let hit = layout.control_hit(control);
            assert!(hit.x >= layout.card.rect.x);
            assert!(hit.right() <= layout.card.rect.right());
            assert!(hit.bottom() <= layout.card.rect.bottom());
        }
        assert!(layout.progress_track.bottom() < layout.card.rect.bottom());
        assert_eq!(layout.previous_alpha, 1.0);
        assert_eq!(layout.progress_alpha, 1.0);
        assert_eq!(layout.time_alpha, 1.0);
    }

    #[test]
    fn media_grows_without_resizing_its_canvas() {
        let compact = compute(WidgetKind::Media, 0.0, 34.0, &widgets());
        let expanded = compute(WidgetKind::Media, 1.0, 34.0, &widgets());
        assert_eq!(compact.canvas, expanded.canvas);
        assert_eq!(compact.canvas, (CANVAS_WIDTH, CANVAS_HEIGHT));
    }

    #[test]
    fn intermediate_media_layout_interpolates_monotonically() {
        let mut last_height = 0.0;
        let mut last_art = 0.0;
        for step in 0..=20 {
            let layout = compute(WidgetKind::Media, step as f32 / 20.0, 34.0, &widgets());
            assert!(layout.card.rect.height >= last_height);
            assert!(layout.art.width >= last_art);
            last_height = layout.card.rect.height;
            last_art = layout.art.width;
        }
    }

    #[test]
    fn progress_hit_testing_works_in_both_states() {
        let expanded = compute(WidgetKind::Media, 1.0, 34.0, &widgets());
        let mid_x = expanded.progress_track.x + expanded.progress_track.width * 0.5;
        let fraction = expanded.progress_fraction_at(mid_x).unwrap();
        assert!((fraction - 0.5).abs() < 0.01);
        let compact = compute(WidgetKind::Media, 0.0, 34.0, &widgets());
        assert!(compact.progress_fraction_at(mid_x).is_none());
    }

    #[test]
    fn input_region_only_covers_the_visible_card() {
        let layout = compute(WidgetKind::Media, 0.3, 34.0, &widgets());
        let input = layout.input_rect();
        assert!(input.width < layout.canvas.0);
        assert!(input.height < layout.canvas.1);
        assert!(input.y > 0.0);
    }

    #[test]
    fn every_widget_gets_its_own_canvas() {
        let widgets = widgets();
        for kind in WidgetKind::ALL {
            let (canvas_w, canvas_h) = canvas_size(kind, &widgets);
            let (card_w, card_h) = card_size(kind, &widgets);
            assert!((canvas_w - card_w - HORIZONTAL_MARGIN * 2.0).abs() < 0.001);
            assert!((canvas_h - card_h - VERTICAL_MARGIN * 2.0).abs() < 0.001);
        }
    }

    #[test]
    fn card_fills_its_canvas_for_non_media_widgets() {
        let widgets = widgets();
        for kind in [WidgetKind::Clock, WidgetKind::Date, WidgetKind::Text] {
            let layout = compute(kind, 0.0, 34.0, &widgets);
            let (card_w, card_h) = card_size(kind, &widgets);
            assert!((layout.card.rect.width - card_w).abs() < 0.001);
            assert!((layout.card.rect.height - card_h).abs() < 0.001);
            assert_eq!(layout.card.kind, kind);
        }
    }

    #[test]
    fn row_offsets_follow_the_configured_order() {
        let mut widgets = widgets();
        widgets.order = vec![WidgetKind::Clock, WidgetKind::Media];
        assert_eq!(row_offset(WidgetKind::Clock, &widgets), 0.0);
        let expected = widgets.clock.width.clamp(96.0, 420.0) + widgets.gap;
        assert!((row_offset(WidgetKind::Media, &widgets) - expected).abs() < 0.001);
    }

    #[test]
    fn alignment_offsets_are_sane() {
        assert_eq!(align_offset(RowAlign::Top, 100.0, 40.0), 0.0);
        assert!((align_offset(RowAlign::Center, 100.0, 40.0) - 30.0).abs() < 0.001);
        assert!((align_offset(RowAlign::Bottom, 100.0, 40.0) - 60.0).abs() < 0.001);
    }
}
