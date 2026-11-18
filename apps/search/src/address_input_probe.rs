// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use url::{Host, Url};

// Classify input for research only. A web address still needs a separate
// navigation policy. Search text stays local; this module has no provider.
#[derive(Debug, PartialEq, Eq)]
enum Input<'a> {
    Empty,
    WebAddress(Url),
    Search(&'a str),
    InvalidAddress,
}

fn classify(raw: &str) -> Input<'_> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Input::Empty;
    }
    if raw.chars().any(|char| char.is_control() || char == '\\') {
        return Input::InvalidAddress;
    }

    let token = raw.split_whitespace().next().unwrap_or_default();
    let first = token.split(['/', '?', '#']).next().unwrap_or_default();
    let explicit = raw.contains("://");
    let host_candidate = first.split(':').next().unwrap_or_default();
    let bare_host = host_candidate.contains('.')
        || host_candidate.eq_ignore_ascii_case("localhost")
        || first.starts_with('[');
    let has_scheme = raw.split_once(':').is_some_and(|(prefix, _)| {
        !prefix.is_empty()
            && prefix.starts_with(|c: char| c.is_ascii_alphabetic())
            && prefix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"+-.".contains(&byte))
    });

    if token.contains('@') {
        return Input::InvalidAddress;
    }
    if !explicit && !bare_host && !has_scheme {
        return Input::Search(raw);
    }
    if raw.chars().any(char::is_whitespace) {
        return if explicit || has_scheme || (bare_host && token.contains(['/', '?', '#', ':'])) {
            Input::InvalidAddress
        } else {
            Input::Search(raw)
        };
    }
    if !explicit && !bare_host {
        return Input::InvalidAddress;
    }

    let parsed = if explicit {
        let Some((scheme, rest)) = raw.split_once("://") else {
            return Input::InvalidAddress;
        };
        if (!scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https"))
            || rest.is_empty()
            || rest.starts_with(['/', '?', '#'])
        {
            return Input::InvalidAddress;
        }
        Url::parse(raw)
    } else {
        Url::parse(&format!("https://{raw}"))
    };
    match parsed {
        Ok(url)
            if matches!(url.scheme(), "http" | "https")
                && url.host().is_some_and(|host| match host {
                    Host::Domain(name) if !explicit => {
                        name.eq_ignore_ascii_case("localhost") || name.contains('.')
                    }
                    _ => true,
                })
                && url.username().is_empty()
                && url.password().is_none() =>
        {
            Input::WebAddress(url)
        }
        _ => Input::InvalidAddress,
    }
}

#[test]
fn web_addresses_are_normalized_without_deciding_navigation_policy() {
    for (input, output) in [
        (" HTTPS://EXAMPLE.ORG:443/docs ", "https://example.org/docs"),
        ("example.org/docs", "https://example.org/docs"),
        ("пример.рф", "https://xn--e1afmkfd.xn--p1ai/"),
        ("http://localhost:43127/", "http://localhost:43127/"),
        ("localhost:43127", "https://localhost:43127/"),
        ("[::1]:1234", "https://[::1]:1234/"),
    ] {
        let Input::WebAddress(url) = classify(input) else {
            panic!("expected web address for {input}");
        };
        assert_eq!(url.as_str(), output);
    }
}

#[test]
fn unsafe_or_malformed_address_forms_do_not_turn_into_search_queries() {
    for input in [
        "file:///tmp/test",
        "javascript:alert(1)",
        "https:example.org",
        "https://",
        "https:///example.org",
        "https://a.test:99999/",
        "https://name:secret@example.org/",
        "name@example.org",
        "name@example.org/docs more",
        "example.org/one two",
        "example.org:443 extra",
        "example.org?q=one two",
        "https://exam\nple.org/",
        r"https://example.org\docs",
        "https://example.org/one two",
    ] {
        assert_eq!(classify(input), Input::InvalidAddress, "{input}");
    }
}

#[test]
fn search_text_is_kept_local_and_empty_input_is_distinct() {
    for query in [
        "книга по Rust",
        "Cargo.toml features",
        "rust",
        "site search",
    ] {
        assert_eq!(classify(query), Input::Search(query));
    }
    assert_eq!(classify("   "), Input::Empty);
}
