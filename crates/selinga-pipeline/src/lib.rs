#[derive(Debug, Clone)]
pub struct LinePair {
    pub source: String,
    pub translated: String,
    pub rect: Option<selinga_ocr::Rect>,
}

#[derive(Debug, Clone)]
pub struct PipelineOutput {
    pub lines: Vec<LinePair>,
}

impl PipelineOutput {
    pub fn translated_text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.translated.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn bilingual_text(&self) -> String {
        let mut out = Vec::with_capacity(self.lines.len() * 2);
        for line in &self.lines {
            out.push(line.source.as_str());
            out.push(line.translated.as_str());
        }
        out.join("\n")
    }
}

pub fn normalize_translation(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let strip_spaces = trimmed.chars().any(is_cjk);
    let mut folded = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if let Some(replacement) = punct_to_ascii(ch) {
            folded.push_str(replacement);
        } else if ('\u{FF01}'..='\u{FF5E}').contains(&ch) {
            folded.push(char::from_u32(ch as u32 - 0xFEE0).unwrap_or(ch));
        } else if ch == '\u{3000}' {
            folded.push(' ');
        } else {
            folded.push(ch);
        }
    }

    let mut out = String::with_capacity(folded.len());
    let mut previous_space = false;
    for ch in folded.chars() {
        if ch == ' ' {
            if strip_spaces || previous_space {
                continue;
            }
            previous_space = true;
        } else {
            previous_space = false;
        }
        out.push(ch);
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

fn punct_to_ascii(ch: char) -> Option<&'static str> {
    Some(match ch {
        '。' => ".",
        '、' => ",",
        '「' | '」' | '『' | '』' | '“' | '”' | '《' | '》' | '〈' | '〉' => "\"",
        '‘' | '’' => "'",
        '【' => "[",
        '】' => "]",
        '〔' => "(",
        '〕' => ")",
        '…' => "...",
        '—' | '―' => "-",
        '～' | '〜' => "~",
        _ => return None,
    })
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3000..=0x303F
        | 0x3040..=0x30FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF
        | 0xFF00..=0xFFEF
        | 0x20000..=0x2FA1F)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cjk_lines_lose_spaces_and_gain_ascii_punctuation() {
        assert_eq!(normalize_translation("你 好 ， 世 界"), "你好,世界");
        assert_eq!(normalize_translation("使用 Rust 编写"), "使用Rust编写");
        assert_eq!(normalize_translation("中文 。"), "中文.");
        assert_eq!(
            normalize_translation("《三体》 很好看 ！"),
            "\"三体\"很好看!"
        );
        assert_eq!(normalize_translation("等等…"), "等等...");
    }

    #[test]
    fn full_width_forms_fold_to_ascii() {
        assert_eq!(
            normalize_translation("ＡＢＣ１２３（测试）"),
            "ABC123(测试)"
        );
    }

    #[test]
    fn latin_lines_keep_single_spaces() {
        assert_eq!(normalize_translation("Hello   world."), "Hello world.");
        assert_eq!(normalize_translation("  padded  "), "padded");
    }

    #[test]
    fn empty_input_stays_empty() {
        assert_eq!(normalize_translation("   "), "");
    }
}
