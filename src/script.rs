#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seg {
    Plain(String),
    Ruby { base: String, reading: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptLine {
    pub id: String,
    pub scene: String,
    pub cast: String,
    pub segs: Vec<Seg>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneHead {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptItem {
    Scene(SceneHead),
    Line(ScriptLine),
}

pub fn parse_segs(text: &str) -> Vec<Seg> {
    let mut segs = Vec::new();
    let mut plain = String::new();
    let mut cursor = 0;

    while let Some(relative_start) = text[cursor..].find('\'') {
        let start = cursor + relative_start;
        let after_start = start + 1;
        let Some(relative_end) = text[after_start..].find('\'') else {
            plain.push_str(&text[cursor..]);
            cursor = text.len();
            break;
        };
        let end = after_start + relative_end;
        let candidate = &text[after_start..end];
        let mut parts = candidate.split('|');
        let base = parts.next().unwrap_or_default();
        let reading = parts.next();
        if let Some(reading) = reading.filter(|_| parts.next().is_none())
            && !base.is_empty()
            && !reading.is_empty()
        {
            plain.push_str(&text[cursor..start]);
            if !plain.is_empty() {
                segs.push(Seg::Plain(std::mem::take(&mut plain)));
            }
            segs.push(Seg::Ruby {
                base: base.to_owned(),
                reading: reading.to_owned(),
            });
            cursor = end + 1;
        } else {
            plain.push_str(&text[cursor..after_start]);
            cursor = after_start;
        }
    }

    if cursor < text.len() {
        plain.push_str(&text[cursor..]);
    }
    if !plain.is_empty() || segs.is_empty() {
        segs.push(Seg::Plain(plain));
    }
    segs
}

pub fn display_text(segs: &[Seg]) -> String {
    segs.iter()
        .map(|seg| match seg {
            Seg::Plain(text) => text.as_str(),
            Seg::Ruby { base, .. } => base.as_str(),
        })
        .collect()
}

pub fn parse_script(text: &str) -> Vec<ScriptItem> {
    text.trim_start_matches('\u{feff}')
        .lines()
        .filter_map(|line| parse_scene(line).or_else(|| parse_line(line)))
        .collect()
}

fn parse_scene(line: &str) -> Option<ScriptItem> {
    let rest = line.strip_prefix("#paragraph[")?;
    let (id, name) = rest.split_once(']')?;
    (!id.is_empty()).then(|| {
        ScriptItem::Scene(SceneHead {
            id: id.to_owned(),
            name: name.trim().to_owned(),
        })
    })
}

fn parse_line(line: &str) -> Option<ScriptItem> {
    let rest = line.strip_prefix('[')?;
    let (id, after_id) = rest.split_once(']')?;
    let (scene, number) = id.split_once("-L")?;
    if !is_scene_id(scene) || number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let after_id = after_id.trim_start();
    let separator = after_id.find([':', '：'])?;
    let cast = after_id[..separator].trim();
    if cast.is_empty() {
        return None;
    }
    let separator_len = after_id[separator..]
        .chars()
        .next()
        .expect("separator was found")
        .len_utf8();
    let dialogue = &after_id[separator + separator_len..];
    Some(ScriptItem::Line(ScriptLine {
        id: id.to_owned(),
        scene: scene.to_owned(),
        cast: cast.to_owned(),
        segs: parse_segs(dialogue),
    }))
}

fn is_scene_id(scene: &str) -> bool {
    scene.starts_with('S')
        && scene.len() > 1
        && scene[1..].bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ruby_and_keeps_display_text() {
        let segs = parse_segs("'Codex|コーデックス'の非対話");
        assert!(matches!(
            segs.as_slice(),
            [Seg::Ruby { base, reading }, Seg::Plain(tail)]
                if base == "Codex" && reading == "コーデックス" && tail == "の非対話"
        ));
        assert_eq!(display_text(&segs), "Codexの非対話");
    }

    #[test]
    fn keeps_unclosed_or_full_width_ruby_markers_as_plain_text() {
        assert!(matches!(
            parse_segs("ab'cd").as_slice(),
            [Seg::Plain(text)] if text == "ab'cd"
        ));
        assert!(parse_segs("全角｜は区切りでない'a｜b'")
            .iter()
            .all(|seg| matches!(seg, Seg::Plain(_))));
    }

    #[test]
    fn parses_only_scene_heads_and_dialogue_lines() {
        let items = parse_script(
            "# c\n@script\n#paragraph[S01] オープニング\n[S01-L001] 蒼真:こんにちは\n",
        );
        assert!(matches!(
            items.as_slice(),
            [
                ScriptItem::Scene(SceneHead { id, name }),
                ScriptItem::Line(ScriptLine { id: line_id, scene, cast, segs })
            ] if id == "S01"
                && name == "オープニング"
                && line_id == "S01-L001"
                && scene == "S01"
                && cast == "蒼真"
                && display_text(segs) == "こんにちは"
        ));
    }

    #[test]
    fn accepts_bom_crlf_and_ignores_cast_and_visual_explanations() {
        let items = parse_script(
            "\u{feff}# c\r\n@script\r\n#paragraph[S01] オープニング\r\n[S01-L001] 蒼真：こんにちは\r\n@cast\r\n蒼真は説明\r\n#visual 画面\r\n",
        );
        assert_eq!(items.len(), 2);
        assert!(matches!(items[1], ScriptItem::Line(ref line) if line.cast == "蒼真"));
    }
}
