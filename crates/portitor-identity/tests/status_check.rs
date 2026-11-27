// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The credential status check (TD-20): the identity reads the status
//! lists of the credentials it holds, and the presentation API leaves
//! out those revoked or suspended. Driven in memory: the status lists
//! come from the fake directory, rounds are started by hand.
#![allow(clippy::unwrap_used, missing_docs)]

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use common::*;
use portitor_dcp::credential::Presentation;
use portitor_dcp::status_list::{encode_list, set_bit};
use portitor_identity::status_check::{RoundCounts, run_if_due};
use portitor_identity::store::{DatabaseStore, HeldStatus, IdentityStore};
use portitor_runtime::system::{new_uuid, now_secs, xsd_date_time};
use serde_json::{Value, json};

const REVOCATION_LIST: &str = "https://issuer.example/status/revocation";
const SUSPENSION_LIST: &str = "https://issuer.example/status/suspension";
/// The positions of the two credentials in each list.
const FIRST: u64 = 7;
const SECOND: u64 = 8;

/// A config with the status check on, every minute.
fn checking() -> portitor_identity::config::Config {
    checking_every(60)
}

/// A config with the status check on, every `seconds`. Each round reads
/// the lists fresh (`status_check::cache_seconds`), so tests that run
/// several rounds see what was published between them.
fn checking_every(seconds: u64) -> portitor_identity::config::Config {
    portitor_identity::config::Config {
        credential_status_check_seconds: seconds,
        ..config()
    }
}

/// A VC 2.0 membership from the issuer, for `HOLDER`, with a revocation
/// and a suspension entry at `index`.
fn membership_with_status(issuer: &Party, index: u64) -> String {
    let entry = |purpose: &str, list: &str| {
        json!({
            "id": format!("{list}#{index}"),
            "type": "BitstringStatusListEntry",
            "statusPurpose": purpose,
            "statusListIndex": index.to_string(),
            "statusListCredential": list
        })
    };
    let credential = json!({
        "@context": ["https://www.w3.org/ns/credentials/v2"],
        "id": new_uuid(),
        "type": ["VerifiableCredential", "MembershipCredential"],
        "issuer": issuer.did,
        "validUntil": "2099-01-01T00:00:00Z",
        "credentialSubject": {"id": HOLDER},
        "credentialStatus": [entry("revocation", REVOCATION_LIST), entry("suspension", SUSPENSION_LIST)]
    });
    issuer
        .key
        .sign_json(&issuer.kid(), "vc+jwt", &credential)
        .unwrap()
}

/// The status list credential at `url` for `purpose`, signed by
/// `signer`, with the positions `set`.
fn status_list(signer: &Party, url: &str, purpose: &str, set: &[u64]) -> String {
    let mut bits = vec![0u8; 2048];
    for index in set {
        set_bit(&mut bits, *index).unwrap();
    }
    let credential = json!({
        "@context": ["https://www.w3.org/ns/credentials/v2"],
        "id": url,
        "type": ["VerifiableCredential", "BitstringStatusListCredential"],
        "issuer": signer.did,
        "validFrom": xsd_date_time(now_secs() - 10),
        "credentialSubject": {
            "id": format!("{url}#list"),
            "type": "BitstringStatusList",
            "statusPurpose": purpose,
            "encodedList": encode_list(&bits)
        }
    });
    signer
        .key
        .sign_json(&signer.kid(), "vc+jwt", &credential)
        .unwrap()
}

/// Publishes both lists with the given positions set.
fn publish(world: &World, revoked: &[u64], suspended: &[u64]) {
    let mut lists = world.directory.status_lists.lock().unwrap();
    lists.insert(
        REVOCATION_LIST.to_owned(),
        status_list(&world.issuer, REVOCATION_LIST, "revocation", revoked),
    );
    lists.insert(
        SUSPENSION_LIST.to_owned(),
        status_list(&world.issuer, SUSPENSION_LIST, "suspension", suspended),
    );
}

/// Takes both lists offline: their server answers 503.
fn take_lists_offline(world: &World) {
    world.directory.status_lists.lock().unwrap().clear();
}

