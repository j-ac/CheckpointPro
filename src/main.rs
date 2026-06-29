#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use egui::{
    FontData,
    epaint::text::{FontInsert, InsertFontFamily},
};

use crate::{data_structures::App, licensing::RegistrationInfo};

mod data_structures;
mod err;
mod file_system;
mod licensing;
mod ui;
mod utilities;
mod view_directory;
mod view_history;
mod view_recover;

const LICENSE_PUBLIC_KEY: &[u8; 32] = include_bytes!("../assets/license_public.key");

#[cfg(test)]
mod tests;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 750.0])
            .with_icon(std::sync::Arc::new(load_icon()))
            .with_clamp_size_to_monitor_size(true),
        ..Default::default()
    };
    eframe::run_native(
        "CheckpointPro",
        options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            cc.egui_ctx
                .all_styles_mut(|style| style.interaction.tooltip_delay = 0.05);

            cc.egui_ctx.add_font(FontInsert::new(
                "inter",
                FontData::from_static(include_bytes!("../assets/Inter_18pt-Regular.ttf")),
                vec![InsertFontFamily {
                    family: egui::FontFamily::Proportional,
                    priority: egui::epaint::text::FontPriority::Highest,
                }],
            ));

            let license =
                licensing::load_license().and_then(|text| RegistrationInfo::new(text).ok());

            // Execution loops over the function in impl eframe::App for App from now on.
            Ok(Box::new(App::new(license)))
        }),
    )
}

fn load_icon() -> egui::IconData {
    let (icon_rgba, icon_width, icon_height) = {
        let image = image::load_from_memory(include_bytes!("../assets/icon.png"))
            .expect("failed to load icon")
            .into_rgba8();
        let (w, h) = image.dimensions();
        (image.into_raw(), w, h)
    };
    egui::IconData {
        rgba: icon_rgba,
        width: icon_width,
        height: icon_height,
    }
}
