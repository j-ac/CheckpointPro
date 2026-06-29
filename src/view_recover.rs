/// The screen that appears if when loading a repository, the software sees that the .wal file is still there
/// and therefore the previous commit failed.
/// For testing purposes, you can get this view to appear by creating a restore.wal file inside a project's checkpoint data folder
use egui::Vec2;

use crate::{
    data_structures::Repo, file_system::ProjectRoot, ui::UIMenu, view_history::HistoryView,
};

pub struct RecoverView {
    commit: usize,
    project_root: ProjectRoot,
    generic_warning: Option<String>,
}

impl RecoverView {
    pub fn new(commit: usize, project_root: ProjectRoot) -> Self {
        RecoverView {
            commit,
            project_root,
            generic_warning: None,
        }
    }
    pub fn spec(
        &mut self,
        ui: &mut egui::Ui,
        ctx: egui::Context,
        repo: &mut Repo,
        selected: &mut Option<usize>,
    ) -> Option<UIMenu> {
        let mut transition = None;

        if let Some(warning) = self.generic_warning.clone() {
            egui::Window::new("Restore Checkpoint Error")
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO) // Pins the window perfectly center
                .collapsible(false)
                .resizable(false)
                .show(&ctx, |ui| {
                    ui.label(warning);

                    ui.vertical_centered(|ui| {
                        if ui.button("Close").clicked() {
                            self.generic_warning = None;
                        }
                    })
                });
        }

        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.10);

            ui.label(
                egui::RichText::new(
                    "Something went wrong the last time you restored a checkpoint.",
                )
                .size(24.0),
            );
            ui.add_space(20.0);

            ui.add(
                egui::Image::new(egui::include_image!("../assets/crash.png"))
                    .fit_to_exact_size(Vec2::new(300., 300.)),
            );
            ui.add_space(20.0);

            ui.label(egui::RichText::new("Don't worry. Your files are still safe.").size(18.0));

            ui.add_space(20.0);

            let button = egui::Button::new(egui::RichText::new("Try again").size(20.0))
                .min_size(egui::vec2(150., 50.0));

            if ui.add(button).clicked() {
                *selected = Some(self.commit);

                if let Err(e) = repo.restore_checkpoint(self.commit, &self.project_root) {
                    self.generic_warning = Some(e.to_string());
                } else {
                    transition = Some(UIMenu::History(HistoryView::new(self.project_root.clone())));
                }
            }
        });

        transition
    }
}
