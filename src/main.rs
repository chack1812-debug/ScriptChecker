#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    panic::{self, AssertUnwindSafe},
    sync::atomic::{AtomicBool, Ordering},
};

static PANIC_DIALOG_SHOWN: AtomicBool = AtomicBool::new(false);

fn main() {
    install_panic_dialog_hook();
    let script_path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions::default();
    if let Err(error) = eframe::run_native(
        "ScriptChecker",
        options,
        Box::new(move |creation_context| {
            let font_error =
                scriptchecker::fonts::install_japanese_font(&creation_context.egui_ctx).err();
            if let Some(error) = &font_error {
                debug_stderr(error);
            }
            Ok(Box::new(scriptchecker::app::App::with_startup_error(
                script_path,
                font_error,
            )))
        }),
    ) {
        let message = format!("ScriptChecker を起動できません: {error}");
        debug_stderr(&message);
        show_error_dialog(&message);
    }
}

fn install_panic_dialog_hook() {
    panic::set_hook(Box::new(|panic_info| {
        if PANIC_DIALOG_SHOWN.swap(true, Ordering::Relaxed) {
            return;
        }
        let message = format!("予期しないエラーが発生しました: {panic_info}");
        debug_stderr(&message);
        show_error_dialog(&message);
    }));
}

fn show_error_dialog(message: &str) {
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        rfd::MessageDialog::new()
            .set_title("ScriptChecker")
            .set_description(message)
            .set_level(rfd::MessageLevel::Error)
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    }));
}

#[cfg(debug_assertions)]
fn debug_stderr(message: &str) {
    eprintln!("{message}");
}

#[cfg(not(debug_assertions))]
fn debug_stderr(_: &str) {}
