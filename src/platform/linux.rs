//! Linux distribution discovery for the diagnostics report.

use std::{collections::HashMap, fs};

pub fn distribution_name() -> Option<String> {
    let text = fs::read_to_string("/etc/os-release").ok()?;
    let values = text
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((
                key.to_owned(),
                value.trim_matches('"').trim_matches('\'').to_owned(),
            ))
        })
        .collect::<HashMap<_, _>>();
    values
        .get("PRETTY_NAME")
        .cloned()
        .or_else(|| values.get("NAME").cloned())
}