/// The world with two memberships delivered, at positions [`FIRST`] and
/// [`SECOND`]; returns their JWTs.
async fn with_two_memberships(world: &World) -> (String, String) {
    let pid = world.request_credentials().await;
    let first = membership_with_status(&world.issuer, FIRST);
    let second = membership_with_status(&world.issuer, SECOND);
    let (status, body) = world
        .deliver(
            &pid,
            &[
                ("vc20-bssl/jwt", "MembershipCredential", &first),
                ("vc20-bssl/jwt", "MembershipCredential", &second),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    (first, second)
}

/// The credential JWTs the identity presents for the membership scope.
async fn presented(world: &World) -> Vec<String> {
    let (status, answer) = world
        .query(MEMBERSHIP_SCOPE, &scope_query(&[MEMBERSHIP_SCOPE]))
        .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let mut jwts = Vec::new();
    for presentation in answer["presentation"].as_array().unwrap() {
        let presentation = Presentation::parse(presentation.as_str().unwrap()).unwrap();
        for credential in &presentation.credentials {
            jwts.push(credential.jwt().to_owned());
        }
    }
    jwts.sort();
    jwts
}

fn sorted(jwts: &[&String]) -> Vec<String> {
    let mut jwts: Vec<String> = jwts.iter().map(|jwt| (*jwt).clone()).collect();
    jwts.sort();
    jwts
}

/// The status recorded for the credential `jwt`.
fn recorded(world: &World, jwt: &str) -> Option<HeldStatus> {
    let credentials = world.identity.store().credentials().unwrap();
    let held = credentials.iter().find(|held| held.jwt == jwt).unwrap();
    held.status.map(|checked| checked.state)
}

/// Lets the lease of the last round end, as a period later: the lease
/// is held until a second before the next tick.
fn end_the_round_lease(world: &World) {
    // deletes every lease that ends within the hour: the round's
    world
        .identity
        .store()
        .forget_ended(now_secs() + 3_600)
        .unwrap();
}

#[tokio::test]
async fn a_revoked_credential_is_no_longer_presented() {
    let world = World::with(&checking());
    let (first, second) = with_two_memberships(&world).await;
    publish(&world, &[FIRST], &[]);

    let counts = run_if_due(&world.identity).await.unwrap();
    assert_eq!(
        counts,
        RoundCounts {
            valid: 1,
            revoked: 1,
            suspended: 0,
            uncheckable: 0,
            unreadable: 0
        }
    );
    assert_eq!(recorded(&world, &first), Some(HeldStatus::Revoked));
    assert_eq!(recorded(&world, &second), Some(HeldStatus::Valid));
    assert_eq!(presented(&world).await, sorted(&[&second]));
    // the management API shows it
    let (status, listed) = call(
        &world.management,
        axum::http::Request::get("/v1/credentials")
            .header("authorization", format!("Bearer {API_KEY}"))
            .body(axum::body::Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let mut shown: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|credential| credential["status"].as_str().unwrap())
        .collect();
    shown.sort_unstable();
    assert_eq!(shown, ["REVOKED", "VALID"]);
    assert!(listed[0]["statusCheckedAt"].is_string());
    let reads = world
        .directory
        .lookups
        .lock()
        .unwrap()
        .iter()
        .filter(|url| *url == REVOCATION_LIST)
        .count();
    assert_eq!(
        reads, 1,
        "one read of the list for both credentials (cache)"
    );
    let checked = world.identity.store().credentials().unwrap();
    assert!(
        checked
            .iter()
            .all(|held| held.status.unwrap().checked_at >= now_secs() - 5),
        "the time of the check is recorded"
    );
}

#[tokio::test]
async fn a_suspended_credential_is_presented_again_once_the_suspension_is_lifted() {
    let world = World::with(&checking_every(1));
    let (first, second) = with_two_memberships(&world).await;
    publish(&world, &[], &[SECOND]);
    run_if_due(&world.identity).await.unwrap();
    assert_eq!(recorded(&world, &second), Some(HeldStatus::Suspended));
    assert_eq!(presented(&world).await, sorted(&[&first]));

    publish(&world, &[], &[]);
    end_the_round_lease(&world);
    let counts = run_if_due(&world.identity).await.unwrap();
    assert_eq!(counts.valid, 2);
    assert_eq!(recorded(&world, &second), Some(HeldStatus::Valid));
    assert_eq!(presented(&world).await, sorted(&[&first, &second]));
}

#[tokio::test]
async fn a_credential_whose_lists_cannot_be_read_keeps_its_last_status() {
    let world = World::with(&checking_every(1));
    let (first, second) = with_two_memberships(&world).await;
    // never checked, lists offline: still presented
    take_lists_offline(&world);
    let counts = run_if_due(&world.identity).await.unwrap();
    assert_eq!(counts.unreadable, 2);
    assert_eq!(recorded(&world, &first), None);
    assert_eq!(presented(&world).await, sorted(&[&first, &second]));

    // checked once (first revoked), then the lists go offline: the last
    // known status stays, the other is still presented
    publish(&world, &[FIRST], &[]);
    end_the_round_lease(&world);
    run_if_due(&world.identity).await.unwrap();
    take_lists_offline(&world);
    end_the_round_lease(&world);
    let counts = run_if_due(&world.identity).await.unwrap();
    assert_eq!(counts.unreadable, 2);
    assert_eq!(recorded(&world, &first), Some(HeldStatus::Revoked));
    assert_eq!(recorded(&world, &second), Some(HeldStatus::Valid));
    assert_eq!(presented(&world).await, sorted(&[&second]));
}

#[tokio::test]
async fn a_list_signed_by_another_party_is_not_believed() {
    let world = World::with(&checking());
    let (first, second) = with_two_memberships(&world).await;
    // the verifier (not the credential's issuer) signs a list that
    // revokes both
    let forged = status_list(
        &world.verifier,
        REVOCATION_LIST,
        "revocation",
        &[FIRST, SECOND],
    );
    world
        .directory
        .status_lists
        .lock()
        .unwrap()
        .insert(REVOCATION_LIST.to_owned(), forged);
    // a verifier refuses such a credential: it is no longer presented,
    // and the management API says why
    let counts = run_if_due(&world.identity).await.unwrap();
    assert_eq!(counts.uncheckable, 2);
    assert_eq!(counts.unreadable, 0);
    assert_eq!(recorded(&world, &first), Some(HeldStatus::Uncheckable));
    assert_eq!(recorded(&world, &second), Some(HeldStatus::Uncheckable));
    assert!(presented(&world).await.is_empty());
}

/// A list that is gone (HTTP 404) is not a passing failure: the
/// credential is recorded as uncheckable and no longer presented, also
/// when it was valid before; once the list is back, it is presented
/// again.
#[tokio::test]
async fn a_credential_whose_list_is_gone_is_no_longer_presented_until_it_is_back() {
    let world = World::with(&checking_every(1));
    let (first, second) = with_two_memberships(&world).await;
    publish(&world, &[], &[]);
    run_if_due(&world.identity).await.unwrap();
    assert_eq!(recorded(&world, &first), Some(HeldStatus::Valid));

    // the suspension list is gone
    world
        .directory
        .status_lists
        .lock()
        .unwrap()
        .remove(SUSPENSION_LIST);
    world
        .directory
        .gone_lists
        .lock()
        .unwrap()
        .push(SUSPENSION_LIST.to_owned());
    end_the_round_lease(&world);
    let counts = run_if_due(&world.identity).await.unwrap();
    assert_eq!(counts.uncheckable, 2);
    assert_eq!(recorded(&world, &first), Some(HeldStatus::Uncheckable));
    assert!(presented(&world).await.is_empty());
    // a second round changes nothing
    end_the_round_lease(&world);
    let counts = run_if_due(&world.identity).await.unwrap();
    assert_eq!(counts.uncheckable, 2);

    world.directory.gone_lists.lock().unwrap().clear();
    publish(&world, &[], &[]);
    end_the_round_lease(&world);
    let counts = run_if_due(&world.identity).await.unwrap();
    assert_eq!(counts.valid, 2);
    assert_eq!(presented(&world).await, sorted(&[&first, &second]));
}

/// An issuer whose DID document cannot be used (here: none) makes its
/// credentials uncheckable: the connector's verifier refuses them too.
#[tokio::test]
async fn a_credential_whose_issuer_cannot_be_resolved_is_uncheckable() {
    let world = World::with(&checking());
    let (first, _second) = with_two_memberships(&world).await;
    publish(&world, &[], &[]);
    world
        .directory
        .documents
        .lock()
        .unwrap()
        .remove(world.issuer.did);
    let counts = run_if_due(&world.identity).await.unwrap();
    assert_eq!(counts.uncheckable, 2);
    assert_eq!(recorded(&world, &first), Some(HeldStatus::Uncheckable));
}

#[tokio::test]
async fn at_zero_the_check_is_off() {
    // the common config has PORTITOR_CREDENTIAL_STATUS_CHECK_SECONDS=0
    let world = World::new();
    let (first, second) = with_two_memberships(&world).await;
    publish(&world, &[FIRST, SECOND], &[]);
    assert_eq!(run_if_due(&world.identity).await, None);
    assert!(portitor_identity::status_check::schedule(&world.identity).is_none());
    assert!(
        !world.directory.looked_up(REVOCATION_LIST),
        "nothing fetched"
    );
    assert_eq!(recorded(&world, &first), None);
    assert_eq!(presented(&world).await, sorted(&[&first, &second]));
}

/// Two replicas on one database: one round per period, by the replica
/// that took the lease; the other leaves it.
#[tokio::test]
async fn two_replicas_run_one_round_per_period() {
    let store: Arc<dyn IdentityStore> = Arc::new(DatabaseStore::in_memory().unwrap());
    let one = World::with_store(&checking(), Arc::clone(&store));
    let other = World::with_store(&checking(), Arc::clone(&store));
    let (first, _second) = with_two_memberships(&one).await;
    // each replica has its own directory; the issuer's keys differ per
    // world, so the lists the first replica can check are its own
    publish(&one, &[FIRST], &[]);
    publish(&other, &[FIRST], &[]);

    assert!(run_if_due(&one.identity).await.is_some());
    assert_eq!(
        run_if_due(&other.identity).await,
        None,
        "the lease is held for the rest of the period"
    );
    assert!(!other.directory.looked_up(REVOCATION_LIST));
    assert_eq!(recorded(&one, &first), Some(HeldStatus::Revoked));
    // the other replica reads the same database: it does not present it
    // either
    let shown: Vec<Value> = other
        .query(MEMBERSHIP_SCOPE, &scope_query(&[MEMBERSHIP_SCOPE]))
        .await
        .1["presentation"]
        .as_array()
        .unwrap()
        .clone();
    let mut jwts = Vec::new();
    for presentation in &shown {
        let presentation = Presentation::parse(presentation.as_str().unwrap()).unwrap();
        for credential in &presentation.credentials {
            jwts.push(credential.jwt().to_owned());
        }
    }
    assert!(!jwts.contains(&first));
    assert_eq!(jwts.len(), 1);
}

/// A credential whose bit is set in both lists is revoked: revocation is
/// for good, and wins over a suspension.
#[tokio::test]
async fn revoked_wins_over_suspended() {
    let world = World::with(&checking());
    let (first, second) = with_two_memberships(&world).await;
    publish(&world, &[FIRST], &[FIRST]);
    let counts = run_if_due(&world.identity).await.unwrap();
    assert_eq!(
        counts,
        RoundCounts {
            valid: 1,
            revoked: 1,
            suspended: 0,
            unreadable: 0
        }
    );
    assert_eq!(recorded(&world, &first), Some(HeldStatus::Revoked));
    assert_eq!(presented(&world).await, sorted(&[&second]));
}

/// Pins today's behaviour: the identity records what the lists say in
/// each round, so a revoked credential whose bit the issuer clears again
/// is recorded valid and presented again. (A bitstring status list's
/// `revocation` is meant to be for good; a verifier reads the list
/// itself, so it would accept it again too.)
#[tokio::test]
async fn a_revoked_credential_whose_bit_is_cleared_is_valid_again() {
    let world = World::with(&checking_every(1));
    let (first, second) = with_two_memberships(&world).await;
    publish(&world, &[FIRST], &[]);
    run_if_due(&world.identity).await.unwrap();
    assert_eq!(recorded(&world, &first), Some(HeldStatus::Revoked));

    publish(&world, &[], &[]);
    end_the_round_lease(&world);
    let counts = run_if_due(&world.identity).await.unwrap();
    assert_eq!(counts.valid, 2);
    assert_eq!(recorded(&world, &first), Some(HeldStatus::Valid));
    assert_eq!(presented(&world).await, sorted(&[&first, &second]));
}

/// The stores of two replicas on one database: one `SQLite` store shared
/// (one process per file), then, with a test server, two connections
/// pools on one new PostgreSQL database.
fn stores_of_two_replicas() -> Vec<(Arc<dyn IdentityStore>, Arc<dyn IdentityStore>)> {
    let shared: Arc<dyn IdentityStore> = Arc::new(DatabaseStore::in_memory().unwrap());
    let mut pairs = vec![(Arc::clone(&shared), shared)];
    if let Some(url) = portitor_runtime::testing::databases::postgres_url() {
        let settings = portitor_runtime::testing::databases::fresh_postgres_settings(&url);
        let setting = portitor_runtime::database::DatabaseSetting::Postgres(Box::new(settings));
        let one: Arc<dyn IdentityStore> = Arc::new(DatabaseStore::open(&setting).unwrap());
        let other: Arc<dyn IdentityStore> = Arc::new(DatabaseStore::open(&setting).unwrap());
        pairs.push((one, other));
    }
    pairs
}

/// Two replicas: once the lease of the first one's round has ended (the
/// next period), the other replica takes it and runs the round; the
/// first one then leaves it.
#[tokio::test]
async fn the_other_replica_runs_the_next_period() {
    for (one_store, other_store) in stores_of_two_replicas() {
        let one = World::with_store(&checking(), one_store);
        let other = World::with_store(&checking(), other_store);
        with_two_memberships(&one).await;
        publish(&one, &[], &[]);
        publish(&other, &[], &[]);

        assert!(run_if_due(&one.identity).await.is_some());
        assert_eq!(run_if_due(&other.identity).await, None);
        assert!(!other.directory.looked_up(REVOCATION_LIST));

        end_the_round_lease(&one);
        assert!(
            run_if_due(&other.identity).await.is_some(),
            "the other replica runs the next period"
        );
        assert!(other.directory.looked_up(REVOCATION_LIST));
        assert_eq!(
            run_if_due(&one.identity).await,
            None,
            "the other replica holds the lease for its period"
        );
    }
}

/// `schedule` runs a round on each tick of its timer, from one period
/// after the start: with the clock paused, tokio moves it on whenever
/// the test waits, so the rounds come at once. Every second, so the
/// lease of a round (kept until a second before the next tick, in the
/// system's seconds) does not stop the next one.
#[tokio::test]
async fn schedule_runs_rounds_on_the_timer() {
    let world = World::with(&checking_every(1));
    let (first, _second) = with_two_memberships(&world).await;
    publish(&world, &[], &[FIRST]);
    tokio::time::pause();
    let schedule = portitor_identity::status_check::schedule(&world.identity).unwrap();
    assert_eq!(
        recorded(&world, &first),
        None,
        "no round before the first tick"
    );

    let suspended = wait_for_status(&world, &first, HeldStatus::Suspended).await;
    assert!(suspended, "a round ran on the timer");
    // the issuer lifts the suspension: a later round sees it
    publish(&world, &[], &[]);
    let valid = wait_for_status(&world, &first, HeldStatus::Valid).await;
    assert!(valid, "a later round ran on the timer");
    schedule.abort();
}

/// Waits, on the paused clock, up to 100 periods of one second for
/// `jwt` to be recorded with `state`: whether it was.
async fn wait_for_status(world: &World, jwt: &str, state: HeldStatus) -> bool {
    for _ in 0..100 {
        if recorded(world, jwt) == Some(state) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    false
}
