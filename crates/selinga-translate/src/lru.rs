use std::collections::HashMap;

pub struct LruCache {
    cap: usize,
    seq: u64,
    map: HashMap<String, (String, u64)>,
}

impl LruCache {
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            seq: 0,
            map: HashMap::new(),
        }
    }

    pub fn get(&mut self, key: &str) -> Option<&str> {
        self.seq += 1;
        let stamp = self.seq;
        self.map.get_mut(key).map(|(value, s)| {
            *s = stamp;
            value.as_str()
        })
    }

    pub fn put(&mut self, key: String, value: String) {
        self.seq += 1;
        let stamp = self.seq;
        if !self.map.contains_key(&key) && self.map.len() >= self.cap {
            let victim = self
                .map
                .iter()
                .min_by_key(|(_, (_, s))| *s)
                .map(|(k, _)| k.clone());
            if let Some(victim) = victim {
                self.map.remove(&victim);
            }
        }
        self.map.insert(key, (value, stamp));
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_roundtrip() {
        let mut c = LruCache::new(2);
        assert!(c.is_empty());
        c.put("a".into(), "1".into());
        assert_eq!(c.get("a"), Some("1"));
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn evicts_least_recently_used() {
        let mut c = LruCache::new(2);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into());
        assert_eq!(c.get("a"), Some("1"));
        c.put("c".into(), "3".into());
        assert_eq!(c.get("a"), Some("1"));
        assert_eq!(c.get("b"), None);
        assert_eq!(c.get("c"), Some("3"));
    }

    #[test]
    fn overwrite_updates_value() {
        let mut c = LruCache::new(2);
        c.put("a".into(), "1".into());
        c.put("a".into(), "2".into());
        assert_eq!(c.get("a"), Some("2"));
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn capacity_never_exceeded() {
        let mut c = LruCache::new(3);
        for i in 0..50 {
            c.put(format!("k{i}"), format!("v{i}"));
        }
        assert_eq!(c.len(), 3);
    }
}
