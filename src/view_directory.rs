use chrono::Local;
use egui::{Align2, Vec2, vec2};
use egui_file_dialog::{self, FileDialog};

use crate::{
    data_structures::Repo,
    err::{self},
    file_system::{self, ProjectRoot, SentinelFile, get_failed_restore_id},
    licensing::{self, Registration, RegistrationInfo},
    ui::UIMenu,
    view_history::HistoryView,
    view_recover::RecoverView,
};

pub struct DirectoryView {
    pub picked_dir: Option<ProjectRoot>,
    pub new_project_dialog: FileDialog,
    pub existing_project_dialog: FileDialog,
    pub show_legal: bool,
    pub show_collision_warning: bool,
    pub generic_warning: Option<String>,

    //pub registration: Option<RegistrationInfo>,
    pub registered: bool,
    pub registration_window: bool,
    pub license_input: String,
}

impl DirectoryView {
    pub fn spec(
        &mut self,
        ui: &mut egui::Ui,
        repo: &mut Repo,
        registration: &Option<RegistrationInfo>,
        registration_status: Option<Registration>,
        ctx: egui::Context,
    ) -> (Option<UIMenu>, Option<RegistrationInfo>) {
        self.new_project_dialog.update(&ctx);
        self.existing_project_dialog.update(&ctx);

        let mut license_update: Option<RegistrationInfo> = None;

        let licensing_text = match registration_status {
            Some(status) => match status {
                Registration::Unregistered => {
                    "Unregistered CheckpointPro - For personal use only.".to_string()
                }
                Registration::Expired => {
                    "Your registration for CheckpointPro has expired.".to_string()
                }
                Registration::Registered => format!(
                    "Registered to {} — {} days remaining",
                    registration
                        .as_ref()
                        .expect("registration info present without status")
                        .licensee,
                    ((registration.as_ref().unwrap().expiry - Local::now()).num_days())
                ),
            },
            None => "Unregistered CheckpointPro - For personal use only.".to_string(),
        };

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::Panel::bottom("legal").show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        if ui.button("Legal").clicked() {
                            self.show_legal = true;
                        }
                        ui.label(licensing_text);
                    });
                });
            });

            if self.registration_window {
                egui::Window::new("Register CheckpointPro")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(&ctx, |ui| {
                        ui.label("Paste your license below:");
                        ui.add_space(8.0);
                        egui::TextEdit::multiline(&mut self.license_input)
                            .font(egui::TextStyle::Monospace)
                            .desired_rows(8)
                            .desired_width(400.0)
                            .show(ui);
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("Register").clicked() {
                                license_update = match RegistrationInfo::new(self.license_input.clone()) {
                                    Ok(r) => Some(r),
                                    Err(e) => {
                                        self.generic_warning = Some(e.to_string());
                                        return;
                                    },
                                };


                                let registration = license_update.clone().unwrap().validate();
                                if registration == Registration::Registered {
                                    self.registered = true;
                                    self.registration_window = false;
                                    match licensing::save_license(self.license_input.clone()) {
                                        Ok(x) => match x {
                                            licensing::LicenseSaveResult::MachineWide => {
                                                self.generic_warning = Some("Successfuly registered for all users. Thank you for registering CheckpointPro!".to_string())
                                            },
                                            licensing::LicenseSaveResult::UserOnly => {
                                                self.generic_warning = Some("Successfuly registered for this user. Thank you for registering CheckpointPro!".to_string())
                                            },
                                        },
                                        Err(e) => self.generic_warning = Some(format!("Failed to save registration information. Reason: {e}")),
                                    };
                                } else if registration == Registration::Expired {
                                    self.generic_warning = Some("This license is expired.".to_string())
                                } else {
                                    self.generic_warning = Some("Invalid license key. Please try again.".to_string())
                                }
                            }

                            if ui.button("📋 Paste license").clicked()
                                && let Ok(mut clipboard) = arboard::Clipboard::new()
                                    && let Ok(text) = clipboard.get_text() {
                                        self.license_input = text;
                                    }

                            if ui.button("Cancel").clicked() {
                                self.registration_window = false;
                            }
                        });
                    });
            }

            if self.show_legal {
                egui::Window::new("Legal")
                    .collapsible(false)
                    .resizable(false)
                    .fixed_size((500., 300.))
                    .show(&ctx, |ui| {
                        ui.heading("CheckpointPro");
                        ui.label("© 2026 James Correia All rights reserved.");
                        ui.separator();
                        ui.label("CheckpointPro uses Inter. Here is its liscence:");
                        ui.separator();
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            ui.label(include_str!("../assets/OFL.txt"));
                        });
                        ui.separator();
                        if ui.button("Close").clicked() {
                            self.show_legal = false;
                        }
                    });
            }

            if self.show_collision_warning {
                egui::Window::new("Create Project Error")
                    .collapsible(false)
                    .resizable(false)
                    .fixed_size((500., 300.))
                    .show(&ctx, |ui| {
                        ui.centered_and_justified(|ui| {
                            ui.add(
                                egui::Image::new(egui::include_image!("../assets/collision.png"))
                                    .fit_to_exact_size(Vec2::new(300., 218.)),
                            );

                            ui.label("Failed to create project: a project already exists in the selected folder.");

                        });

                        ui.vertical_centered(|ui| {
                            if ui.button("Close").clicked() {
                                self.show_collision_warning = false;
                            }

                        })

                    });
            }

            if let Some(warning) = self.generic_warning.clone() {
                egui::Window::new("Directory Select Error")
                    .collapsible(false)
                    .anchor(Align2::CENTER_CENTER, vec2(0., -200.))
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

            const IMG_SLOT: egui::Vec2 = egui::vec2(300.0, 180.0); // same in BOTH columns
            const BTN_SIZE: egui::Vec2 = egui::vec2(300.0, 60.0);

            ui.vertical_centered(|ui| {
                ui.heading("Welcome to CheckpointPro");
                ui.add_space(100.);

                ui.allocate_ui(egui::vec2(620.0, IMG_SLOT.y + BTN_SIZE.y + 10.0), |ui| {
                    ui.columns(2, |cols| {
                        cols[0].vertical_centered(|ui| {
                            ui.allocate_ui(IMG_SLOT, |ui| {
                                ui.centered_and_justified(|ui| {
                                    ui.add(
                                        egui::Image::new(egui::include_image!(
                                            "../assets/job_racer_man.png"
                                        ))
                                        .max_size(IMG_SLOT), // contain, keep aspect
                                    );
                                });
                            });
                            if ui
                                .add(egui::Button::new("Start new project").min_size(BTN_SIZE))
                                .clicked()
                            {
                                self.new_project_dialog.pick_directory();
                            }
                        });

                        cols[1].vertical_centered(|ui| {
                            ui.allocate_ui(IMG_SLOT, |ui| {
                                ui.centered_and_justified(|ui| {
                                    ui.add(
                                        egui::Image::new(egui::include_image!(
                                            "../assets/bike_auto_race2.png"
                                        ))
                                        .max_size(IMG_SLOT),
                                    );
                                });
                            });
                            if ui
                                .add(egui::Button::new("Open existing project").min_size(BTN_SIZE))
                                .clicked()
                            {
                                self.existing_project_dialog.pick_file();
                            }
                        });
                    });
                });
            });

            ui.add_space(100.0);

            ui.vertical_centered(|ui| {
                let button_text = match self.registered {
                    true => "Manage license",
                    false => "Register CheckpointPro",
                };

                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(button_text).size(24.0),
                        )
                        .min_size(egui::vec2(300.0, 60.0)),
                    )
                    .clicked()
                {
                    self.registration_window = true;
                }
            });
        });
        if let Some(path) = self
            .new_project_dialog
            .take_picked()
            .map(|p| p.to_path_buf())
        {
            let root = ProjectRoot::new(&path);
            match file_system::init_project(&root) {
                Ok(_) => {
                    self.picked_dir = Some(root.clone());
                    return (
                        Some(UIMenu::History(HistoryView::new(root))),
                        license_update,
                    );
                }
                Err(e) => {
                    match e {
                        err::Init::ProjectAlreadyExists => self.show_collision_warning = true,
                        err::Init::Io(_, _) => self.generic_warning = Some(e.to_string()),
                    }
                    return (None, license_update);
                }
            }
        } else if let Some(path) = self.existing_project_dialog.take_picked() {
            let sentinel = SentinelFile::new(path);

            match Repo::load_project(&sentinel) {
                Ok(load) => {
                    *repo = load;
                }
                Err(e) => {
                    self.generic_warning = Some(e.to_string());
                    return (None, license_update);
                }
            };

            if let Some(commit) = get_failed_restore_id(&sentinel.project_root()) {
                return (
                    Some(UIMenu::Recover(RecoverView::new(
                        commit,
                        sentinel.project_root(),
                    ))),
                    license_update,
                );
            } else {
                return (
                    Some(UIMenu::History(HistoryView::new(sentinel.project_root()))),
                    license_update,
                );
            }
        }
        (None, license_update)
    }
}

impl Default for DirectoryView {
    fn default() -> Self {
        let new_project_dialog = FileDialog::new()
            .show_search(false)
            .show_menu_button(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, (0., 0.));

        let existing_project_dialog = FileDialog::new()
            .title("Select a project.checkpoint file")
            .show_search(false)
            .show_menu_button(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, (0., 0.))
            .add_file_filter_extensions("project_file", vec!["checkpoint"])
            .default_file_filter("project_file");

        Self {
            picked_dir: None,
            new_project_dialog,
            existing_project_dialog,
            show_legal: false,
            show_collision_warning: false,

            registered: false,
            registration_window: false,
            license_input: "".to_string(),

            generic_warning: None,
        }
    }
}
