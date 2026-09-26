//! fit_it: a GUI curve-fitting workbench.
//!
//! The fitting core (`expr`, `model`, `fit`, `data`) has no UI dependencies and is
//! unit-tested on its own; `plugin` turns files in the preset/plugin folders into
//! models; `app` is the egui front end.

pub mod app;
pub mod data;
pub mod export;
pub mod expr;
pub mod fit;
pub mod model;
pub mod plugin;
pub mod project;
pub mod report_pdf;
pub mod transform;

pub use app::FitApp;

/// Display name, used for the window title and the eframe persistence key.
pub const APP_NAME: &str = "fit_it";
