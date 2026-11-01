//! Media integration: MPRIS discovery, player selection, controls and artwork.

pub mod artwork;
pub mod discovery;
pub mod metadata;
pub mod mpris;
pub mod player_manager;
pub mod service;

pub use service::{MediaCommand, MediaEvent, MediaService};
