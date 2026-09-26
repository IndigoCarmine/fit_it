//! The model registry: loading presets and plugins in the background, adding
//! components, and the plugin folder.

use super::FitApp;
use super::i18n::t;
use crate::plugin::{self, Folder, LoadOptions, Registry};
use std::path::Path;
use std::sync::{Arc, mpsc};

impl FitApp {
    fn folders(&self) -> Vec<Folder> {
        let mut v = Vec::new();
        if let Some(p) = plugin::find_presets_dir() {
            v.push(Folder {
                label: "Presets".into(),
                path: p,
            });
        }
        v.push(Folder {
            label: "Plugins".into(),
            path: self.settings.plugin_dir.clone(),
        });
        v
    }

    pub(super) fn reload_models(&mut self, ctx: &egui::Context) {
        if let Err(e) = plugin::prepare_plugin_dir(&self.settings.plugin_dir) {
            self.set_error(e);
        }
        let folders = self.folders();
        let opts = LoadOptions {
            python: self.settings.python.clone(),
        };
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Registry::load(&folders, &opts));
            ctx.request_repaint();
        });
        self.rt.loading = Some(rx);
        self.set_status(t("Loading models…", "モデルを読み込み中…"));
    }

    pub(super) fn poll_registry(&mut self) {
        let Some(rx) = &self.rt.loading else { return };
        let Ok(reg) = rx.try_recv() else { return };
        self.rt.loading = None;
        let errors = reg.issues.iter().filter(|i| !i.warning).count();
        let n = reg.entries.len();
        self.rt.registry = Arc::new(reg);
        self.rt.registry_gen += 1;
        self.sync_params();
        if errors > 0 {
            self.set_error(t(
                format!("Loaded {n} models; {errors} file(s) failed — see Models > Plugins & presets"),
                format!("{n} 個のモデルを読み込みました。{errors} 個のファイルでエラー — モデル > プラグインとプリセット を参照"),
            ));
        } else {
            self.set_status(t(
                format!("Loaded {n} models"),
                format!("{n} 個のモデルを読み込みました"),
            ));
        }
        if let Some(name) = self.rt.pending_add.take() {
            self.add_component(&name);
        }
    }

    pub(super) fn add_component(&mut self, model: &str) {
        let Some(ds) = self.current() else {
            self.set_error(t("Load a dataset first", "先にデータを読み込んでください"));
            return;
        };
        let name = self.project.add_component(ds, model);
        self.sync_params();
        let lookup = self.lookup();
        let idx = self.project.datasets[ds].spec.components.len() - 1;
        match self.project.guess_component(ds, idx, &lookup) {
            Ok(()) => self.set_status(t(
                format!("Added {name} ({model}) with guessed initial values"),
                format!("{name} ({model}) を追加し、初期値を推定しました"),
            )),
            Err(_) => self.set_status(t(
                format!("Added {name} ({model})"),
                format!("{name} ({model}) を追加しました"),
            )),
        }
    }

    /// Show the plugin folder in the file manager, creating it first if needed.
    pub(super) fn open_plugin_dir(&self) {
        let _ = plugin::prepare_plugin_dir(&self.settings.plugin_dir);
        plugin::open_in_os(&self.settings.plugin_dir);
    }

    /// Write `contents` to `file_name` in the plugin folder and reload the models.
    pub(super) fn write_plugin_file(&mut self, file_name: &str, contents: &str) -> bool {
        let file = self.settings.plugin_dir.join(file_name);
        match plugin::prepare_plugin_dir(&self.settings.plugin_dir)
            .and_then(|_| std::fs::write(&file, contents).map_err(|e| e.to_string()))
        {
            Ok(()) => {
                self.rt.windows.reload_requested = true;
                self.status_saved(&file);
                true
            }
            Err(e) => {
                self.set_error(e);
                false
            }
        }
    }

    /// Copy a model file (dropped or opened) into the plugin folder.
    pub(super) fn install_model_file(&mut self, path: &Path) {
        let dest = self
            .settings
            .plugin_dir
            .join(path.file_name().unwrap_or_default());
        match plugin::prepare_plugin_dir(&self.settings.plugin_dir)
            .and_then(|_| std::fs::copy(path, &dest).map_err(|e| e.to_string()))
        {
            Ok(_) => {
                self.rt.windows.reload_requested = true;
                self.set_status(t(
                    format!("Copied {} into the plugin folder", dest.display()),
                    format!("{} をプラグインフォルダにコピーしました", dest.display()),
                ));
            }
            Err(e) => self.set_error(e),
        }
    }
}
