// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[allow(dead_code, reason = "the probe only uses the browser's read path")]
#[path = "../src/bookmarks.rs"]
mod bookmarks;

use std::io::{self, Read};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(bookmarks::MAX_BOOKMARK_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let imported = bookmarks::Bookmarks::from_json(&bytes).ok_or("invalid bookmark file")?;
    let original: serde_json::Value = serde_json::from_slice(&bytes)?;
    if serde_json::to_value(imported.entries())? != original {
        return Err("the browser would change or discard imported bookmarks".into());
    }
    Ok(())
}
