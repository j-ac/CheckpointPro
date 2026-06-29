use std::path::PathBuf;

use egui::{Align2, Context, Ui, vec2};

use crate::{
    data_structures::{CommitResult, Repo, WorkspaceDelta},
    file_system::ProjectRoot,
};

pub struct HistoryView {
    pub project_path: ProjectRoot,

    pub restore_dialog: bool,
    pub new_checkpoint_dialog: bool,

    pub diff_view: bool,
    pub selected_diff_file: Option<PathBuf>,
    pub recalculate_diff: bool,
    pub cached_diff_lines: Vec<(similar::ChangeTag, String)>,

    pub show_project_report: bool,

    pub workspace_delta: Option<WorkspaceDelta>,
    pub checkpoint_name_input: String,
    pub checkpoint_description_input: String,
    pub generic_warning: Option<String>,
}

impl HistoryView {
    pub fn new(root: ProjectRoot) -> Self {
        Self {
            project_path: root,
            restore_dialog: false,
            new_checkpoint_dialog: false,

            diff_view: false,
            selected_diff_file: None,
            recalculate_diff: true,
            cached_diff_lines: Vec::new(),

            show_project_report: false,

            workspace_delta: None,
            checkpoint_name_input: "".to_string(),
            checkpoint_description_input: "".to_string(),
            generic_warning: None,
        }
    }
}

impl HistoryView {
    pub fn show_restore_dialog(&mut self) {
        self.restore_dialog = true;
    }
    pub fn hide_restore_dialog(&mut self) {
        self.restore_dialog = false;
    }
    pub fn show_new_checkpoint_dialog(&mut self) {
        self.new_checkpoint_dialog = true;
    }
    pub fn hide_new_checkpoint_dialog(&mut self) {
        self.new_checkpoint_dialog = false;
    }
}

