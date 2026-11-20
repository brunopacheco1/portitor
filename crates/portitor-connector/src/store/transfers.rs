// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Transfers as provider, and the agreements they are made under, in the
//! same `SQLite` file as the negotiations (ADR 0013).

use portitor_dsp::negotiation::provider::{Negotiation, NegotiationStore};
use portitor_dsp::negotiation::{Agreement, NegotiationState};
use portitor_dsp::transfer::provider::{Agreements, Outgoing, Transfer, TransferStore};
use portitor_dsp::transfer::{TransferState, Transition};
use rusqlite::{Row, params};

use super::{
    NEGOTIATION_COLUMNS, SqliteNegotiations, StoreError, StoredMessage, col, err, json,
    negotiation_from_row, parsed, parsed_version, state_text, version,
};

const TRANSFER_COLUMNS: &str = "provider_pid, consumer_pid, state, callback_address, \
     counter_party, agreement_id, dataset_id, format, pending, version";

/// A transfer state as stored: its DSP name, for example `STARTED`.
pub(super) fn transfer_state_text(s: TransferState) -> Result<String, StoreError> {
    match serde_json::to_value(s) {
        Ok(serde_json::Value::String(text)) => Ok(text),
        _ => Err(StoreError(format!("cannot store state {s:?}"))),
    }
}

pub(super) fn parsed_transfer_state(text: String) -> Result<TransferState, StoreError> {
    serde_json::from_value(serde_json::Value::String(text))
        .map_err(|e| StoreError(format!("stored state: {e}")))
}

/// The pending message as stored in the `pending` column.
fn pending_text(o: Option<&Outgoing>) -> Result<Option<String>, StoreError> {
    let Some(o) = o else { return Ok(None) };
    let stored = StoredMessage::new(
        &o.url,
        &o.status_url,
        &o.body,
        o.transition.name(),
        o.to,
        false,
    );
    json(&stored).map(Some)
}

/// The pending message read back from the `pending` column; it goes to
/// the transfer's `counter_party`.
fn pending_message(
    provider_pid: &str,
    counter_party: &str,
    text: &str,
) -> Result<Outgoing, StoreError> {
    let m: StoredMessage<TransferState> = parsed("pending message", text)?;
    let transition = Transition::from_name(&m.transition)
        .ok_or_else(|| StoreError(format!("stored transition {}", m.transition)))?;
    Ok(Outgoing {
        provider_pid: provider_pid.to_owned(),
        url: m.url,
        body: m.body,
        transition,
        to: m.to,
        status_url: m.status_url,
        counter_party: counter_party.to_owned(),
    })
}

fn transfer_from_row(row: &Row<'_>) -> Result<Transfer, StoreError> {
    let provider_pid: String = col(row, 0)?;
    let counter_party: String = col(row, 4)?;
    let pending = match col::<Option<String>>(row, 8)? {
        Some(text) => Some(pending_message(&provider_pid, &counter_party, &text)?),
        None => None,
    };
    Ok(Transfer {
        consumer_pid: col(row, 1)?,
        state: parsed_transfer_state(col(row, 2)?)?,
        callback_address: col(row, 3)?,
        counter_party,
        agreement_id: col(row, 5)?,
        dataset_id: col(row, 6)?,
        format: col(row, 7)?,
        pending,
        version: parsed_version(col(row, 9)?)?,
        provider_pid,
    })
}

impl TransferStore for SqliteNegotiations {
    fn get(&self, provider_pid: &str) -> Result<Option<Transfer>, StoreError> {
        let sql = format!("SELECT {TRANSFER_COLUMNS} FROM transfer WHERE provider_pid = ?1");
        let found = self.query(&sql, params![provider_pid], transfer_from_row)?;
        Ok(found.into_iter().next())
    }

