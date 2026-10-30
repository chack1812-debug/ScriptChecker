use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use eframe::egui;

use crate::{
    audio::{AudioData, load_wav},
    meter::analyze_meter,
    output::{Output, start_output},
    script::{ScriptItem, parse_script},
    timeline::{Timeline, Warnings, check_consistency, parse_timeline},
    transport::{Mode, Transport},
    ui::{
        controls::{ControlState, apply_command, draw_controls},
        meter_view::{METER_WINDOW_S, MeterViewState, draw_meter},
        script_view::draw_script,
    },
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
    let script_text = std::str::from_utf8(&script_bytes).map_err(|error| {
        format!(
            "台本は UTF-8 ではありません {}: {error}",
            script_path.display()
        )
    })?;
    let parent = script_path
        .parent()
        .ok_or_else(|| format!("台本の親フォルダを取得できません {}", script_path.display()))?;
    let timeline_path = parent.join("timeline").join("timeline.json");
    let timeline_json = std::fs::read_to_string(&timeline_path).map_err(|error| {
        format!(
            "timeline.json を読み取れません {}: {error}",
            timeline_path.display()
        )
    })?;
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
    output_error: Option<String>,
    meter_state: MeterViewState,
    last_seek_epoch: u64,
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
            output_error: None,
            meter_state: MeterViewState::default(),
            last_seek_epoch: 0,
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
                let (output, output_error) = match start_output(
                    AudioData {
                        sample_rate: project.audio.sample_rate,
                        channels: project.audio.channels,
                        samples: Arc::clone(&project.audio.samples),
                    },
                    Arc::clone(&transport),
                ) {
                    Ok(output) => (Some(output), None),
                    Err(error) => (None, Some(error)),
                };
                self.project = Some(project);
                self.transport = transport;
                self.output = output;
                self.output_error = output_error;
                self.error = None;
                self.meter_state.reset();
                self.last_seek_epoch = 0;
            }
            Err(error) => self.error = Some(error),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if let Some(message) = self.output.as_ref().and_then(Output::error_message) {
            if let Ok(mut transport) = self.transport.lock() {
                transport.set_mode(Mode::Stopped);
            }
            self.output_error = Some(format!("出力デバイスエラー: {message}"));
        }
        let output_view = project_view_state(self.output_error.as_deref());
        egui::TopBottomPanel::top("status").show(ctx, |ui| {
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, error);
            }
            if let Some(error) = &output_view.output_error {
                ui.colored_label(egui::Color32::RED, format!("出力エラー: {error}"));
            }
            if let Some(project) = &self.project {
                let missing_count = project
                    .warnings
                    .0
                    .iter()
                    .filter(|warning| is_line_availability_warning(warning))
                    .count();
                if missing_count > 0 {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        format!("timeline.json と対応しない台本行: {missing_count}件"),
                    );
                }
                for warning in project
                    .warnings
                    .0
                    .iter()
                    .filter(|warning| !is_line_availability_warning(warning))
                {
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
            if let Some(project) = &self.project {
                let state = self
                    .transport
                    .lock()
                    .map(|transport| ControlState {
                        position_ms: transport.position_ms(),
                        mode: transport.mode(),
                    })
                    .unwrap_or(ControlState {
                        position_ms: 0,
                        mode: Mode::Stopped,
                    });
                if let Some(command) = draw_controls(
                    ui,
                    state,
                    project.audio.duration_ms(),
                    self.output.is_some() && output_view.playback_enabled,
                ) && let Ok(mut transport) = self.transport.lock()
                {
                    apply_command(&mut transport, command);
                }
            }
        });
        egui::SidePanel::right("meter")
            .exact_width(90.0)
            .show(ctx, |ui| {
                if let Some(project) = &self.project {
                    let (position_ms, stopped, seek_epoch) = self
                        .transport
                        .lock()
                        .map(|transport| {
                            (
                                transport.position_ms(),
                                transport.mode() == Mode::Stopped,
                                transport.seek_epoch(),
                            )
                        })
                        .unwrap_or((0, true, 0));
                    if stopped || seek_epoch != self.last_seek_epoch {
                        self.meter_state.reset();
                        self.last_seek_epoch = seek_epoch;
                    }
                    let meter = analyze_meter(
                        &project.audio.samples,
                        project.audio.channels,
                        project.audio.sample_rate,
                        position_ms as f32 / 1_000.0,
                        METER_WINDOW_S,
                    );
                    draw_meter(ui, &mut self.meter_state, &meter);
                }
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(project) = &self.project {
                ui.heading(project.script_path.display().to_string());
                let position = self
                    .transport
                    .lock()
                    .map(|transport| transport.position_ms())
                    .unwrap_or(0);
                if let Some(seek_to) = draw_script(
                    ui,
                    &project.items,
                    &project.timeline,
                    position,
                    self.follow,
                    self.show_ruby,
                ) && let Ok(mut transport) = self.transport.lock()
                {
                    transport.seek_ms(seek_to as i64);
                }
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

fn is_line_availability_warning(warning: &str) -> bool {
    warning.contains("timeline.json にありません") || warning.contains("missing:true")
}

#[derive(Debug, PartialEq, Eq)]
struct ProjectViewState {
    show_project: bool,
    playback_enabled: bool,
    output_error: Option<String>,
}

fn project_view_state(output_error: Option<&str>) -> ProjectViewState {
    match output_error {
        None => ProjectViewState {
            show_project: true,
            playback_enabled: true,
            output_error: None,
        },
        Some(error) => ProjectViewState {
            show_project: true,
            playback_enabled: false,
            output_error: Some(error.to_owned()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::{SampleFormat, WavSpec, WavWriter};

    fn temp_project(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "scriptchecker-project-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(path.join("timeline")).expect("create project directory");
        path
    }

    #[test]
    fn reports_full_paths_for_missing_project_files() {
        let project_dir = temp_project("missing-timeline");
        let script_path = project_dir.join("第1回.txt");
        std::fs::write(&script_path, "[S01-L001] 蒼真:こんにちは\n").expect("write script");
        let error = match load_project(&script_path) {
            Err(error) => error,
            Ok(_) => panic!("missing timeline must fail"),
        };
        assert!(
            error.contains(
                &project_dir
                    .join("timeline")
                    .join("timeline.json")
                    .display()
                    .to_string()
            )
        );
        std::fs::remove_dir_all(project_dir).expect("remove project directory");

        let project_dir = temp_project("missing-audio");
        let script_path = project_dir.join("第1回.txt");
        let script = "[S01-L001] 蒼真:こんにちは\n";
        std::fs::write(&script_path, script).expect("write script");
        std::fs::write(
            project_dir.join("timeline").join("timeline.json"),
            format!(
                r#"{{"script_sha256":"{}","duration_ms":0,"sample_rate":48000,"lines":[]}}"#,
                crate::timeline::sha256_hex(script.as_bytes())
            ),
        )
        .expect("write timeline");
        let error = match load_project(&script_path) {
            Err(error) => error,
            Ok(_) => panic!("missing audio must fail"),
        };
        assert!(error.contains(&project_dir.join("full_dialogue.wav").display().to_string()));
        std::fs::remove_dir_all(project_dir).expect("remove project directory");
    }

    #[test]
    fn keeps_project_visible_when_output_start_fails() {
        let state = project_view_state(Some("48000Hz 非対応"));
        assert!(state.show_project);
        assert!(!state.playback_enabled);
        assert_eq!(state.output_error.as_deref(), Some("48000Hz 非対応"));
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
        let mut writer =
            WavWriter::create(project_dir.join("full_dialogue.wav"), spec).expect("create wav");
        writer.write_sample(0_i16).expect("write wav");
        writer.finalize().expect("finalize wav");

        let loaded = load_project(&script_path).expect("load minimal project");
        assert_eq!(loaded.items.len(), 2);
        assert_eq!(loaded.timeline.lines.len(), 1);
        assert_eq!(loaded.audio.channels, 1);
        std::fs::remove_dir_all(project_dir).expect("remove project directory");
    }

    #[test]
    fn warns_only_when_audio_duration_differs_by_more_than_one_second() {
        for (name, timeline_duration, expects_warning) in [
            ("duration-warning", 0_u64, true),
            ("duration-match", 2_000_u64, false),
        ] {
            let project_dir = temp_project(name);
            let script_path = project_dir.join("第1回.txt");
            let script = "[S01-L001] 蒼真:こんにちは\n";
            std::fs::write(&script_path, script).expect("write script");
            std::fs::write(
                project_dir.join("timeline").join("timeline.json"),
                format!(
                    r#"{{"script_sha256":"{}","duration_ms":{timeline_duration},"sample_rate":48000,"lines":[{{"id":"S01-L001","start_ms":0,"end_ms":2000,"text":"こんにちは","missing":false}}]}}"#,
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
            let mut writer =
                WavWriter::create(project_dir.join("full_dialogue.wav"), spec).expect("create wav");
            for _ in 0..96_000 {
                writer.write_sample(0_i16).expect("write wav");
            }
            writer.finalize().expect("finalize wav");

            let project = load_project(&script_path).expect("load project");
            assert_eq!(
                project
                    .warnings
                    .0
                    .iter()
                    .any(|warning| warning.contains("音声長")),
                expects_warning
            );
            std::fs::remove_dir_all(project_dir).expect("remove project directory");
        }
    }
}
