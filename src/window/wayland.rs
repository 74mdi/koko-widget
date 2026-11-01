//! Wayland capability checks shared by layer-shell and toplevel paths.

#[cfg(feature = "layer-shell")]
pub fn is_layer_shell_available() -> bool {
    gtk4_layer_shell::is_supported()
}

#[cfg(not(feature = "layer-shell"))]
pub fn is_layer_shell_available() -> bool {
    false
}

pub fn layer_shell_build_enabled() -> bool {
    cfg!(feature = "layer-shell")
}

pub fn namespace() -> &'static str {
    "koko-widget"
}

/// Recommended niri layer rule for genuine compositor-side liquid glass.
///
/// The `liquid-glass` block is provided by the niri glass fork. Upstream niri
/// accepts the `blur`/`xray` portion; users on upstream should remove the nested
/// `liquid-glass` block.
pub const NIRI_LIQUID_GLASS_RULE: &str = r#"layer-rule {
    match namespace="^koko-widget$"

    background-effect {
        blur true
        xray true
        liquid-glass {
            mode "liquid"
            liquidity 0.6
            refraction-strength 3.5
            power-factor 8.0
            refraction-power 1.2
            fringing 0.35
            edge-lighting 0.4
            glow-weight 0.001
            saturation 1.1
            vibrancy 0.3
        }
    }
}"#;
