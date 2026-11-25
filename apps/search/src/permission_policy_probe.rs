// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[cfg(target_os = "macos")]
use std::{fs, io, path::Path};

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

fn eligible(request: Request<'_>, active: Document<'_>, ordinary: bool) -> Option<String> {
    if !ordinary || !request.main_frame || request.document != active {
        return None;
    }
    let (_, url) = super::preview_url(active.origin)?;
    let canonical = url.origin().ascii_serialization();
    if canonical != active.origin
        || request.top_origin != canonical
        || request.frame_origin != canonical
    {
        return None;
    }
    Some(canonical)
}

fn allowed(rules: &Rules, request: Request<'_>, active: Document<'_>, ordinary: bool) -> bool {
    let Some(canonical) = eligible(request, active, ordinary) else {
        return false;
    };
    !request.capabilities.is_empty()
        && rules.get(&canonical).is_some_and(|grants| {
            request
                .capabilities
                .iter()
                .all(|capability| grants.get(capability) == Some(&true))
        })
}

#[cfg(target_os = "macos")]
const MAX_RULE_BYTES: usize = 64 * 1024;

#[cfg(target_os = "macos")]
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    version: u8,
    rules: Rules,
}

#[cfg(target_os = "macos")]
fn load_policy(path: &Path) -> Option<PolicyFile> {
    let policy: PolicyFile =
        serde_json::from_slice(&super::read_profile_data(path, MAX_RULE_BYTES)?).ok()?;
    (policy.version == 1
        && policy.rules.keys().all(|origin| {
            super::preview_url(origin)
                .is_some_and(|(_, url)| url.origin().ascii_serialization() == *origin)
        }))
    .then_some(policy)
}

#[cfg(target_os = "macos")]
fn allowed_from_profile(
    path: &Path,
    request: Request<'_>,
    active: Document<'_>,
    ordinary: bool,
) -> bool {
    load_policy(path).is_some_and(|policy| allowed(&policy.rules, request, active, ordinary))
}

#[cfg(target_os = "macos")]
fn record_decision(
    path: &Path,
    policy: &mut PolicyFile,
    request: Request<'_>,
    active: Document<'_>,
    ordinary: bool,
    granted: bool,
) -> io::Result<bool> {
    let Some(origin) =
        eligible(request, active, ordinary).filter(|_| !request.capabilities.is_empty())
    else {
        return Ok(false);
    };
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    lock.lock()?;
    let current_rules = if path.try_exists()? {
        load_policy(path).ok_or(io::ErrorKind::InvalidData)?.rules
    } else {
        Rules::new()
    };
    if current_rules != policy.rules {
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    let mut updated = PolicyFile {
        version: 1,
        rules: policy.rules.clone(),
    };
    let site = updated.rules.entry(origin).or_default();
    for capability in request.capabilities {
        site.insert(*capability, granted);
    }
    if serde_json::to_vec(&updated)?.len() > MAX_RULE_BYTES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    super::save_preference(path, &updated)?;
    *policy = updated;
    Ok(true)
}

#[cfg(target_os = "macos")]
#[test]
fn disk_policy_applies_only_current_normal_window_decisions_and_fails_closed() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("permissions.json");
    let origin = "https://example.test";
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
    let mut policy = PolicyFile {
        version: 1,
        rules: Rules::new(),
    };
    assert!(!record_decision(&path, &mut policy, request, doc, false, true).unwrap());
    let reloaded = Document {
        navigation: 2,
        ..doc
    };
    assert!(!record_decision(&path, &mut policy, request, reloaded, true, true).unwrap());
    assert!(!path.exists());
    assert!(record_decision(&path, &mut policy, request, doc, true, true).unwrap());
    let mut other_window = load_policy(&path).unwrap();
    assert!(allowed_from_profile(&path, request, doc, true));
    assert!(record_decision(&path, &mut policy, request, doc, true, false).unwrap());
    assert!(!allowed(
        &load_policy(&path).unwrap().rules,
        request,
        doc,
        true
    ));

    assert_eq!(
        record_decision(&path, &mut other_window, request, doc, true, true)
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert!(!allowed(
        &load_policy(&path).unwrap().rules,
        request,
        doc,
        true
    ));

    assert!(allowed(&other_window.rules, request, doc, true));
    assert!(!allowed_from_profile(&path, request, doc, true));
    fs::remove_file(&path).unwrap();
    assert!(!allowed_from_profile(&path, request, doc, true));
    assert_eq!(
        record_decision(&path, &mut other_window, request, doc, true, true)
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert!(!path.exists());

    let unavailable = path.join("unavailable.json");
    assert!(record_decision(&unavailable, &mut policy, request, doc, true, true).is_err());
    assert!(!allowed(&policy.rules, request, doc, true));

    for malformed in [
        b"{}".as_slice(),
        br#"{"version":2,"rules":{}}"#,
        br#"{"version":1,"rules":{},"unknown":true}"#,
        br#"{"version":1,"rules":{"https://127.0.0.1":{"camera":true}}}"#,
        &vec![b' '; MAX_RULE_BYTES + 1],
    ] {
        std::fs::write(&path, malformed).unwrap();
        assert!(load_policy(&path).is_none());
        assert!(record_decision(&path, &mut policy, request, doc, true, true).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), malformed);
    }
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
