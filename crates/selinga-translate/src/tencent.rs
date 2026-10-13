use crate::{Cooldown, TranslateError, Translator, http_agent};
use serde_json::Value;
use std::time::Duration;

const ENDPOINT: &str = "https://transmart.qq.com/api/imt";
const TIMEOUT: Duration = Duration::from_secs(10);
const COOLDOWN: Duration = Duration::from_secs(60);
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/126.0.0.0";
const REFERER: &str = "https://transmart.qq.com/";
const CLIENT_KEY: &str = "browser-chrome-126-windows";

pub struct TransmartTranslator {
    target: String,
    agent: ureq::Agent,
    cooldown: Cooldown,
}

impl TransmartTranslator {
    pub fn new(target: &str) -> Self {
        Self {
            target: target.to_owned(),
            agent: http_agent(TIMEOUT),
            cooldown: Cooldown::new(),
        }
    }

    fn request(&self, lines: &[String]) -> Result<Vec<String>, TranslateError> {
        let body = serde_json::json!({
            "header": {
                "fn": "auto_translation",
                "session": "",
                "client_key": CLIENT_KEY,
                "user": "",
            },
            "source": {"text_list": lines, "lang": "auto"},
            "target": {"lang": map_lang(&self.target)},
            "type": "plain",
            "model_category": "normal",
        });
        let resp = self
            .agent
            .post(ENDPOINT)
            .set("Content-Type", "application/json")
            .set("User-Agent", USER_AGENT)
            .set("Referer", REFERER)
            .send_string(&body.to_string())
            .map_err(|e| {
                self.cooldown.trip(COOLDOWN);
                TranslateError::Backend(format!("transmart http: {e}"))
            })?;
        let json: Value = resp
            .into_json()
            .map_err(|e| TranslateError::Backend(format!("transmart json: {e}")))?;
        parse_reply(&json)
    }
}

impl Translator for TransmartTranslator {
    fn name(&self) -> &'static str {
        "transmart"
    }

    fn translate_lines(&self, lines: &[String]) -> Result<Vec<String>, TranslateError> {
        if lines.is_empty() {
            return Ok(Vec::new());
        }
        if self.cooldown.active() {
            return Err(TranslateError::Backend("transmart: cooling down".into()));
        }
        let out = self.request(lines)?;
        if out.len() == lines.len() {
            Ok(out)
        } else {
            Err(TranslateError::Backend(format!(
                "transmart: expected {} lines, got {}",
                lines.len(),
                out.len()
            )))
        }
    }
}

fn map_lang(bcp47: &str) -> String {
    bcp47
        .split(['-', '_'])
        .next()
        .unwrap_or("en")
        .to_ascii_lowercase()
}

fn parse_reply(json: &Value) -> Result<Vec<String>, TranslateError> {
    let ret = json
        .pointer("/header/ret_code")
        .and_then(Value::as_str)
        .unwrap_or("missing");
    if ret != "succ" {
        let message = json
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("(no message)");
        return Err(TranslateError::Backend(format!(
            "transmart: ret_code {ret}: {message}"
        )));
    }
    json.get("auto_translation")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|v| v.as_str().unwrap_or_default().to_owned())
                .collect()
        })
        .ok_or_else(|| TranslateError::Backend("transmart: unexpected response".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_success_reply() {
        let json: Value = serde_json::from_str(
            r#"{"header":{"ret_code":"succ"},"auto_translation":["你好世界","你好"],"src_lang":"en"}"#,
        )
        .unwrap();
        assert_eq!(
            parse_reply(&json).unwrap(),
            vec!["你好世界".to_owned(), "你好".to_owned()]
        );
    }

    #[test]
    fn rejects_error_reply() {
        let json: Value = serde_json::from_str(
            r#"{"header":{"ret_code":"error"},"message":"Invalid value of source.text_list"}"#,
        )
        .unwrap();
        assert!(parse_reply(&json).is_err());
    }

    #[test]
    fn maps_language_codes() {
        assert_eq!(map_lang("zh-CN"), "zh");
        assert_eq!(map_lang("en-US"), "en");
        assert_eq!(map_lang("ja-JP"), "ja");
    }
}
