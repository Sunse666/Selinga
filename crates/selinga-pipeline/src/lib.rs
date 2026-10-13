#![deny(missing_docs)]

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
