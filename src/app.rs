use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use eframe::egui;

use crate::{
    audio::{load_wav, AudioData},
    output::{start_output, Output},
    script::{parse_script, ScriptItem},
    timeline::{check_consistency, parse_timeline, Timeline, Warnings},
    transport::{Mode, Transport},
    ui::controls::draw_controls,
};

pub struct Project {
    pub script_path: PathBuf,
    pub items: Vec<ScriptItem>,
    pub timeline: Timeline,
    pub audio: AudioData,
    pub warnings: Warnings,
}

pub fn load_project(script_path: &Path) -> Result<Project, String> {
    let script_bytes = std::fs::read(script_path)
        .map_err(|error| format!("台本を読み取れません {}: {error}", script_path.display()))?;
    let script_text = std::str::from_utf8(&script_bytes)
        .map_err(|error| format!("台本は UTF-8 ではありません {}: {error}", script_path.display()))?;
    let parent = script_path
        .parent()
        .ok_or_else(|| format!("台本の親フォルダを取得できません {}", script_path.display()))?;
    let timeline_path = parent.join("timeline").join("timeline.json");
    let timeline_json = std::fs::read_to_string(&timeline_path)
        .map_err(|error| format!("timeline.json を読み取れません {}: {error}", timeline_path.display()))?;
    let timeline = parse_timeline(&timeline_json)?;
    let audio_path = parent.join("full_dialogue.wav");
    let audio = load_wav(&audio_path)?;
    let items = parse_script(script_text);
    let mut warnings = check_consistency(&items, &timeline, &script_bytes);
    if audio.duration_ms().abs_diff(timeline.duration_ms) > 1_000 {
        warnings.0.push(format!(
            "音声長 {}ms と timeline.json の {}ms が1秒以上異なります",
            audio.duration_ms(),
            timeline.duration_ms
        ));
    }
    Ok(Project {
        script_path: script_path.to_owned(),
        items,
        timeline,
        audio,
        warnings,
    })
}

pub struct App {
    pub project: Option<Project>,
    transport: Arc<Mutex<Transport>>,
    output: Option<Output>,
    follow: bool,
    show_ruby: bool,
    error: Option<String>,
}

impl App {
    pub fn new(script_path: Option<PathBuf>) -> Self {
        let mut app = Self {
            project: None,
            transport: Arc::new(Mutex::new(Transport::new(0, 1))),
            output: None,
            follow: true,
            show_ruby: true,
            error: None,
        };
        if let Some(script_path) = script_path {
            app.open_project(script_path);
        }
        app
    }

    fn open_project(&mut self, script_path: PathBuf) {
        match load_project(&script_path) {
            Ok(project) => {
                let transport = Arc::new(Mutex::new(Transport::new(
                    project.audio.frame_count(),
                    project.audio.sample_rate,
                )));
                match start_output(
                    AudioData {
                        sample_rate: project.audio.sample_rate,
                        channels: project.audio.channels,
                        samples: Arc::clone(&project.audio.samples),
                    },
                    Arc::clone(&transport),
                ) {
                    Ok(output) => {
                        self.project = Some(project);
                        self.transport = transport;
                        self.output = Some(output);
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Err(error) => self.error = Some(error),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        egui::TopBottomPanel::top("status").show(ctx, |ui| {
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, error);
            }
            if self.output.as_ref().is_some_and(Output::has_error) {
                ui.colored_label(egui::Color32::RED, "出力デバイスでエラーが発生しました");
            }
            if let Some(project) = &self.project {
                for warning in &project.warnings.0 {
                    ui.colored_label(egui::Color32::YELLOW, warning);
                }
            }
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.follow, "追従");
                ui.checkbox(&mut self.show_ruby, "ルビ表示");
                if ui.button("台本を開く").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("台本", &["txt"])
                        .pick_file()
                {
                    self.open_project(path);
                }
            });
        });

        egui::TopBottomPanel::bottom("controls").show(ctx, |ui| {
            if let Some(project) = &self.project
                && let Ok(mut transport) = self.transport.lock()
            {
                draw_controls(ui, &mut transport, project.audio.duration_ms());
            }
        });
        egui::SidePanel::right("meter")
            .exact_width(90.0)
            .show(ctx, |ui| ui.label("メーター\n(Task 8)"));
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(project) = &self.project {
                ui.heading(project.script_path.display().to_string());
                ui.label("台本表示は Task 7 で追加します。");
            } else {
                ui.label("台本を開くか、起動引数に第N回.txtを指定してください。");
            }
        });

        if self
            .transport
            .lock()
            .is_ok_and(|transport| transport.mode() != Mode::Stopped)
        {
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::{SampleFormat, WavSpec, WavWriter};

    fn temp_project(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("scriptchecker-project-{name}-{}", std::process::id()));
        std::fs::create_dir_all(path.join("timeline")).expect("create project directory");
        path
    }

    #[test]
    fn reports_full_paths_for_missing_project_files() {
        let missing_script = std::env::temp_dir().join("scriptchecker-no-such-project").join("第1回.txt");
        let error = match load_project(&missing_script) {
            Err(error) => error,
            Ok(_) => panic!("missing project must fail"),
        };
        assert!(error.contains("timeline.json") || error.contains("full_dialogue.wav") || error.contains("第1回.txt"));
    }

    #[test]
    fn loads_minimal_project_from_script_siblings() {
        let project_dir = temp_project("minimal");
        let script_path = project_dir.join("第1回.txt");
        let script = "#paragraph[S01] 見出し\n[S01-L001] 蒼真:こんにちは\n";
        std::fs::write(&script_path, script).expect("write script");
        std::fs::write(
            project_dir.join("timeline").join("timeline.json"),
            format!(
                r#"{{"script_sha256":"{}","duration_ms":0,"sample_rate":48000,"lines":[{{"id":"S01-L001","start_ms":0,"end_ms":0,"text":"こんにちは","missing":false}}]}}"#,
                crate::timeline::sha256_hex(script.as_bytes())
            ),
        )
        .expect("write timeline");
        let spec = WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(project_dir.join("full_dialogue.wav"), spec).expect("create wav");
        writer.write_sample(0_i16).expect("write wav");
        writer.finalize().expect("finalize wav");

        let loaded = load_project(&script_path).expect("load minimal project");
        assert_eq!(loaded.items.len(), 2);
        assert_eq!(loaded.timeline.lines.len(), 1);
        assert_eq!(loaded.audio.channels, 1);
        std::fs::remove_dir_all(project_dir).expect("remove project directory");
    }
}
