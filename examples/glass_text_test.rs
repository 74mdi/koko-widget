//! Render one glass-text card at large size over a wallpaper. Dev tool.

use koko_widget::{
    config::{Config, WidgetKind},
    ui::{artwork::ArtworkVisual, draw},
};

fn main() -> anyhow::Result<()> {
    let mut config = Config::load().config;
    config.widgets.set_enabled(WidgetKind::Clock, true);
    config.widgets.clock.font_size = 150.0;
    config.widgets.clock.width = 620.0;
    config.widgets.clock.height = 260.0;
    config.widgets.clock.background = false;
    config.widgets.clock.show_label = false;
    let artwork = ArtworkVisual::new();
    let layer = draw::render_preview(WidgetKind::Clock, &config, None, &artwork, 0.0, 684, 324)
        .expect("surface");
    let target = cairo::ImageSurface::create(cairo::Format::ARgb32, 684, 324)?;
    let cr = cairo::Context::new(&target)?;
    cr.set_source_surface(&layer, 0.0, 0.0)?;
    cr.paint()?;
    drop(cr);
    target.write_to_png(&mut std::fs::File::create("/tmp/glass_text.png")?)?;
    println!("wrote /tmp/glass_text.png");
    Ok(())
}
