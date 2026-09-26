//! Opening data, model files and projects; saving projects.

use super::FitApp;
use super::i18n::t;
use crate::data::Dataset;
use crate::plugin::SourceKind;
use crate::project::{PROJECT_EXTENSION, Project};
use std::path::{Path, PathBuf};

/// A native "Save as" dialog with one file-type filter.
pub(super) fn save_dialog(filter: &str, extensions: &[&str], file_name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter(filter, extensions)
        .set_file_name(file_name)
        .save_file()
}

impl FitApp {
    pub(super) fn open_data_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            // "All files" first: JASCO exports are often renamed (`.txt.re`, `.txta`).
            .add_filter(t("All files", "すべてのファイル"), &["*"])
            .add_filter(
                t("Data", "データ"),
                &["csv", "tsv", "txt", "dat", "xy", "asc", "prn"],
            )
            .pick_files()
        {
            for p in paths {
                self.open_path(&p);
            }
        }
    }

    fn load_data(&mut self, path: &Path) {
        match Dataset::load(path) {
            Ok(d) => {
                let n = d.table.rows();
                let had_model = self
                    .current()
                    .map(|i| self.project.datasets[i].spec.clone());
                let i = self.project.add_dataset(d);
                // New data usually needs the same model as what is on screen.
                if let Some(spec) = had_model.filter(|s| !s.components.is_empty()) {
                    self.project.datasets[i].spec = spec;
                    let src = self.selected;
                    self.project.datasets[i].params = self.project.datasets[src].params.clone();
                }
                self.selected = i;
                self.sync_params();
                self.set_status(t(
                    format!("Loaded {} ({n} rows)", path.display()),
                    format!("{} を読み込みました ({n} 行)", path.display()),
                ));
            }
            Err(e) => self.set_error(e),
        }
    }

    /// Open whatever was dropped or picked: a project, a model file or data.
    pub(super) fn open_path(&mut self, path: &Path) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if name.ends_with(&format!(".{PROJECT_EXTENSION}")) {
            self.open_project(path);
        } else if SourceKind::from_extension(&ext).is_some() {
            self.install_model_file(path);
        } else {
            self.load_data(path);
        }
    }

    pub(super) fn open_project_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter(t("fit_it project", "fit_it プロジェクト"), &["json"])
            .pick_file()
        {
            self.open_project(&p);
        }
    }

    fn open_project(&mut self, path: &Path) {
        match Project::load(path) {
            Ok(p) => {
                self.project = p;
                self.selected = 0;
                self.project_path = Some(path.to_path_buf());
                self.sync_params();
                self.rt.undo = None;
                self.set_status(t(
                    format!("Opened {}", path.display()),
                    format!("{} を開きました", path.display()),
                ));
            }
            Err(e) => self.set_error(e),
        }
    }

    pub(super) fn save_project(&mut self, save_as: bool) {
        let path = match (&self.project_path, save_as) {
            (Some(p), false) => Some(p.clone()),
            _ => save_dialog(
                t("fit_it project", "fit_it プロジェクト"),
                &[PROJECT_EXTENSION],
                &format!("project.{PROJECT_EXTENSION}"),
            ),
        };
        let Some(mut path) = path else { return };
        if !path.to_string_lossy().ends_with(PROJECT_EXTENSION) {
            path = PathBuf::from(format!(
                "{}.{PROJECT_EXTENSION}",
                path.with_extension("").display()
            ));
        }
        match self.project.save(&path) {
            Ok(()) => {
                self.status_saved(&path);
                self.project_path = Some(path);
            }
            Err(e) => self.set_error(e),
        }
    }

    pub(super) fn new_project(&mut self) {
        self.project = Project::default();
        self.project_path = None;
        self.rt.report.clear();
        self.rt.undo = None;
    }
}