    fn requested(
        &self,
        counter_party: &str,
        consumer_pid: &str,
    ) -> Result<Option<Transfer>, StoreError> {
        // served by the index `transfer_request` (migration 9). `rowid`
        // grows with each insert, so a file from before 0.12 that holds
        // several transfers for one request gives the first one.
        let sql = format!(
            "SELECT {TRANSFER_COLUMNS} FROM transfer
             WHERE counter_party = ?1 AND consumer_pid = ?2
             ORDER BY rowid LIMIT 1"
        );
        let found = self.query(
            &sql,
            params![counter_party, consumer_pid],
            transfer_from_row,
        )?;
        Ok(found.into_iter().next())
    }

    fn insert(&self, transfer: &Transfer) -> Result<bool, StoreError> {
        // one statement, so it is atomic: the row is added only if this
        // consumer has no transfer with this consumer pid yet (DSP 2025-1,
        // "Transfer Request Message": idempotent on `consumerPid`). Not a
        // UNIQUE index, because files from before 0.12 may hold repeated
        // requests and must still open.
        let sql = format!(
            "INSERT INTO transfer ({TRANSFER_COLUMNS})
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10
             WHERE NOT EXISTS (
                 SELECT 1 FROM transfer WHERE counter_party = ?5 AND consumer_pid = ?2
             )"
        );
        let inserted = self
            .conn()
            .execute(
                &sql,
                params![
                    transfer.provider_pid,
                    transfer.consumer_pid,
                    transfer_state_text(transfer.state)?,
                    transfer.callback_address,
                    transfer.counter_party,
                    transfer.agreement_id,
                    transfer.dataset_id,
                    transfer.format,
                    pending_text(transfer.pending.as_ref())?,
                    version(transfer.version)?,
                ],
            )
            .map_err(err)?;
        Ok(inserted == 1)
    }

    fn replace_if(&self, transfer: &Transfer, expected: u64) -> Result<bool, StoreError> {
        // compare-and-update in one statement: a concurrent change makes
        // the WHERE clause miss and nothing is written. Only the fields a
        // transfer step changes are written.
        let changed = self
            .conn()
            .execute(
                "UPDATE transfer SET state = ?2, pending = ?3, version = ?4
                 WHERE provider_pid = ?1 AND version = ?5",
                params![
                    transfer.provider_pid,
                    transfer_state_text(transfer.state)?,
                    pending_text(transfer.pending.as_ref())?,
                    version(transfer.version)?,
                    version(expected)?,
                ],
            )
            .map_err(err)?;
        Ok(changed == 1)
    }

    fn pending(&self) -> Result<Vec<Transfer>, StoreError> {
        let sql = format!(
            "SELECT {TRANSFER_COLUMNS} FROM transfer WHERE pending IS NOT NULL
             ORDER BY provider_pid"
        );
        self.query(&sql, [], transfer_from_row)
    }
}

impl Agreements for SqliteNegotiations {
    fn finalized(&self, agreement_id: &str) -> Result<Option<Agreement>, StoreError> {
        // json_extract is SQLite's; a PostgreSQL adapter would use
        // `agreement::json->>'@id'` or a column of its own
        let sql = "SELECT agreement FROM negotiation
                   WHERE state = ?1 AND json_extract(agreement, '$.\"@id\"') = ?2";
        let finalized = state_text(NegotiationState::Finalized)?;
        let found = self.query(sql, params![finalized, agreement_id], |row| {
            parsed("agreement", &col::<String>(row, 0)?)
        })?;
        Ok(found.into_iter().next())
    }

    fn negotiation_of(&self, agreement_id: &str) -> Result<Option<Negotiation>, StoreError> {
        // the expression is the one of the index `negotiation_agreement`
        // (migration 10), as in `finalized` above; agreement ids are made
        // here and are unique, so there is at most one row
        let sql = format!(
            "SELECT {NEGOTIATION_COLUMNS} FROM negotiation
             WHERE json_extract(agreement, '$.\"@id\"') = ?1"
        );
        let found = self.query(&sql, params![agreement_id], negotiation_from_row)?;
        Ok(found.into_iter().next())
    }

    fn replace_negotiation_if(
        &self,
        negotiation: &Negotiation,
        expected: u64,
    ) -> Result<bool, StoreError> {
        // the same compare-and-update the negotiation service uses
        NegotiationStore::replace_if(self, negotiation, expected)
    }
}

