#![deny(missing_docs)]

pub mod google;
pub mod lru;
pub mod mymemory;
pub mod tencent;
pub mod vlm;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TranslateError {
    #[error("translation failed: {0}")]
    Backend(String),
}

pub trait Translator: Send + Sync {
    fn name(&self) -> &'static str;

    fn translate(&self, text: &str) -> Result<String, TranslateError> {
        let one = [text.to_owned()];
        Ok(self.translate_lines(&one)?.pop().unwrap_or_default())
    }

    fn translate_lines(&self, lines: &[String]) -> Result<Vec<String>, TranslateError>;
}

pub struct IdentityTranslator;

impl Translator for IdentityTranslator {
    fn name(&self) -> &'static str {
        "identity"
    }

    fn translate_lines(&self, lines: &[String]) -> Result<Vec<String>, TranslateError> {
        Ok(lines.to_vec())
    }
}

pub struct CachedTranslator<T> {
    inner: T,
    cache: std::sync::Mutex<lru::LruCache>,
}

impl<T: Translator> CachedTranslator<T> {
    pub fn new(inner: T, cap: usize) -> Self {
        Self {
            inner,
            cache: std::sync::Mutex::new(lru::LruCache::new(cap)),
        }
    }

    pub fn len(&self) -> usize {
        self.cache.lock().map(|c| c.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: Translator> Translator for CachedTranslator<T> {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn translate_lines(&self, lines: &[String]) -> Result<Vec<String>, TranslateError> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| TranslateError::Backend("translation cache poisoned".into()))?;

        let mut out = vec![String::new(); lines.len()];
        let mut misses = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            match cache.get(line) {
                Some(hit) => out[i] = hit.to_owned(),
                None => misses.push(i),
            }
        }

        if !misses.is_empty() {
            let inputs: Vec<String> = misses.iter().map(|&i| lines[i].clone()).collect();
            let translated = self.inner.translate_lines(&inputs)?;
            for (j, &i) in misses.iter().enumerate() {
                cache.put(inputs[j].clone(), translated[j].clone());
                out[i] = translated[j].clone();
            }
        }
        Ok(out)
    }
}

pub struct FallbackTranslator {
    backends: Vec<Box<dyn Translator>>,
}

impl FallbackTranslator {
    pub fn new(backends: Vec<Box<dyn Translator>>) -> Self {
        Self { backends }
    }
}

impl Translator for FallbackTranslator {
    fn name(&self) -> &'static str {
        "fallback"
    }

    fn translate_lines(&self, lines: &[String]) -> Result<Vec<String>, TranslateError> {
        let mut last = TranslateError::Backend("no translation backends configured".into());
        for backend in &self.backends {
            match backend.translate_lines(lines) {
                Ok(out) => return Ok(out),
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}

pub(crate) fn http_agent(timeout: std::time::Duration) -> ureq::Agent {
    let mut builder = ureq::AgentBuilder::new()
        .timeout(timeout)
        .timeout_connect(std::time::Duration::from_secs(3));
    if let Some(url) = env_proxy_url() {
        if let Ok(proxy) = ureq::Proxy::new(&url) {
            builder = builder.proxy(proxy);
        }
    }
    builder.build()
}

fn env_proxy_url() -> Option<String> {
    ["ALL_PROXY", "all_proxy", "HTTPS_PROXY", "https_proxy"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .map(|v| v.trim().to_owned())
        .find(|v| !v.is_empty())
}

pub(crate) struct Cooldown(std::sync::Mutex<Option<std::time::Instant>>);

impl Cooldown {
    pub fn new() -> Self {
        Self(std::sync::Mutex::new(None))
    }

    pub fn active(&self) -> bool {
        self.0
            .lock()
            .map(|g| g.is_some_and(|t| std::time::Instant::now() < t))
            .unwrap_or(false)
    }

    pub fn trip(&self, duration: std::time::Duration) {
        if let Ok(mut g) = self.0.lock() {
            *g = Some(std::time::Instant::now() + duration);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_preserves_lines() {
        let lines = vec!["hello".to_owned(), "世界".to_owned()];
        assert_eq!(IdentityTranslator.translate_lines(&lines).unwrap(), lines);
    }

    #[test]
    fn cached_translator_hits_cache_and_batches_misses() {
        let cached = CachedTranslator::new(IdentityTranslator, 8);
        let lines = vec!["a".to_owned(), "b".to_owned()];
        assert_eq!(cached.translate_lines(&lines).unwrap(), lines);
        assert_eq!(cached.translate_lines(&lines).unwrap(), lines);
        assert_eq!(cached.len(), 2);
    }

    #[test]
    fn fallback_uses_first_healthy_backend() {
        struct Fail;
        impl Translator for Fail {
            fn name(&self) -> &'static str {
                "fail"
            }
            fn translate_lines(&self, _: &[String]) -> Result<Vec<String>, TranslateError> {
                Err(TranslateError::Backend("down".into()))
            }
        }
        let chain = FallbackTranslator::new(vec![Box::new(Fail), Box::new(IdentityTranslator)]);
        let lines = vec!["x".to_owned()];
        assert_eq!(chain.translate_lines(&lines).unwrap(), lines);
    }

    #[test]
    fn fallback_reports_last_error_when_all_fail() {
        let chain = FallbackTranslator::new(vec![]);
        let lines = vec!["x".to_owned()];
        assert!(chain.translate_lines(&lines).is_err());
    }
}
