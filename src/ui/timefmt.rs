//! Time and date formatting for the clock and date widgets.
//!
//! `glib::DateTime` is used instead of a separate time-zone crate so the
//! overlay always agrees with the rest of the desktop session.

use glib::DateTime;

use crate::config::{DateWidget, HourFormat};

/// Locale-aware clock string, honouring the configured hour format.
pub fn clock_text(format: HourFormat, show_seconds: bool) -> String {
    let Some(now) = DateTime::now_local()
        .ok()
        .or_else(|| DateTime::now(&glib::TimeZone::local()).ok())
    else {
        return "--:--".to_owned();
    };
    clock_text_at(&now, format, show_seconds)
}

pub fn clock_text_at(now: &DateTime, format: HourFormat, show_seconds: bool) -> String {
    let pattern = match (format, show_seconds) {
        (HourFormat::TwentyFourHour, false) => "%H:%M",
        (HourFormat::TwentyFourHour, true) => "%H:%M:%S",
        (HourFormat::TwelveHour, false) => "%-I:%M %p",
        (HourFormat::TwelveHour, true) => "%-I:%M:%S %p",
        (HourFormat::Auto, false) => {
            if locale_uses_twelve_hour(now) {
                "%-I:%M %p"
            } else {
                "%H:%M"
            }
        }
        (HourFormat::Auto, true) => {
            if locale_uses_twelve_hour(now) {
                "%-I:%M:%S %p"
            } else {
                "%H:%M:%S"
            }
        }
    };
    format_pattern(now, pattern)
}

/// Locale-formatted date, with an invalid pattern falling back to a readable
/// default rather than an empty card.
pub fn date_text(widget: &DateWidget) -> String {
    let Some(now) = DateTime::now_local()
        .ok()
        .or_else(|| DateTime::now(&glib::TimeZone::local()).ok())
    else {
        return String::new();
    };
    let pattern = widget.format.trim();
    let formatted = if pattern.is_empty() {
        String::new()
    } else {
        format_pattern(&now, pattern)
    };
    if formatted.trim().is_empty() {
        format_pattern(&now, "%A %-d %B")
    } else {
        formatted
    }
}

/// Split a long date into at most two balanced lines at word boundaries.
pub fn wrap_date(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.len() < 2 {
        return vec![trimmed.to_owned()];
    }
    let target = trimmed.chars().count() / 2;
    let mut first = String::new();
    let mut best = 1;
    let mut best_delta = usize::MAX;
    for (index, word) in words.iter().enumerate() {
        if index > 0 {
            first.push(' ');
        }
        first.push_str(word);
        if index + 1 == words.len() {
            break;
        }
        let delta = first.chars().count().abs_diff(target);
        if delta < best_delta {
            best_delta = delta;
            best = index + 1;
        }
    }
    let head = words[..best].join(" ");
    let tail = words[best..].join(" ");
    if tail.is_empty() {
        vec![head]
    } else {
        vec![head, tail]
    }
}

fn locale_uses_twelve_hour(now: &DateTime) -> bool {
    let marker = format_pattern(now, "%p");
    let marker = marker.trim();
    !marker.is_empty() && marker != "%p"
}

fn format_pattern(now: &DateTime, pattern: &str) -> String {
    now.format(pattern)
        .map(|value| value.trim().to_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed() -> DateTime {
        // 2026-03-08 15:04:05 UTC, a Sunday.
        DateTime::from_utc(2026, 3, 8, 15, 4, 5.0).unwrap()
    }

    #[test]
    fn twenty_four_hour_format_is_stable() {
        let text = clock_text_at(&fixed(), HourFormat::TwentyFourHour, false);
        assert_eq!(text, "15:04");
        let with_seconds = clock_text_at(&fixed(), HourFormat::TwentyFourHour, true);
        assert_eq!(with_seconds, "15:04:05");
    }

    #[test]
    fn twelve_hour_format_includes_a_marker() {
        let text = clock_text_at(&fixed(), HourFormat::TwelveHour, false);
        assert!(text.contains("3:04"), "unexpected clock text: {text}");
        assert!(
            text.to_lowercase().contains("pm"),
            "unexpected clock text: {text}"
        );
    }

    #[test]
    fn date_patterns_are_expanded() {
        let widget = DateWidget {
            format: "%Y-%m-%d".to_owned(),
            ..DateWidget::default()
        };
        let text = date_text(&widget);
        assert_eq!(text.len(), 10, "unexpected date text: {text}");
        assert_eq!(text.chars().filter(|c| *c == '-').count(), 2);
    }

    #[test]
    fn wrapping_balances_two_lines() {
        let lines = wrap_date("Sunday 8 March");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines.concat().replace(' ', ""), "Sunday8March");
    }

    #[test]
    fn wrapping_keeps_a_single_word_intact() {
        assert_eq!(wrap_date("Sunday"), vec!["Sunday".to_owned()]);
        assert!(wrap_date("   ").is_empty());
    }
}
