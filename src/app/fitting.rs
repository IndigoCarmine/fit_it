//! Running fits on a background thread and applying their outcome.

use super::FitApp;
use super::clock::utc_clock;
use super::i18n::t;
use super::widgets;
use crate::fit::{self, FitOutcome, Progress};
use crate::project::DerivedSpec;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

pub(super) struct FitTask {
    pub(super) handle: JoinHandle<Result<FitOutcome, String>>,
    pub(super) progress: Arc<Mutex<Progress>>,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) label: String,
    pub(super) started: Instant,
}

impl FitApp {
    pub(super) fn start_fit(&mut self, global: bool, ctx: &egui::Context) {
        if self.fitting() {
            return;
        }
        let Some(sel) = self.current() else { return };
        let lookup = self.lookup();
        let include: Vec<bool> = self.project.datasets.iter().map(|d| d.include).collect();
        let active = move |i: usize| if global { include[i] } else { i == sel };
        let problem = match self.project.problem(&lookup, &active) {
            Ok(p) => p,
            Err(e) => {
                self.set_error(e);
                return;
            }
        };
        let label = if global {
            t("Global fit", "グローバルフィット").to_string()
        } else {
            t(
                format!("Fit {}", self.project.datasets[sel].tag),
                format!("{} のフィット", self.project.datasets[sel].tag),
            )
        };
        self.rt.undo = Some(
            self.project
                .datasets
                .iter()
                .map(|d| (d.tag.clone(), d.params.clone()))
                .collect(),
        );
        let progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let opts = self.project.options;
        let specs = DerivedSpec::pairs(&self.project.derived);
        let tag = self.project.datasets[sel].tag.clone();
        let (p2, c2, ctx2) = (progress.clone(), cancel.clone(), ctx.clone());
        let handle = std::thread::spawn(move || {
            let r = fit::fit(&problem, &opts, Some(&p2), Some(&c2)).map(|mut out| {
                out.derived = fit::derive(&problem, &out, &specs, Some(&tag));
                out
            });
            ctx2.request_repaint();
            r
        });
        self.rt.fit = Some(FitTask {
            handle,
            progress,
            cancel,
            label,
            started: Instant::now(),
        });
        self.set_status(t("Fitting…", "フィット中…"));
    }

    pub(super) fn poll_fit(&mut self, ctx: &egui::Context) {
        let Some(task) = &self.rt.fit else { return };
        if !task.handle.is_finished() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        let task = self.rt.fit.take().unwrap();
        let secs = task.started.elapsed().as_secs_f64();
        match task.handle.join() {
            Ok(Ok(out)) => {
                self.project.clear_stderr();
                self.project.apply_outcome(&out);
                self.rt.report = format!("{} — {}\n{}", task.label, utc_clock(), out.report());
                let chi = widgets::fmt_num(out.redchi);
                let msg = if out.success {
                    t(
                        format!(
                            "{}: converged — reduced χ² = {chi}, {} evaluations, {secs:.2} s",
                            task.label, out.nfev
                        ),
                        format!(
                            "{}: 収束 — reduced χ² = {chi}、評価 {} 回、{secs:.2} 秒",
                            task.label, out.nfev
                        ),
                    )
                } else {
                    t(
                        format!(
                            "{}: not converged — reduced χ² = {chi}, {} evaluations, {secs:.2} s",
                            task.label, out.nfev
                        ),
                        format!(
                            "{}: 未収束 — reduced χ² = {chi}、評価 {} 回、{secs:.2} 秒",
                            task.label, out.nfev
                        ),
                    )
                };
                if out.success {
                    self.set_status(msg);
                } else {
                    self.set_error(format!("{msg} ({})", out.message));
                }
            }
            Ok(Err(e)) => self.set_error(format!("{}: {e}", task.label)),
            Err(_) => self.set_error(t(
                format!("{}: the fit crashed", task.label),
                format!("{}: フィットが異常終了しました", task.label),
            )),
        }
    }

    pub(super) fn undo_fit(&mut self) {
        if let Some(saved) = self.rt.undo.take() {
            for (tag, params) in saved {
                if let Some(d) = self.project.datasets.iter_mut().find(|d| d.tag == tag) {
                    d.params = params;
                }
            }
            self.set_status(t(
                "Restored parameters from before the fit",
                "フィット前のパラメータに戻しました",
            ));
        }
    }
}