/// The agreement of a finalized negotiation, for adapters that keep
/// negotiations in memory.
#[must_use]
pub fn finalized_agreement(negotiation: &Negotiation, agreement_id: &str) -> Option<Agreement> {
    let agreement = negotiation.agreement.as_ref()?;
    (negotiation.state == NegotiationState::Finalized && agreement.id == agreement_id)
        .then(|| agreement.clone())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    fn stored_transfer(pending: Option<Outgoing>) -> Transfer {
        Transfer {
            provider_pid: "urn:uuid:p".to_owned(),
            consumer_pid: "urn:uuid:c".to_owned(),
            state: TransferState::Requested,
            callback_address: "https://c.example/cb".to_owned(),
            counter_party: "did:web:c".to_owned(),
            agreement_id: "urn:uuid:a".to_owned(),
            dataset_id: "d".to_owned(),
            format: "HttpData-PULL".to_owned(),
            pending,
            version: 0,
        }
    }

    #[test]
    fn transfers_round_trip_with_their_pending_message() {
        let file = SqliteNegotiations::in_memory().unwrap();
        let db: &dyn TransferStore = &file;
        let start = Outgoing {
            provider_pid: "urn:uuid:p".to_owned(),
            url: "https://c.example/cb/transfers/urn:uuid:c/start".to_owned(),
            body: json!({"@type": "TransferStartMessage"}),
            transition: Transition::Start,
            to: TransferState::Started,
            status_url: "https://c.example/cb/transfers/urn:uuid:c".to_owned(),
            counter_party: "did:web:c".to_owned(),
        };
        let t = stored_transfer(Some(start));
        assert!(db.insert(&t).unwrap());
        assert_eq!(db.get("urn:uuid:p").unwrap(), Some(t.clone()));
        assert_eq!(db.pending().unwrap(), std::slice::from_ref(&t));
        let mut same_id = t.clone();
        same_id.consumer_pid = "urn:uuid:c2".to_owned();
        assert!(db.insert(&same_id).is_err(), "the id is taken");

        let mut started = t.clone();
        started.state = TransferState::Started;
        started.pending = None;
        started.version = 1;
        assert!(db.replace_if(&started, 0).unwrap());
        assert!(!db.replace_if(&started, 0).unwrap(), "version 0 is gone");
        assert_eq!(db.get("urn:uuid:p").unwrap(), Some(started));
        assert!(db.pending().unwrap().is_empty());
    }

    /// DSP 2025-1, "Transfer Request Message": idempotent on
    /// `consumerPid`, for the same consumer.
    #[test]
    fn a_request_is_stored_once_for_each_consumer() {
        let file = SqliteNegotiations::in_memory().unwrap();
        let db: &dyn TransferStore = &file;
        let first = stored_transfer(None);
        assert!(db.insert(&first).unwrap());

        // the same consumer and consumer pid again: nothing is written
        let mut again = first.clone();
        again.provider_pid = "urn:uuid:p2".to_owned();
        assert!(!db.insert(&again).unwrap());
        assert_eq!(db.get("urn:uuid:p2").unwrap(), None);
        let found = db.requested("did:web:c", "urn:uuid:c").unwrap();
        assert_eq!(found, Some(first.clone()));

        // another consumer with the same consumer pid: its own transfer
        assert_eq!(db.requested("did:web:other", "urn:uuid:c").unwrap(), None);
        let mut others = first.clone();
        others.provider_pid = "urn:uuid:p3".to_owned();
        others.counter_party = "did:web:other".to_owned();
        assert!(db.insert(&others).unwrap());
        let found = db.requested("did:web:other", "urn:uuid:c").unwrap();
        assert_eq!(found, Some(others));
        let found = db.requested("did:web:c", "urn:uuid:c").unwrap();
        assert_eq!(found, Some(first));
        assert_eq!(db.requested("did:web:c", "urn:uuid:unknown").unwrap(), None);
    }

    #[test]
    fn only_finalized_agreements_are_found() {
        let db = SqliteNegotiations::in_memory().unwrap();
        let negotiations: &dyn NegotiationStore = &db;
        let agreement: Agreement = serde_json::from_value(json!({
            "@id": "urn:uuid:a", "@type": "Agreement", "target": "d",
            "assigner": "did:web:p", "assignee": "did:web:c",
            "timestamp": "2026-11-13T00:00:00Z", "permission": [{"action": "use"}]
        }))
        .unwrap();
        let mut n: Negotiation = Negotiation {
            provider_pid: "urn:uuid:n".to_owned(),
            consumer_pid: "urn:uuid:c".to_owned(),
            state: NegotiationState::Agreed,
            callback_address: "https://c.example/cb".to_owned(),
            counter_party: "did:web:c".to_owned(),
            offer: serde_json::from_value(json!({
                "@id": "o", "@type": "Offer", "target": "d", "permission": [{"action": "use"}]
            }))
            .unwrap(),
            agreement: Some(agreement.clone()),
            pending: None,
            version: 0,
        };
        negotiations.insert(&n).unwrap();
        assert_eq!(db.finalized("urn:uuid:a").unwrap(), None, "only agreed");
        assert_eq!(finalized_agreement(&n, "urn:uuid:a"), None);

        n.state = NegotiationState::Finalized;
        n.version = 1;
        assert!(negotiations.replace_if(&n, 0).unwrap());
        assert_eq!(db.finalized("urn:uuid:a").unwrap(), Some(agreement.clone()));
        assert_eq!(db.finalized("urn:uuid:other").unwrap(), None);
        assert_eq!(finalized_agreement(&n, "urn:uuid:a"), Some(agreement));
    }

    /// The negotiation behind an agreement is found in any state, and
    /// written with the version check of the negotiation store (ADR 0007,
    /// amended in 0.13).
    #[test]
    fn the_negotiation_of_an_agreement_is_found_and_replaced_by_version() {
        let db = SqliteNegotiations::in_memory().unwrap();
        let negotiations: &dyn NegotiationStore = &db;
        let without_agreement: Negotiation = Negotiation {
            provider_pid: "urn:uuid:n0".to_owned(),
            consumer_pid: "urn:uuid:c0".to_owned(),
            state: NegotiationState::Requested,
            callback_address: "https://c.example/cb".to_owned(),
            counter_party: "did:web:c".to_owned(),
            offer: serde_json::from_value(json!({
                "@id": "o", "@type": "Offer", "target": "d", "permission": [{"action": "use"}]
            }))
            .unwrap(),
            agreement: None,
            pending: None,
            version: 0,
        };
        negotiations.insert(&without_agreement).unwrap();
        assert_eq!(db.negotiation_of("urn:uuid:a").unwrap(), None);

        let mut verified = without_agreement.clone();
        verified.provider_pid = "urn:uuid:n1".to_owned();
        verified.state = NegotiationState::Verified;
        verified.agreement = Some(
            serde_json::from_value(json!({
                "@id": "urn:uuid:a", "@type": "Agreement", "target": "d",
                "assigner": "did:web:p", "assignee": "did:web:c",
                "timestamp": "2026-11-13T00:00:00Z", "permission": [{"action": "use"}]
            }))
            .unwrap(),
        );
        negotiations.insert(&verified).unwrap();
        assert_eq!(db.finalized("urn:uuid:a").unwrap(), None);
        assert_eq!(
            db.negotiation_of("urn:uuid:a").unwrap(),
            Some(verified.clone())
        );
        assert_eq!(db.negotiation_of("urn:uuid:other").unwrap(), None);

        let mut finalized = verified.clone();
        finalized.state = NegotiationState::Finalized;
        finalized.version = 1;
        assert!(db.replace_negotiation_if(&finalized, 0).unwrap());
        assert!(
            !db.replace_negotiation_if(&finalized, 0).unwrap(),
            "version 0 is gone"
        );
        assert_eq!(db.finalized("urn:uuid:a").unwrap(), finalized.agreement);
        // the other negotiation is untouched
        assert_eq!(
            negotiations.get("urn:uuid:n0").unwrap(),
            Some(without_agreement)
        );
    }
}
