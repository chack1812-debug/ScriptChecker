use crate::script::Seg;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharSpan {
    pub start_ms: u64,
    pub end_ms: u64,
}

pub fn mora_count(reading: &str) -> f32 {
    reading
        .chars()
        .map(|character| if is_small_kana(character) { 0.0 } else { 1.0 })
        .sum()
}

pub fn char_spans(segs: &[Seg], start_ms: u64, end_ms: u64) -> Vec<CharSpan> {
    let mut weights = Vec::new();
    for seg in segs {
        match seg {
            Seg::Plain(text) => weights.extend(text.chars().map(character_weight)),
            Seg::Ruby { base, reading } => {
                let base_count = base.chars().count();
                if base_count > 0 {
                    let weight = mora_count(reading) / base_count as f32;
                    weights.extend(std::iter::repeat_n(weight, base_count));
                }
            }
        }
    }
    if weights.is_empty() {
        return Vec::new();
    }
    if weights.iter().all(|weight| *weight == 0.0) {
        weights.fill(1.0);
    }

    let total_weight: f32 = weights.iter().sum();
    let duration = end_ms.saturating_sub(start_ms);
    let exact: Vec<f32> = weights
        .iter()
        .map(|weight| duration as f32 * *weight / total_weight)
        .collect();
    let mut allocated: Vec<u64> = exact.iter().map(|value| value.floor() as u64).collect();
    let remainder = duration.saturating_sub(allocated.iter().sum());
    let mut order: Vec<usize> = (0..allocated.len()).collect();
    order.sort_by(|left, right| {
        exact[*right]
            .fract()
            .total_cmp(&exact[*left].fract())
            .then_with(|| left.cmp(right))
    });
    for index in order.into_iter().take(remainder as usize) {
        allocated[index] += 1;
    }

    let mut position = start_ms;
    allocated
        .into_iter()
        .map(|width| {
            let span = CharSpan {
                start_ms: position,
                end_ms: position + width,
            };
            position += width;
            span
        })
        .collect()
}

pub fn char_index_at(spans: &[CharSpan], ms: u64) -> Option<usize> {
    spans
        .iter()
        .position(|span| span.start_ms <= ms && ms < span.end_ms)
}

fn character_weight(character: char) -> f32 {
    match character {
        '、' => 1.0,
        '。' => 2.0,
        character if is_small_kana(character) => 0.0,
        character if is_kana(character) => 1.0,
        character if is_cjk(character) => 2.0,
        character if character.is_ascii_alphanumeric() => 0.5,
        character if character.is_alphabetic() || character.is_numeric() => 0.5,
        _ => 1.0,
    }
}

fn is_small_kana(character: char) -> bool {
    matches!(
        character,
        'ゃ' | 'ゅ'
            | 'ょ'
            | 'ぁ'
            | 'ぃ'
            | 'ぅ'
            | 'ぇ'
            | 'ぉ'
            | 'ャ'
            | 'ュ'
            | 'ョ'
            | 'ァ'
            | 'ィ'
            | 'ゥ'
            | 'ェ'
            | 'ォ'
    )
}

fn is_kana(character: char) -> bool {
    matches!(character, '\u{3040}'..='\u{30ff}')
}

fn is_cjk(character: char) -> bool {
    matches!(character, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::{Seg, display_text};

    #[test]
    fn counts_kana_mora_with_small_kana_and_sokuon() {
        assert_eq!(mora_count("きょう"), 2.0);
        assert_eq!(mora_count("っ"), 1.0);
    }

    #[test]
    fn assigns_contiguous_spans_for_display_characters() {
        let segs = vec![
            Seg::Plain("あ".to_owned()),
            Seg::Ruby {
                base: "東京".to_owned(),
                reading: "とうきょう".to_owned(),
            },
            Seg::Plain("。".to_owned()),
        ];
        let spans = char_spans(&segs, 100, 900);
        assert_eq!(spans.len(), display_text(&segs).chars().count());
        assert_eq!(spans.first().map(|span| span.start_ms), Some(100));
        assert_eq!(spans.last().map(|span| span.end_ms), Some(900));
        assert!(
            spans
                .windows(2)
                .all(|pair| pair[0].end_ms == pair[1].start_ms)
        );
        assert_eq!(
            spans[1].end_ms - spans[1].start_ms,
            spans[2].end_ms - spans[2].start_ms
        );
    }

    #[test]
    fn distributes_ruby_time_in_whole_milliseconds_from_left_to_right() {
        let spans = char_spans(
            &[Seg::Ruby {
                base: "東京".to_owned(),
                reading: "とうきょう".to_owned(),
            }],
            0,
            4_000,
        );
        assert_eq!(
            spans,
            vec![
                CharSpan {
                    start_ms: 0,
                    end_ms: 2_000
                },
                CharSpan {
                    start_ms: 2_000,
                    end_ms: 4_000
                },
            ]
        );

        let spans = char_spans(&[Seg::Plain("abc".to_owned())], 0, 10);
        assert_eq!(
            spans,
            vec![
                CharSpan {
                    start_ms: 0,
                    end_ms: 4
                },
                CharSpan {
                    start_ms: 4,
                    end_ms: 7
                },
                CharSpan {
                    start_ms: 7,
                    end_ms: 10
                },
            ]
        );
    }

    #[test]
    fn gives_full_stop_about_twice_the_width_of_comma() {
        let spans = char_spans(&[Seg::Plain("、。".to_owned())], 0, 300);
        assert_eq!(spans[0].end_ms - spans[0].start_ms, 100);
        assert_eq!(spans[1].end_ms - spans[1].start_ms, 200);
    }

    #[test]
    fn finds_character_index_with_next_character_at_boundary() {
        let spans = char_spans(&[Seg::Plain("abc".to_owned())], 0, 300);
        assert_eq!(char_index_at(&spans, 0), Some(0));
        assert_eq!(char_index_at(&spans, 100), Some(1));
        assert_eq!(char_index_at(&spans, 300), None);
        assert_eq!(char_index_at(&spans, 301), None);
        assert!(char_spans(&[], 0, 100).is_empty());
    }
}
