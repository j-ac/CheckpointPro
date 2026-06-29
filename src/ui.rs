#![allow(clippy::large_enum_variant)]
use crate::{
    App, view_directory::DirectoryView, view_history::HistoryView, view_recover::RecoverView,
};
use eframe::egui;

pub enum UIMenu {
    DirectorySelect(DirectoryView),
    Recover(RecoverView),
    History(HistoryView),
}

impl Default for UIMenu {
    fn default() -> Self {
        Self::DirectorySelect(DirectoryView::default())
    }
}

#[derive(Default)]
pub struct UIData {
    menu: UIMenu,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        match &mut self.ui_data.menu {
            UIMenu::DirectorySelect(menu) => {
                let (transition, new_license) = menu.spec(
                    ui,
                    &mut self.repo,
                    &self.registration,
                    self.registration_status,
                    ctx,
                );
                if let Some(transition) = transition {
                    self.ui_data.menu = transition;
                }
                if let Some(new_license) = new_license {
                    self.registration = Some(new_license.clone());
                    self.registration_status = Some(new_license.validate());
                }
            }
            UIMenu::Recover(menu) => {
                if let Some(transition) = menu.spec(ui, ctx, &mut self.repo, &mut self.selected) {
                    self.ui_data.menu = transition;
                }
            }
            UIMenu::History(menu) => {
                menu.spec(ui, ctx, &mut self.selected, &mut self.repo);
            }
        }
    }
}
