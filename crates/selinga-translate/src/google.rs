use crate::{Cooldown, TranslateError, Translator, http_agent};
use serde_json::Value;
use std::time::Duration;

const ENDPOINT: &str = "https://translate.googleapis.com/translate_a/single";
const TIMEOUT: Duration = Duration::from_secs(4);
const COOLDOWN: Duration = Duration::from_secs(60);

pub struct GoogleTranslator {
    target: String,
    agent: ureq::Agent,
    cooldown: Cooldown,
}

impl GoogleTranslator {
    pub fn new(target: &str) -> Self {
        Self {
            target: target.to_owned(),
            agent: http_agent(TIMEOUT),
            cooldown: Cooldown::new(),
        }
    }

    fn request(&self, text: &str) -> Result<String, TranslateError> {
        let resp = self
            .agent
            .post(ENDPOINT)
            .query("client", "gtx")
            .query("sl", "auto")
            .query("tl", &self.target)
            .query("dt", "t")
            .set("Content-Type", "application/x-www-form-urlencoded")
            .send_string(&format!("q={}", urlencode(text)))
            .map_err(|e| {
                self.cooldown.trip(COOLDOWN);
                TranslateError::Backend(format!("google http: {e}"))
            })?;
        let json: Value = resp
            .into_json()
            .map_err(|e| TranslateError::Backend(format!("google json: {e}")))?;
        parse_translated(&json)
    }
}

impl Translator for GoogleTranslator {
    fn name(&self) -> &'static str {
        "google"
    }

    fn translate_lines(&self, lines: &[String]) -> Result<Vec<String>, TranslateError> {
        if lines.is_empty() {
            return Ok(Vec::new());
        }
        if self.cooldown.active() {
            return Err(TranslateError::Backend("google: cooling down".into()));
        }
        match self.request(&lines.join("\n")) {
            Ok(merged) => {
                let split: Vec<String> = merged.split('\n').map(str::to_owned).collect();
                if split.len() == lines.len() {
                    return Ok(split);
                }
                lines.iter().map(|l| self.request(l)).collect()
            }
            Err(e) => Err(e),
        }
    }
}

fn parse_translated(json: &Value) -> Result<String, TranslateError> {
    let segments = json
        .get(0)
        .and_then(Value::as_array)
        .ok_or_else(|| TranslateError::Backend("google: unexpected response".into()))?;
    let mut out = String::new();
    for segment in segments {
        let piece = segment
            .get(0)
            .and_then(Value::as_str)
            .ok_or_else(|| TranslateError::Backend("google: bad segment".into()))?;
        out.push_str(piece);
    }
    Ok(out)
}

fn urlencode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gtx_response() {
        let json: Value = serde_json::from_str(
            r#"[[["你好，","hello",null,null,10],["世界","world",null,null,10]],null,"en"]"#,
        )
        .unwrap();
        assert_eq!(parse_translated(&json).unwrap(), "你好，世界");
    }

    #[test]
    fn parses_response_with_newlines() {
        let json: Value = serde_json::from_str(
            r#"[[["你好\n","hello\n",null,null,10],["世界","world",null,null,10]],null,"en"]"#,
        )
        .unwrap();
        assert_eq!(parse_translated(&json).unwrap(), "你好\n世界");
    }

    #[test]
    fn rejects_wrong_shape() {
        let json: Value = serde_json::from_str(r#"{"error": "bad"}"#).unwrap();
        assert!(parse_translated(&json).is_err());
    }

    #[test]
    fn urlencodes_specials() {
        assert_eq!(urlencode("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
    }
}