impl HistoryView {
    pub fn spec(
        &mut self,
        ui: &mut egui::Ui,
        ctx: egui::Context,
        selected: &mut Option<usize>,
        repo: &mut Repo,
    ) {
        if selected.is_none() {
            *selected = repo.current_checkpoint;
        }
        if let Some(warning) = self.generic_warning.clone() {
            egui::Window::new("Checkpoint History Error")
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

        egui::Panel::left("commits")
            .resizable(false)
            .exact_size(ctx.content_rect().width() / 4.0)
            .show_inside(ui, |ui| {
                ui.heading("Checkpoints");
                for (id, c) in repo.commits.iter().enumerate().rev() {
                    let is_selected = *selected == Some(id);

                    let message = {
                        if Some(id) == repo.current_checkpoint {
                            format!("● {}", c.message)
                        } else {
                            c.message.clone()
                        }
                    };

                    if ui.selectable_label(is_selected, message).clicked() {
                        *selected = Some(id);
                    }

                    ui.add_space(3.);
                }
            });

        egui::Panel::bottom("restore_bar").show_inside(ui, |ui| {
            ui.columns(2, |columns| {
                let can_create_checkpoints = repo.is_on_latest_checkpoint();
                let can_restore_checkpoints = selected.is_some();

                let checkpoint_button =
                    egui::Button::new(egui::RichText::new("Create checkpoint").size(32.0))
                        .min_size(egui::vec2(columns[0].available_width(), 100.0));

                columns[0].with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(can_create_checkpoints, checkpoint_button)
                        .on_disabled_hover_text(
                            "You can only create new checkpoints from the latest checkpoint.",
                        )
                        .clicked()
                    {
                        match repo.get_workspace_delta(&self.project_path) {
                            Ok(delta) => {
                                if delta.is_noop() {
                                    self.generic_warning = Some(
                                        "You cannot make a checkpoint when there are no changes!"
                                            .to_string(),
                                    )
                                } else {
                                    self.show_new_checkpoint_dialog();
                                }
                            }
                            Err(e) => {
                                self.generic_warning = Some(e.to_string());
                            }
                        }
                    }
                });

                let restore_button =
                    egui::Button::new(egui::RichText::new("Restore checkpoint").size(30.0))
                        .min_size(egui::vec2(columns[1].available_width(), 100.0));

                columns[1].with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(can_restore_checkpoints, restore_button)
                        .on_disabled_hover_text("You don't have a checkpoint selected!")
                        .clicked()
                    {
                        match repo.get_workspace_delta(&self.project_path) {
                            Ok(delta) => {
                                if !delta.is_noop() {
                                    self.show_restore_dialog();
                                } else {
                                    if let Err(e) = repo.restore_checkpoint(selected.expect("restore button should be greyed out if selected is None."), &self.project_path) {
                                        self.generic_warning = Some(e.to_string());
                                    }
                                }
                            }
                            Err(e) => {
                                self.generic_warning = Some(e.to_string());
                            }
                        }
                    }
                });
            });
        });

        egui::Panel::bottom("on_old_checkpoint_warning")
            .min_size(30.)
            .show_inside(ui, |ui| {
                if !repo.is_on_latest_checkpoint() {
                    ui.label(
                egui::RichText::new(
                    "⚠ Old Checkpoint: Saving is disabled until you return to the latest version",
                )
                .color(egui::Color32::from_rgb(255, 140, 0))
                .heading()
                .strong(),
            );
                }
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.heading("Details");
            if let Some(id) = *selected {
                let commit = repo.commits[id].clone();
                ui.label(format!("Checkpoint: {}", commit.message));
                ui.separator();
                ui.label(commit.description.to_string());

                egui::Panel::bottom("timestamp")
                    .show_inside(ui, |ui| {
                        ui.label(
                            commit.timestamp
                                .format("Created on %B %d, %Y at %I:%M %p")
                                .to_string(),
                        );
                    });


                egui::Panel::bottom("buttons").show_inside(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("View unsaved work").clicked() {
                            self.recalculate_diff = true;
                            match repo.get_workspace_delta(&self.project_path) {
                                Ok(delta) => {
                                    self.workspace_delta = Some(delta);
                                    self.diff_view = true;
                                }
                                Err(e) => self.generic_warning = Some(e.to_string()),
                            }
                        }
                        if ui.button("Project Report").clicked() {
                            self.show_project_report = true;
                        }
                    });
                });

                if self.diff_view{
                    self.diff_view(&ctx, ui, repo);
                }

                if self.restore_dialog {
                    egui::Window::new("WARNING")
                        .collapsible(false)
                        .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                        .resizable(false)
                        .show(&ctx, |ui| {
                            ui.label("Some of your work is not saved. If you restore now you will lose it. Continue?");
                            ui.add_space(8.0);
                            ui.vertical_centered(|ui| {
                                ui.columns(4, |cols| {
                                    cols[1].with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                                        if ui.button("Yes").clicked() {
                                            if let Err(e) = repo.restore_checkpoint(id, &self.project_path) {
                                                self.generic_warning = Some(e.to_string());
                                                self.hide_restore_dialog();
                                                return;
                                            }
                                            self.hide_restore_dialog();
                                        }
                                    });
                                    cols[2].with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                                        if ui.button("Cancel").clicked() {
                                            self.hide_restore_dialog();
                                        }
                                    });
                                });
                            });
                        });
                }
            }

            if self.new_checkpoint_dialog {
                let center = ctx.content_rect().center();
                egui::Window::new("New Checkpoint")
                    .collapsible(false)
                    .default_pos(center)
                    .pivot(Align2::CENTER_CENTER)
                    .resizable(false)
                    .show(&ctx, |ui| {
                        ui.label("Name your checkpoint:");
                        egui::TextEdit::singleline(&mut self.checkpoint_name_input)
                            .hint_text("Fixed the crash on level 2")
                            .show(ui);

                        ui.label("Description (optional):");
                        egui::TextEdit::multiline(&mut self.checkpoint_description_input)
                            .hint_text("Describe what changed, what you fixed, or what you were working on...")
                            .desired_width(350.0)
                            .desired_rows(4)
                            .show(ui);

                        ui.vertical_centered(|ui| {
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Button::new("Create")
                                            .min_size(egui::Vec2 { x: 125., y: 20. }),
                                    )
                                    .clicked()
                                {
                                    match repo.create_commit(&self.project_path.clone(), self.checkpoint_name_input.clone(), self.checkpoint_description_input.clone()) {
                                        Ok(c) => {
                                            self.checkpoint_name_input.clear();
                                            self.checkpoint_description_input.clear();
                                            self.hide_new_checkpoint_dialog();
                                            if c == CommitResult::NoOp {
                                                self.generic_warning = Some("You must have changes to make a checkpoint!".to_string());
                                                return;
                                            } else {
                                                *selected = Some(repo.commits.len() - 1);

                                            }
                                        },
                                        Err(e) => {
                                            self.generic_warning = Some(e.to_string());
                                            return;
                                        }
                                    };


                                }

                                if ui
                                    .add(
                                        egui::Button::new("Cancel")
                                            .min_size(egui::Vec2 { x: 125., y: 20. }),
                                    )
                                    .clicked()
                                {
                                    self.checkpoint_name_input.clear();
                                    self.hide_new_checkpoint_dialog();
                                }
                            });
                        })
                    });
            }


            if self.show_project_report {
                egui::Window::new("Project Report")
                    .collapsible(false)
                    .resizable(false)
                    .fixed_size(egui::vec2(500.0, 550.0))
                    .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(&ctx, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.vertical_centered(|ui| {
                                    egui::Grid::new("report_grid")
                                        .striped(true)
                                        .num_columns(5)
                                        .spacing(egui::vec2(16.0, 6.0))
                                        .show(ui, |ui| {
                                            ui.strong("Checkpoint");
                                            ui.strong("Created");
                                            ui.strong("Additions");
                                            ui.strong("Deletions");
                                            ui.strong("Lines/min");
                                            ui.end_row();

                                            for (id, commit) in repo.commits.iter().enumerate().rev() {
                                                ui.allocate_ui_with_layout(
                                                    egui::vec2(220.0, ui.available_height()),
                                                    egui::Layout::left_to_right(egui::Align::Center),
                                                    |ui| {
                                                        ui.add(
                                                            egui::Label::new(&commit.message).truncate(),
                                                        )
                                                    },
                                                );

                                                ui.label(
                                                    commit
                                                        .timestamp
                                                        .format("Created on %B %d, %Y at %I:%M %p")
                                                        .to_string(),
                                                );

                                                ui.colored_label(
                                                    egui::Color32::from_rgb(100, 200, 100),
                                                    format!("+{}", commit.additions),
                                                );

                                                ui.colored_label(
                                                    egui::Color32::from_rgb(200, 100, 100),
                                                    format!("-{}", commit.deletions),
                                                );

                                                match repo.additions_per_minute(id) {
                                                    Some(rate) => ui.label(format!("{rate:.1}")),
                                                    None => ui.label("—"),
                                                };

                                                ui.end_row();
                                            }
                                        });
                                });
                            });

                        ui.add_space(8.0);
                        ui.vertical_centered(|ui| {
                            if ui.button("Close").clicked() {
                                self.show_project_report = false;
                            }
                        });
                    });
            }


        });
    }
    pub fn diff_view(&mut self, ctx: &Context, _ui: &mut Ui, repo: &Repo) {
        egui::Window::new("Diff")
            .resizable(false)
            .collapsible(false)
            .movable(false)
            .show(ctx, |ui| {
                egui::Panel::left("file_list").show_inside(ui, |ui| {
                    egui::Panel::top("modified").show_inside(ui, |ui| {
                        ui.heading("Modified");
                        if let Some(delta) = &self.workspace_delta {
                            for (path, _, _) in &delta.changed_files {
                                let is_selected =
                                    self.selected_diff_file.as_deref() == Some(path.as_path());
                                if ui
                                    .selectable_label(is_selected, path.to_string_lossy())
                                    .clicked()
                                {
                                    self.selected_diff_file = Some(path.clone());
                                    self.recalculate_diff = true;
                                }
                            }
                        }
                    });
                    egui::Panel::top("new").show_inside(ui, |ui| {
                        ui.heading("New");
                        if let Some(delta) = &self.workspace_delta {
                            for (path, _, _) in &delta.new_files {
                                let is_selected =
                                    self.selected_diff_file.as_deref() == Some(path.as_path());
                                if ui
                                    .selectable_label(is_selected, path.to_string_lossy())
                                    .clicked()
                                {
                                    self.selected_diff_file = Some(path.clone());
                                    self.recalculate_diff = true;
                                }
                            }
                        }
                    });
                    egui::Panel::top("deleted").show_inside(ui, |ui| {
                        ui.heading("Deleted");
                        if let Some(delta) = &self.workspace_delta {
                            for path in &delta.deleted_files {
                                let is_selected =
                                    self.selected_diff_file.as_deref() == Some(path.as_path());
                                if ui
                                    .selectable_label(is_selected, path.to_string_lossy())
                                    .clicked()
                                {
                                    self.selected_diff_file = Some(path.clone());
                                    self.recalculate_diff = true;
                                }
                            }
                        }
                    });
                });

                egui::CentralPanel::default().show_inside(ui, |ui| {
                    egui::Panel::bottom("diff_buttons").show_inside(ui, |ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Refresh").clicked() {
                                self.recalculate_diff = true;
                            }

                            if ui.button("Close").clicked() {
                                self.diff_view = false;
                                self.workspace_delta = None;
                            }
                        });
                    });

                    if self.recalculate_diff
                        && let Some(file) = self.selected_diff_file.clone() {
                            let full_path = self.project_path.get().join(&file);
                            let new_content =
                                std::fs::read_to_string(&full_path).unwrap_or_default();

                            let old_content = repo
                                .last_commit()
                                .and_then(|commit| {
                                    commit
                                        .files
                                        .iter()
                                        .find(|(p, _)| p == &file)
                                        .map(|(_, hash)| *hash)
                                })
                                .and_then(|hash| repo.file_data.get(&hash))
                                .map(|data| String::from_utf8_lossy(&data.0).to_string())
                                .unwrap_or_default();

                            let diff = similar::TextDiff::from_lines(
                                old_content.as_str(),
                                new_content.as_str(),
                            );

                            self.recalculate_diff = false;
                            self.cached_diff_lines.clear(); // Wipe the old frame's data first

                            for group in diff.grouped_ops(5) {
                                for op in group {
                                    for change in diff.iter_changes(&op) {
                                        let prefix = match change.tag() {
                                            similar::ChangeTag::Insert => "+ ",
                                            similar::ChangeTag::Delete => "- ",
                                            similar::ChangeTag::Equal => "  ",
                                        };

                                        let clean_line = format!("{}{}", prefix, change)
                                            .trim_end_matches('\n')
                                            .to_string(); // This clones the text into owned heap memory

                                        self.cached_diff_lines.push((change.tag(), clean_line));
                                    }
                                }
                                // Record where the visual separators live so the cached renderer can draw them
                                self.cached_diff_lines.push((
                                    similar::ChangeTag::Equal,
                                    "--- HUNK BREAK ---".to_string(),
                                ));
                            }
                        }
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (tag, line_text) in &self.cached_diff_lines {
                            if line_text == "--- HUNK BREAK ---" {
                                ui.separator();
                                continue;
                            }

                            let color = match tag {
                                similar::ChangeTag::Insert => {
                                    egui::Color32::from_rgb(100, 200, 100)
                                }
                                similar::ChangeTag::Delete => {
                                    egui::Color32::from_rgb(200, 100, 100)
                                }
                                similar::ChangeTag::Equal => egui::Color32::GRAY,
                            };

                            ui.label(
                                egui::RichText::new(line_text)
                                    .text_style(egui::TextStyle::Monospace)
                                    .color(color),
                            );
                        }
                    });
                });
                ui.set_min_size(vec2(1920., 800.));
            });
    }
}
