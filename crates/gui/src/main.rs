mod app;
mod ipc_client;

use app::KbdSplitApp;

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("KbdSplit")
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([920.0, 620.0]),
        ..Default::default()
    };

    eframe::run_native(
        "KbdSplit",
        native_options,
        Box::new(|cc| Ok(Box::new(KbdSplitApp::new(cc)))),
    )
}
