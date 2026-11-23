// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use serde::{Deserialize, Serialize};
use url::Url;

const MAX_ENTRIES: usize = 500;
const MAX_URL_BYTES: usize = 1_024;
const MAX_TITLE_BYTES: usize = 512;

// Allow for JSON escaping as well as the bytes in the decoded strings.
pub(crate) const MAX_BOOKMARK_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Bookmark {
    pub(crate) url: String,
    pub(crate) title: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(transparent)]
pub(crate) struct Bookmarks {
    entries: Vec<Bookmark>,
}

impl Bookmarks {
    pub(crate) fn from_json(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_BOOKMARK_BYTES {
            return None;
        }
        let entries: Vec<Bookmark> = serde_json::from_slice(bytes).ok()?;
        let mut bookmarks = Self::default();
        for entry in entries.into_iter().take(MAX_ENTRIES).rev() {
            if entry.url.len() <= MAX_URL_BYTES
                && !entry
                    .url
                    .chars()
                    .any(|ch| ch.is_control() || ch.is_whitespace())
                && let Ok(url) = Url::parse(&entry.url)
            {
                bookmarks.remove(&url);
                bookmarks.add(&url, &entry.title);
            }
        }
        Some(bookmarks)
    }

    pub(crate) fn entries(&self) -> &[Bookmark] {
        &self.entries
    }

    pub(crate) fn contains(&self, url: &Url) -> bool {
        self.entries.iter().any(|entry| entry.url == url.as_str())
    }

    pub(crate) fn add(&mut self, url: &Url, title: &str) -> bool {
        if self.entries.len() == MAX_ENTRIES
            || self.contains(url)
            || url.scheme() != "https"
            || !url.has_host()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.as_str().len() > MAX_URL_BYTES
        {
            return false;
        }

        let mut title: String = title
            .trim()
            .chars()
            .filter(|ch| !ch.is_control())
            .take(MAX_TITLE_BYTES)
            .collect();
        title.truncate(title.floor_char_boundary(MAX_TITLE_BYTES.min(title.len())));
        self.entries.insert(
            0,
            Bookmark {
                url: url.to_string(),
                title,
            },
        );
        true
    }

    pub(crate) fn remove(&mut self, url: &Url) -> bool {
        let count = self.entries.len();
        self.entries.retain(|entry| entry.url != url.as_str());
        self.entries.len() != count
    }

    /// Returns the new saved state, or None when a bookmark cannot be added.
    pub(crate) fn toggle(&mut self, url: &Url, title: &str) -> Option<bool> {
        if self.remove(url) {
            Some(false)
        } else {
            self.add(url, title).then_some(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_canonical_url_and_round_trips() {
        let mut bookmarks = Bookmarks::default();
        let url = Url::parse("https://EXAMPLE.org:443/path").unwrap();
        assert_eq!(bookmarks.toggle(&url, "Page"), Some(true));
        assert!(bookmarks.contains(&url));
        assert!(!bookmarks.add(&url, "Duplicate"));
        let bytes = serde_json::to_vec(&bookmarks).unwrap();
        let mut restored = Bookmarks::from_json(&bytes).unwrap();
        assert_eq!(restored.entries()[0].url, "https://example.org/path");
        assert_eq!(restored.entries()[0].title, "Page");
        assert_eq!(restored.toggle(&url, "Page"), Some(false));
        assert!(!restored.contains(&url));
    }

    #[test]
    fn caps_collection_without_evicting_existing_bookmarks() {
        let mut bookmarks = Bookmarks::default();
        for index in 0..MAX_ENTRIES {
            let url = Url::parse(&format!("https://example.org/{index}")).unwrap();
            assert!(bookmarks.add(&url, ""));
        }
        let extra = Url::parse("https://example.org/extra").unwrap();
        assert_eq!(bookmarks.toggle(&extra, ""), None);
        assert_eq!(bookmarks.entries().len(), MAX_ENTRIES);
        assert!(bookmarks.contains(&Url::parse("https://example.org/0").unwrap()));
    }

    #[test]
    fn rejects_unsupported_urls_and_bad_data() {
        let mut bookmarks = Bookmarks::default();
        for raw in ["http://example.org/", "https://user:secret@example.org/"] {
            assert_eq!(bookmarks.toggle(&Url::parse(raw).unwrap(), ""), None);
        }
        let oversized = Url::parse(&format!(
            "https://example.org/{}",
            "x".repeat(MAX_URL_BYTES)
        ))
        .unwrap();
        assert_eq!(bookmarks.toggle(&oversized, ""), None);

        let url = Url::parse("https://example.org/").unwrap();
        assert!(bookmarks.add(&url, &"Ж".repeat(MAX_TITLE_BYTES)));
        assert_eq!(bookmarks.entries()[0].title.len(), MAX_TITLE_BYTES);
        assert!(Bookmarks::from_json(b"{").is_none());
        let invalid = br#"[{"url":"https://user:secret@example.org/","title":"bad"}]"#;
        assert!(Bookmarks::from_json(invalid).unwrap().entries().is_empty());
    }
}
