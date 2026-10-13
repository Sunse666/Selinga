use crate::{TranslateError, Translator, http_agent};
use serde_json::Value;
use std::time::Duration;

const ENDPOINT: &str = "https://api.mymemory.translated.net/get";
const TIMEOUT: Duration = Duration::from_secs(10);

pub struct MyMemoryTranslator {
    source: String,
    target: String,
    agent: ureq::Agent,
}

impl MyMemoryTranslator {
    pub fn new(source: &str, target: &str) -> Self {
        Self {
            source: source.to_owned(),
            target: target.to_owned(),
            agent: http_agent(TIMEOUT),
        }
    }

    fn request(&self, text: &str) -> Result<String, TranslateError> {
        let resp = self
            .agent
            .get(ENDPOINT)
            .query("q", text)
            .query("langpair", &format!("{}|{}", self.source, self.target))
            .call()
            .map_err(|e| TranslateError::Backend(format!("mymemory http: {e}")))?;
        let json: Value = resp
            .into_json()
            .map_err(|e| TranslateError::Backend(format!("mymemory json: {e}")))?;
        parse_reply(&json)
    }
}

impl Translator for MyMemoryTranslator {
    fn name(&self) -> &'static str {
        "mymemory"
    }

    fn translate_lines(&self, lines: &[String]) -> Result<Vec<String>, TranslateError> {
        if lines.is_empty() {
            return Ok(Vec::new());
        }
        let results: Vec<Result<String, TranslateError>> = std::thread::scope(|s| {
            let handles: Vec<_> = lines
                .iter()
                .map(|l| s.spawn(move || self.request(l)))
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or_else(|_| {
                        Err(TranslateError::Backend("mymemory worker panicked".into()))
                    })
                })
                .collect()
        });
        results.into_iter().collect()
    }
}

fn parse_reply(json: &Value) -> Result<String, TranslateError> {
    let status = json.get("responseStatus").and_then(Value::as_i64);
    if status != Some(200) {
        return Err(TranslateError::Backend(format!(
            "mymemory: responseStatus {status:?}"
        )));
    }
    if json.get("quotaFinished").and_then(Value::as_bool) == Some(true) {
        return Err(TranslateError::Backend(
            "mymemory: daily quota used up".into(),
        ));
    }
    json.pointer("/responseData/translatedText")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| TranslateError::Backend("mymemory: unexpected response".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_success_reply() {
        let json: Value = serde_json::from_str(
            r#"{"responseData":{"translatedText":"你好，世界","match":1},"quotaFinished":false,"responseStatus":200}"#,
        )
        .unwrap();
        assert_eq!(parse_reply(&json).unwrap(), "你好，世界");
    }

    #[test]
    fn rejects_quota_exhausted() {
        let json: Value = serde_json::from_str(
            r#"{"responseData":{"translatedText":"LIMIT REACHED"},"quotaFinished":true,"responseStatus":200}"#,
        )
        .unwrap();
        assert!(parse_reply(&json).is_err());
    }

    #[test]
    fn rejects_error_status() {
        let json: Value =
            serde_json::from_str(r#"{"responseData":{"translatedText":""},"responseStatus":403}"#)
                .unwrap();
        assert!(parse_reply(&json).is_err());
    }
}
