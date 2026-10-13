use crate::{TranslateError, Translator, http_agent};
use serde_json::Value;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
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

        let out = split_reply(strip_code_fence(content), lines.len())?;
        Ok(out)
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

fn split_reply(content: &str, expected: usize) -> Result<Vec<String>, TranslateError> {
    let lines: Vec<String> = content.split('\n').map(str::to_owned).collect();
    if lines.len() == expected {
        Ok(lines)
    } else {
        Err(TranslateError::Backend(format!(
            "vlm: expected {expected} lines, got {}",
            lines.len()
        )))
    }
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

    #[test]
    fn split_reply_checks_count() {
        assert!(split_reply("a\nb", 2).is_ok());
        assert!(split_reply("a\nb\nc", 2).is_err());
        assert!(split_reply("", 1).is_ok());
    }
}
