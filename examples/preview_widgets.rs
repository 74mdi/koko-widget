//! Render every widget over a wallpaper and write a PNG. Development tool.

use koko_widget::{
    config::{Config, WidgetKind},
    ui::{artwork::ArtworkVisual, draw, layout},
};

fn main() -> anyhow::Result<()> {
    let mut config = Config::load().config;
    for kind in WidgetKind::ALL {
        config.widgets.set_enabled(kind, true);
    }
    let artwork = ArtworkVisual::new();
    let (mut cursor, base) = (64.0_f64, 220.0_f64);
    let mut surface: Option<cairo::ImageSurface> = None;
    for kind in [
        WidgetKind::Text,
        WidgetKind::Clock,
        WidgetKind::Date,
        WidgetKind::Media,
    ] {
        let (w, h) = layout::canvas_size(kind, &config.widgets);
        let layer = draw::render_preview(kind, &config, None, &artwork, 0.0, w as i32, h as i32)
            .expect("surface");
        if surface.is_none() {
            surface = Some(cairo::ImageSurface::create(
                cairo::Format::ARgb32,
                1920,
                1080,
            )?);
        }
        let target = surface.as_ref().unwrap();
        let cr = cairo::Context::new(target)?;
        cr.set_source_surface(&layer, cursor, base)?;
        cr.paint()?;
        drop(cr);
        cursor += w + 12.0;
    }
    surface
        .unwrap()
        .write_to_png(&mut std::fs::File::create("/tmp/preview.png")?)?;
    println!("wrote /tmp/preview.png");
    Ok(())
}
