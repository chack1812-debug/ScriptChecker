fn main() {
    let script_path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions::default();
    if let Err(error) = eframe::run_native(
        "ScriptChecker",
        options,
        Box::new(move |creation_context| {
            if let Err(error) =
                scriptchecker::fonts::install_japanese_font(&creation_context.egui_ctx)
            {
                eprintln!("{error}");
            }
            Ok(Box::new(scriptchecker::app::App::new(script_path)))
        }),
    ) {
        eprintln!("ScriptChecker を起動できません: {error}");
    }
}
