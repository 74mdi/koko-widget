//! Small, dependency-light utilities shared across the application.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};

/// Resolve `$XDG_CONFIG_HOME/koko-widget`, falling back to `~/.config/koko-widget`.
pub fn config_dir() -> PathBuf {
    if let Some(path) = non_empty_env("XDG_CONFIG_HOME") {
        PathBuf::from(path).join("koko-widget")
    } else {
        home_dir().join(".config").join("koko-widget")
    }
}

/// Resolve `$XDG_CACHE_HOME/koko-widget`, falling back to `~/.cache/koko-widget`.
pub fn cache_dir() -> PathBuf {
    if let Some(path) = non_empty_env("XDG_CACHE_HOME") {
        PathBuf::from(path).join("koko-widget")
    } else {
        home_dir().join(".cache").join("koko-widget")
    }
}

/// Resolve `$XDG_STATE_HOME/koko-widget`, falling back to `~/.local/state/koko-widget`.
pub fn state_dir() -> PathBuf {
    if let Some(path) = non_empty_env("XDG_STATE_HOME") {
        PathBuf::from(path).join("koko-widget")
    } else {
        home_dir().join(".local").join("state").join("koko-widget")
    }
}

/// Resolve `$XDG_CONFIG_HOME/autostart` according to the XDG autostart specification.
pub fn autostart_dir() -> PathBuf {
    if let Some(path) = non_empty_env("XDG_CONFIG_HOME") {
        PathBuf::from(path).join("autostart")
    } else {
        home_dir().join(".config").join("autostart")
    }
}

pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn home_dir() -> PathBuf {
    non_empty_env("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// Write a file atomically, creating its parent directory if needed.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("could not create {}", parent.display()))?;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = parent.join(format!(".koko-widget-{suffix}.tmp"));

    let result = (|| {
        let mut file = fs::File::create(&tmp)
            .with_context(|| format!("could not create temporary file {}", tmp.display()))?;
        file.write_all(bytes)
            .with_context(|| format!("could not write {}", tmp.display()))?;
        file.sync_all().ok();
        fs::rename(&tmp, path).with_context(|| format!("could not replace {}", path.display()))?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Decode percent escapes in a file URL path.
fn percent_decode(input: &str) -> Result<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                bail!("incomplete percent escape");
            }
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3])?;
            let value = u8::from_str_radix(hex, 16)?;
            out.push(value);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).context("file URL is not valid UTF-8")
}

/// Convert an MPRIS artwork URL into either a local path or an HTTP(S) URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtworkSource {
    Local(PathBuf),
    Remote(String),
}

pub fn artwork_source(url: &str) -> Result<ArtworkSource> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        bail!("empty artwork URL");
    }
    if let Some(rest) = trimmed.strip_prefix("file://") {
        // file://host/path is uncommon for MPRIS. Reject a remote host rather than
        // accidentally interpreting it as a local path.
        let path = if let Some(local) = rest.strip_prefix("localhost/") {
            format!("/{local}")
        } else if rest.starts_with('/') {
            rest.to_owned()
        } else {
            bail!("file URL with a remote host is not supported");
        };
        return Ok(ArtworkSource::Local(PathBuf::from(percent_decode(&path)?)));
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Ok(ArtworkSource::Remote(trimmed.to_owned()));
    }
    // A handful of players emit a plain filesystem path despite the specification.
    if Path::new(trimmed).is_absolute() {
        return Ok(ArtworkSource::Local(PathBuf::from(trimmed)));
    }
    bail!("unsupported artwork URL scheme")
}

/// Parse `#rrggbb` or `#rrggbbaa` into 0..=1 RGBA values.
pub fn parse_hex_color(value: &str) -> Option<[f64; 4]> {
    let hex = value.trim().trim_start_matches('#');
    let parse = |range: std::ops::Range<usize>| -> Option<f64> {
        u8::from_str_radix(hex.get(range)?, 16)
            .ok()
            .map(|v| f64::from(v) / 255.0)
    };
    match hex.len() {
        6 => Some([parse(0..2)?, parse(2..4)?, parse(4..6)?, 1.0]),
        8 => Some([parse(0..2)?, parse(2..4)?, parse(4..6)?, parse(6..8)?]),
        _ => None,
    }
}

pub fn format_hex_color([r, g, b]: [f64; 3]) -> String {
    let to_byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", to_byte(r), to_byte(g), to_byte(b))
}

/// Format a position in whole seconds as `m:ss`.
pub fn format_duration(seconds: f64) -> String {
    let total = seconds.max(0.0).floor() as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

pub fn format_remaining(position: f64, duration: f64) -> String {
    format!("-{}", format_duration((duration - position).max(0.0)))
}

pub fn filename_component(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artwork_urls_are_decoded() {
        assert_eq!(
            artwork_source("file:///tmp/My%20Cover.jpg").unwrap(),
            ArtworkSource::Local(PathBuf::from("/tmp/My Cover.jpg"))
        );
        assert_eq!(
            artwork_source("https://example.invalid/cover.jpg").unwrap(),
            ArtworkSource::Remote("https://example.invalid/cover.jpg".into())
        );
        assert!(artwork_source("ftp://example.invalid/a.jpg").is_err());
    }

    #[test]
    fn colours_round_trip() {
        let colour = parse_hex_color("#8a63f4").unwrap();
        assert!((colour[0] - 0.541).abs() < 0.01);
        assert_eq!(format_hex_color([0.541, 0.388, 0.956]), "#8a63f4");
        assert!(parse_hex_color("nope").is_none());
    }

    #[test]
    fn time_formatting_is_compact() {
        assert_eq!(format_duration(0.9), "0:00");
        assert_eq!(format_duration(61.4), "1:01");
        assert_eq!(format_remaining(31.0, 109.0), "-1:18");
    }
}
