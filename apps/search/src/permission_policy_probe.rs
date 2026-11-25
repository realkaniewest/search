// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
enum Capability {
    Camera,
    Microphone,
    Geolocation,
}

type Rules = BTreeMap<String, BTreeMap<Capability, bool>>;

#[derive(Clone, Copy, Eq, PartialEq)]
struct Document<'a> {
    view: u64,
    navigation: u64,
    origin: &'a str,
}

#[derive(Clone, Copy)]
struct Request<'a> {
    document: Document<'a>,
    top_origin: &'a str,
    frame_origin: &'a str,
    main_frame: bool,
    capabilities: &'a [Capability],
}

fn allowed(rules: &Rules, request: Request<'_>, active: Document<'_>, ordinary: bool) -> bool {
    if !ordinary || !request.main_frame || request.document != active {
        return false;
    }
    let Some((_, url)) = super::preview_url(active.origin) else {
        return false;
    };
    let canonical = url.origin().ascii_serialization();
    if canonical != active.origin
        || request.top_origin != canonical
        || request.frame_origin != canonical
    {
        return false;
    }
    !request.capabilities.is_empty()
        && rules.get(&canonical).is_some_and(|grants| {
            request
                .capabilities
                .iter()
                .all(|capability| grants.get(capability) == Some(&true))
        })
}

#[test]
fn stored_grant_does_not_transfer_between_capabilities_or_origins() {
    let origin = "https://example.test";
    let rules: Rules =
        BTreeMap::from([(origin.into(), BTreeMap::from([(Capability::Camera, true)]))]);
    let serialized = serde_json::to_vec(&rules).unwrap();
    let mut restored = serde_json::from_slice::<Rules>(&serialized).unwrap();
    let doc = Document {
        view: 1,
        navigation: 1,
        origin,
    };
    let request = Request {
        document: doc,
        top_origin: origin,
        frame_origin: origin,
        main_frame: true,
        capabilities: &[Capability::Camera],
    };
    assert!(allowed(&restored, request, doc, true));
    for denied in [
        Request {
            capabilities: &[Capability::Microphone],
            ..request
        },
        Request {
            capabilities: &[Capability::Geolocation],
            ..request
        },
        Request {
            top_origin: "https://other.test",
            ..request
        },
        Request {
            frame_origin: "https://example.test:444",
            ..request
        },
        Request {
            main_frame: false,
            ..request
        },
    ] {
        assert!(!allowed(&restored, denied, doc, true));
    }
    let combined = Request {
        capabilities: &[Capability::Camera, Capability::Microphone],
        ..request
    };
    assert!(!allowed(&restored, combined, doc, true));
    restored
        .get_mut(origin)
        .unwrap()
        .insert(Capability::Microphone, true);
    assert!(allowed(&restored, combined, doc, true));
    assert!(!allowed(&restored, request, doc, false));
    restored
        .get_mut(origin)
        .unwrap()
        .insert(Capability::Camera, false);
    let revoked = serde_json::from_slice::<Rules>(&serde_json::to_vec(&restored).unwrap()).unwrap();
    assert!(!allowed(&revoked, request, doc, true));
}

#[test]
fn stale_prompt_cannot_grant_access_after_reload_or_view_change() {
    let origin = "https://example.test";
    let rules = BTreeMap::from([(origin.into(), BTreeMap::from([(Capability::Camera, true)]))]);
    let doc = Document {
        view: 1,
        navigation: 1,
        origin,
    };
    let request = Request {
        document: doc,
        top_origin: origin,
        frame_origin: origin,
        main_frame: true,
        capabilities: &[Capability::Camera],
    };
    for new_document in [
        Document {
            navigation: 2,
            ..doc
        },
        Document { view: 2, ..doc },
        Document {
            origin: "https://other.test",
            ..doc
        },
    ] {
        assert!(!allowed(&rules, request, new_document, true));
    }
}

#[test]
fn even_a_saved_grant_cannot_make_an_unsupported_origin_eligible() {
    for origin in [
        "http://example.test",
        "https://127.0.0.1",
        "https://example.test/path",
        "https://example.test:444",
    ] {
        let doc = Document {
            view: 1,
            navigation: 1,
            origin,
        };
        let rules = BTreeMap::from([(origin.into(), BTreeMap::from([(Capability::Camera, true)]))]);
        let request = Request {
            document: doc,
            top_origin: origin,
            frame_origin: origin,
            main_frame: true,
            capabilities: &[Capability::Camera],
        };
        assert!(!allowed(&rules, request, doc, true));
    }
}
