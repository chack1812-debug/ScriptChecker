use std::collections::HashMap;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::script::{display_text, ScriptItem};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TimelineLine {
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    #[serde(default)]
    pub missing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Timeline {
    pub script_sha256: String,
    pub duration_ms: u64,
    pub sample_rate: u32,
    pub lines: Vec<TimelineLine>,
}

pub fn parse_timeline(json: &str) -> Result<Timeline, String> {
    serde_json::from_str(json).map_err(|error| format!("timeline.json の解析に失敗しました: {error}"))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn current_line(timeline: &Timeline, ms: u64) -> Option<usize> {
    timeline
        .lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !line.missing && line.start_ms <= ms)
        .map(|(index, _)| index)
        .next_back()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Warnings(pub Vec<String>);

pub fn check_consistency(items: &[ScriptItem], timeline: &Timeline, script_bytes: &[u8]) -> Warnings {
    let mut warnings = Vec::new();
    if sha256_hex(script_bytes) != timeline.script_sha256 {
        warnings.push(
            "台本がbuild後に変更されています。強調位置がずれる可能性があります".to_owned(),
        );
    }

    let lines_by_id: HashMap<&str, &TimelineLine> = timeline
        .lines
        .iter()
        .map(|line| (line.id.as_str(), line))
        .collect();
    for item in items {
        let ScriptItem::Line(script_line) = item else {
            continue;
        };
        match lines_by_id.get(script_line.id.as_str()) {
            None => warnings.push(format!("台本行 {} は timeline.json にありません", script_line.id)),
            Some(line) if line.missing => warnings.push(format!(
                "台本行 {} は timeline.json で missing:true です",
                script_line.id
            )),
            Some(line) if display_text(&script_line.segs) != line.text => warnings.push(format!(
                "台本行 {} の表示文が timeline.json と一致しません",
                script_line.id
            )),
            Some(_) => {}
        }
    }
    Warnings(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::parse_script;

    const SAMPLE: &str = r#"{
        "script_sha256":"expected",
        "duration_ms":15000,
        "sample_rate":48000,
        "lines":[
            {"id":"S01-L001","start_ms":0,"end_ms":10000,"text":"一行目","missing":false},
            {"id":"S01-L002","start_ms":12000,"end_ms":14000,"text":"二行目","missing":true}
        ]
    }"#;

    #[test]
    fn parses_timeline_and_keeps_previous_non_missing_line_in_gaps() {
        let timeline = parse_timeline(SAMPLE).expect("minimal timeline parses");
        assert_eq!(timeline.lines.len(), 2);
        assert_eq!(timeline.lines[0].start_ms, 0);
        assert_eq!(timeline.lines[1].end_ms, 14_000);
        assert_eq!(current_line(&timeline, 0), Some(0));
        assert_eq!(current_line(&timeline, 13_000), Some(0));
    }

    #[test]
    fn calculates_standard_sha256_hex() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn warns_about_unmapped_or_missing_lines_and_changed_script() {
        let timeline = parse_timeline(SAMPLE).expect("minimal timeline parses");
        let script = parse_script(
            "#paragraph[S01] 見出し\n[S01-L001] 蒼真:一行目\n[S01-L002] 大和:二行目\n[S01-L003] 凪:未対応\n",
        );
        let warnings = check_consistency(&script, &timeline, b"changed");
        assert!(warnings.0.iter().any(|warning| warning.contains("S01-L002")));
        assert!(warnings.0.iter().any(|warning| warning.contains("S01-L003")));
        assert!(warnings
            .0
            .iter()
            .any(|warning| warning.contains("台本がbuild後に変更されています")));
    }

    #[test]
    #[ignore]
    fn loads_episode_60_timeline() {
        let path = std::path::Path::new(
            r"D:\UDS\YouTube\AI道具箱\第60回\timeline\timeline.json",
        );
        let json = std::fs::read_to_string(path).expect("episode 60 timeline exists");
        let timeline = parse_timeline(&json).expect("episode 60 timeline parses");
        assert_eq!(timeline.lines.len(), 114);
        assert_eq!(timeline.duration_ms, 936_710);
    }
}
