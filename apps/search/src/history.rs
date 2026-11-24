// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use serde::{Deserialize, Serialize};
use url::Url;

const MAX_ENTRIES: usize = 500;
const MAX_URL_BYTES: usize = 1_024;
const MAX_TITLE_BYTES: usize = 512;

// Include room for JSON escaping as well as the raw string lengths.
pub(crate) const MAX_HISTORY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct HistoryEntry {
    pub(crate) url: String,
    pub(crate) title: String,
    /// Seconds since the Unix epoch.
    pub(crate) visited_at: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(transparent)]
pub(crate) struct History {
    entries: Vec<HistoryEntry>,
}

impl History {
    pub(crate) fn from_json(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_HISTORY_BYTES {
            return None;
        }
        let entries: Vec<HistoryEntry> = serde_json::from_slice(bytes).ok()?;
        let mut history = Self::default();
        // The file is ordered newest first. Replay only the bounded prefix in
        // reverse to preserve the latest copy of duplicate addresses.
        for entry in entries.into_iter().take(MAX_ENTRIES).rev() {
            if entry.url.len() <= MAX_URL_BYTES
                && !entry
                    .url
                    .chars()
                    .any(|ch| ch.is_control() || ch.is_whitespace())
                && let Ok(url) = Url::parse(&entry.url)
            {
                history.record(&url, &entry.title, entry.visited_at);
            }
        }
        Some(history)
    }

    /// Record a committed top-level visit. Callers also enforce their active
    /// navigation and private browsing policies before recording a visit.
    pub(crate) fn record(&mut self, url: &Url, title: &str, visited_at: u64) -> bool {
        if url.scheme() != "https"
            || !url.has_host()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.as_str().len() > MAX_URL_BYTES
        {
            return false;
        }
        let mut title = title
            .trim()
            .chars()
            .filter(|ch| !ch.is_control())
            .take(MAX_TITLE_BYTES)
            .collect::<String>();
        title.truncate(title.floor_char_boundary(MAX_TITLE_BYTES.min(title.len())));
        let url = url.to_string();
        self.entries.retain(|entry| entry.url != url);
        self.entries.insert(
            0,
            HistoryEntry {
                url,
                title,
                visited_at,
            },
        );
        self.entries.truncate(MAX_ENTRIES);
        true
    }

    pub(crate) fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    pub(crate) fn remove(&mut self, url: &Url) -> bool {
        let count = self.entries.len();
        self.entries.retain(|entry| entry.url != url.as_str());
        self.entries.len() != count
    }

    pub(crate) fn clear(&mut self) -> bool {
        let changed = !self.entries.is_empty();
        self.entries.clear();
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_upserts_and_limits_entries() {
        let first = Url::parse("https://example.org/first").unwrap();
        let mut history = History::default();
        assert!(history.record(&first, "Original", 0));
        for index in 0..MAX_ENTRIES {
            let url = Url::parse(&format!("https://example.org/{index}")).unwrap();
            assert!(history.record(&url, "", index as u64 + 1));
        }
        assert_eq!(history.entries().len(), MAX_ENTRIES);
        assert!(!history.remove(&first));

        let latest = Url::parse(&history.entries()[0].url).unwrap();
        assert!(history.record(&latest, "Updated", 501));
        assert_eq!(history.entries().len(), MAX_ENTRIES);
        assert_eq!(history.entries()[0].title, "Updated");
        assert_eq!(history.entries()[0].visited_at, 501);
        assert!(history.remove(&latest));
        assert!(history.clear());
        assert!(!history.clear());
    }

    #[test]
    fn history_round_trip_rejects_credentials_and_invalid_addresses() {
        let mut history = History::default();
        for raw in ["http://example.org/", "https://user:secret@example.org/"] {
            assert!(!history.record(&Url::parse(raw).unwrap(), "", 0));
        }
        let oversized = Url::parse(&format!(
            "https://example.org/{}",
            "x".repeat(MAX_URL_BYTES)
        ))
        .unwrap();
        assert!(!history.record(&oversized, "", 0));

        let url = Url::parse("https://example.org/").unwrap();
        assert!(history.record(&url, &"Ж".repeat(MAX_TITLE_BYTES), 7));
        let bytes = serde_json::to_vec(&history).unwrap();
        let restored = History::from_json(&bytes).unwrap();
        assert_eq!(restored.entries()[0].url, url.as_str());
        assert_eq!(restored.entries()[0].title.len(), MAX_TITLE_BYTES);
        assert!(History::from_json(b"{").is_none());

        let invalid =
            br#"[{"url":"https://user:secret@example.org/","title":"bad","visited_at":1}]"#;
        assert!(History::from_json(invalid).unwrap().entries().is_empty());
    }
}
