use crate::{TranslateError, Translator, http_agent};
use serde_json::Value;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VlmConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
}

pub struct VlmTranslator {
    cfg: VlmConfig,
    agent: ureq::Agent,
}

const SYSTEM_PROMPT: &str = "You are a translation engine. Translate each input line \
into the target language. Reply with exactly the same number of lines as the input, \
one translated line per input line, no numbering, no explanations, no extra blank lines.";

impl VlmTranslator {
    pub fn new(cfg: VlmConfig) -> Self {
        Self {
            cfg,
            agent: http_agent(TIMEOUT),
        }
    }
}

impl Translator for VlmTranslator {
    fn name(&self) -> &'static str {
        "vlm"
    }

    fn translate_lines(&self, lines: &[String]) -> Result<Vec<String>, TranslateError> {
        if lines.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!(
            "{}/chat/completions",
            self.cfg.base_url.trim_end_matches('/')
        );
        let mut req = self
            .agent
            .post(&url)
            .set("Content-Type", "application/json");
        if let Some(key) = &self.cfg.api_key {
            req = req.set("Authorization", &format!("Bearer {key}"));
        }
        let body = serde_json::json!({
            "model": self.cfg.model,
            "temperature": 0,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": lines.join("\n")},
            ]
        });
        let resp = req
            .send_string(&body.to_string())
            .map_err(|e| TranslateError::Backend(format!("vlm http: {e}")))?;
        let json: Value = resp
            .into_json()
            .map_err(|e| TranslateError::Backend(format!("vlm json: {e}")))?;
        let content = json
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .ok_or_else(|| TranslateError::Backend("vlm: unexpected response".into()))?;

        Ok(split_reply(strip_code_fence(content), lines))
    }
}

fn strip_code_fence(content: &str) -> &str {
    let trimmed = content.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let rest = rest
        .lines()
        .next()
        .map_or(rest, |_| rest.find('\n').map_or("", |i| &rest[i + 1..]));
    rest.trim_end().strip_suffix("```").unwrap_or(rest).trim()
}

// Models do not reliably honour the line count. Rather than failing the whole
// backend (and paying for the request anyway), align the reply with the inputs:
// drop trailing blank/extra lines, pad missing ones with the untranslated text.
fn split_reply(content: &str, inputs: &[String]) -> Vec<String> {
    let mut out: Vec<String> = content
        .split('\n')
        .map(|line| line.trim_end().to_owned())
        .collect();
    while out.last().is_some_and(|line| line.is_empty()) {
        out.pop();
    }
    out.truncate(inputs.len());
    for input in &inputs[out.len()..] {
        out.push(input.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_markdown_fence() {
        assert_eq!(strip_code_fence("```\na\nb\n```"), "a\nb");
        assert_eq!(strip_code_fence("```text\na\n```"), "a");
        assert_eq!(strip_code_fence("a\nb"), "a\nb");
    }

    fn inputs(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("src{i}")).collect()
    }

    #[test]
    fn split_reply_keeps_matching_count() {
        assert_eq!(split_reply("a\nb", &inputs(2)), vec!["a", "b"]);
    }

    #[test]
    fn split_reply_truncates_extra_lines() {
        assert_eq!(split_reply("a\nb\nc", &inputs(2)), vec!["a", "b"]);
    }

    #[test]
    fn split_reply_pads_missing_lines_with_source() {
        assert_eq!(
            split_reply("only", &inputs(3)),
            vec!["only", "src1", "src2"]
        );
        assert_eq!(split_reply("", &inputs(1)), vec!["src0"]);
    }

    #[test]
    fn split_reply_ignores_trailing_blank_lines() {
        assert_eq!(split_reply("a\nb\n\n", &inputs(2)), vec!["a", "b"]);
    }
}
