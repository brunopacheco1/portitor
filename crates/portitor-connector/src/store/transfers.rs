// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Transfers as provider, and the agreements they are made under, in the
//! same database as the negotiations (ADR 0013).

use portitor_dsp::negotiation::provider::{Negotiation, NegotiationStore};
use portitor_dsp::negotiation::{Agreement, NegotiationState};
use portitor_dsp::transfer::provider::{
    Agreements, FlowStep, Lease, Outgoing, Transfer, TransferStore,
};
use portitor_dsp::transfer::{TransferState, Transition};
use portitor_runtime::database::{Row, Value};
use portitor_runtime::values;

use super::{
    ConnectorStore, NEGOTIATION_COLUMNS, StoreError, StoredMessage, col, json,
    negotiation_from_row, parsed, parsed_version, state_text, version,
};

const TRANSFER_COLUMNS: &str = "provider_pid, consumer_pid, state, callback_address, \
     counter_party, agreement_id, dataset_id, format, pending, version, data_address, flow_step, \
     state_before_step, lease_holder, lease_until";

/// The data plane step not run yet, as stored: its name, for example
/// `suspend` (migration 16).
fn flow_step_text(transfer: &Transfer) -> Option<&'static str> {
    transfer.flow_step.map(FlowStep::name)
}

fn parsed_flow_step(text: Option<String>) -> Result<Option<FlowStep>, StoreError> {
    let Some(text) = text else {
        return Ok(None);
    };
    match FlowStep::from_name(&text) {
        Some(step) => Ok(Some(step)),
        None => Err(StoreError(format!("stored data plane step {text}"))),
    }
}

/// The state before the write that recorded the step, as stored: its DSP
/// name, or NULL (migration 17).
fn state_before_step_text(transfer: &Transfer) -> Result<Option<String>, StoreError> {
    transfer
        .state_before_step
        .map(transfer_state_text)
        .transpose()
}

/// The lease as stored: its holder and its end in seconds since 1970, or
/// two NULLs (migration 17).
fn lease_columns(transfer: &Transfer) -> (Option<&str>, Option<i64>) {
    match &transfer.lease {
        Some(lease) => (
            Some(lease.holder.as_str()),
            Some(i64::try_from(lease.until).unwrap_or(i64::MAX)),
        ),
        None => (None, None),
    }
}

/// The lease read back from its two columns. Only both together make
/// one; a negative end (never written) counts as ended long ago.
fn parsed_lease(holder: Option<String>, until: Option<i64>) -> Option<Lease> {
    match (holder, until) {
        (Some(holder), Some(until)) => Some(Lease {
            holder,
            until: u64::try_from(until).unwrap_or(0),
        }),
        _ => None,
    }
}

/// The consumer's push address as stored: its DSP JSON (ADR 0027).
fn address_text(transfer: &Transfer) -> Result<Option<String>, StoreError> {
    transfer.data_address.as_ref().map(json).transpose()
}

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

fn transfer_from_row(row: &Row) -> Result<Transfer, StoreError> {
    let provider_pid: String = col(row, 0)?;
    let counter_party: String = col(row, 4)?;
    let pending = match col::<Option<String>>(row, 8)? {
        Some(text) => Some(pending_message(&provider_pid, &counter_party, &text)?),
        None => None,
    };
    let data_address = match col::<Option<String>>(row, 10)? {
        Some(text) => Some(parsed("push address", &text)?),
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
        data_address,
        flow_step: parsed_flow_step(col(row, 11)?)?,
        state_before_step: match col::<Option<String>>(row, 12)? {
            Some(text) => Some(parsed_transfer_state(text)?),
            None => None,
        },
        lease: parsed_lease(col(row, 13)?, col(row, 14)?),
        provider_pid,
    })
}

/// The values of a new `transfer` row, in the order of [`TRANSFER_COLUMNS`].
fn transfer_values(transfer: &Transfer) -> Result<Vec<Value>, StoreError> {
    let (lease_holder, lease_until) = lease_columns(transfer);
    Ok(values![
        &transfer.provider_pid,
        &transfer.consumer_pid,
        transfer_state_text(transfer.state)?,
        &transfer.callback_address,
        &transfer.counter_party,
        &transfer.agreement_id,
        &transfer.dataset_id,
        &transfer.format,
        pending_text(transfer.pending.as_ref())?,
        version(transfer.version)?,
        address_text(transfer)?,
        flow_step_text(transfer),
        state_before_step_text(transfer)?,
        lease_holder,
        lease_until,
    ]
    .to_vec())
}

impl TransferStore for ConnectorStore {
    fn get(&self, provider_pid: &str) -> Result<Option<Transfer>, StoreError> {
        let sql = format!("SELECT {TRANSFER_COLUMNS} FROM transfer WHERE provider_pid = $1");
        self.query_first(&sql, values![provider_pid], transfer_from_row)
    }

    fn requested(
        &self,
        counter_party: &str,
        consumer_pid: &str,
    ) -> Result<Option<Transfer>, StoreError> {
        // served by the index `transfer_request` (migration 9). In SQLite
        // `rowid` grows with each insert, so a file from before 0.12 that
        // holds several transfers for one request gives the first one.
        // PostgreSQL has no `rowid` and needs no order: its
        // `transfer_request` is UNIQUE, so there is at most one row.
        let first_stored_first = if self.database.is_postgres() {
            ""
        } else {
            "ORDER BY rowid"
        };
        let sql = format!(
            "SELECT {TRANSFER_COLUMNS} FROM transfer
             WHERE counter_party = $1 AND consumer_pid = $2
             {first_stored_first} LIMIT 1"
        );
        self.query_first(
            &sql,
            values![counter_party, consumer_pid],
            transfer_from_row,
        )
    }

    fn insert(&self, transfer: &Transfer) -> Result<bool, StoreError> {
        // one transaction, so the check and the write are atomic: the row
        // is added only if this consumer has no transfer with this
        // consumer pid yet (DSP 2025-1, "Transfer Request Message":
        // idempotent on `consumerPid`). In SQLite not a UNIQUE index,
        // because files from before 0.12 may hold repeated requests and
        // must still open.
        let row = transfer_values(transfer)?;
        let insert = format!(
            "INSERT INTO transfer ({TRANSFER_COLUMNS})
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)"
        );
        self.transaction(|tx| {
            let already_requested = tx.query_one(
                "SELECT 1 FROM transfer WHERE counter_party = $1 AND consumer_pid = $2",
                values![&transfer.counter_party, &transfer.consumer_pid],
            )?;
            if already_requested.is_some() {
                return Ok(false);
            }
            tx.execute(&insert, &row)?;
            Ok(true)
        })
    }

    fn replace_if(&self, transfer: &Transfer, expected: u64) -> Result<bool, StoreError> {
        // compare-and-update in one statement: a concurrent change makes
        // the WHERE clause miss and nothing is written. Only the fields a
        // transfer step changes are written. The lease is taken and given
        // back in this same statement (TD-48): nothing else writes it.
        let (lease_holder, lease_until) = lease_columns(transfer);
        let changed = self.execute(
            "UPDATE transfer SET state = $2, pending = $3, version = $4, data_address = $6,
                 flow_step = $7, state_before_step = $8, lease_holder = $9, lease_until = $10
             WHERE provider_pid = $1 AND version = $5",
            values![
                &transfer.provider_pid,
                transfer_state_text(transfer.state)?,
                pending_text(transfer.pending.as_ref())?,
                version(transfer.version)?,
                version(expected)?,
                address_text(transfer)?,
                flow_step_text(transfer),
                state_before_step_text(transfer)?,
                lease_holder,
                lease_until,
            ],
        )?;
        Ok(changed == 1)
    }

    fn pending(&self) -> Result<Vec<Transfer>, StoreError> {
        let sql = format!(
            "SELECT {TRANSFER_COLUMNS} FROM transfer
             WHERE pending IS NOT NULL OR flow_step IS NOT NULL
             ORDER BY provider_pid"
        );
        self.query(&sql, values![], transfer_from_row)
    }

    fn recorded_steps(&self) -> Result<Vec<Transfer>, StoreError> {
        // served by the partial index `transfer_recorded_step` (migration
        // 17): the sweep reads it every few seconds, and it is almost
        // always empty
        let sql = format!(
            "SELECT {TRANSFER_COLUMNS} FROM transfer
             WHERE flow_step IS NOT NULL
             ORDER BY provider_pid"
        );
        self.query(&sql, values![], transfer_from_row)
    }

    fn list(&self, after: Option<&str>, limit: usize) -> Result<Vec<Transfer>, StoreError> {
        // the primary key is the order, so no index is needed
        // no `after`: from the first. The CAST gives PostgreSQL the type of
        // $1, which it cannot tell from `$1 IS NULL`
        let sql = format!(
            "SELECT {TRANSFER_COLUMNS} FROM transfer
             WHERE CAST($1 AS TEXT) IS NULL OR provider_pid > $1
             ORDER BY provider_pid LIMIT $2"
        );
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.query(&sql, values![after, limit], transfer_from_row)
    }
}

/// The agreement in the first column of a row.
fn agreement_from_row(row: &Row) -> Result<Agreement, StoreError> {
    parsed("agreement", &col::<String>(row, 0)?)
}

impl Agreements for ConnectorStore {
    fn finalized(&self, agreement_id: &str) -> Result<Option<Agreement>, StoreError> {
        // `json_extract` is SQLite's; the PostgreSQL schema defines the
        // same function. The expression is the one of the index
        // `negotiation_agreement` (migration 10)
        let sql = "SELECT agreement FROM negotiation
                   WHERE state = $1 AND json_extract(agreement, '$.\"@id\"') = $2";
        let finalized = state_text(NegotiationState::Finalized)?;
        self.query_first(sql, values![finalized, agreement_id], agreement_from_row)
    }

    fn negotiation_of(&self, agreement_id: &str) -> Result<Option<Negotiation>, StoreError> {
        // the expression is the one of the index `negotiation_agreement`
        // (migration 10), as in `finalized` above; agreement ids are made
        // here and are unique, so there is at most one row
        let sql = format!(
            "SELECT {NEGOTIATION_COLUMNS} FROM negotiation
             WHERE json_extract(agreement, '$.\"@id\"') = $1"
        );
        self.query_first(&sql, values![agreement_id], negotiation_from_row)
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
    use crate::store::on_every_database;

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
            data_address: None,
            pending,
            flow_step: None,
            state_before_step: None,
            lease: None,
            version: 0,
        }
    }

    #[test]
    fn transfers_round_trip_with_their_pending_message() {
        on_every_database(transfers_round_trip_with_their_pending_message_on);
    }

    fn transfers_round_trip_with_their_pending_message_on(file: &ConnectorStore) {
        let db: &dyn TransferStore = file;
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

    /// The data plane step not run yet is kept with the transfer, with its
    /// lease and the state before its write, and a transfer that has one
    /// is taken up again after a restart and by the sweep, also without a
    /// pending message (ADR 0028, decision 6; TD-48).
    #[test]
    fn a_recorded_data_plane_step_round_trips_and_is_taken_up_again() {
        on_every_database(a_recorded_data_plane_step_round_trips_and_is_taken_up_again_on);
    }

    fn a_recorded_data_plane_step_round_trips_and_is_taken_up_again_on(file: &ConnectorStore) {
        let db: &dyn TransferStore = file;
        let mut t = stored_transfer(None);
        t.flow_step = Some(FlowStep::Start);
        t.lease = Some(Lease {
            holder: "urn:uuid:process".to_owned(),
            until: 1_800_000_030,
        });
        assert!(db.insert(&t).unwrap());
        assert_eq!(db.get("urn:uuid:p").unwrap(), Some(t.clone()));
        assert_eq!(db.pending().unwrap(), std::slice::from_ref(&t));
        assert_eq!(db.recorded_steps().unwrap(), std::slice::from_ref(&t));

        let all_steps = [
            FlowStep::Restart,
            FlowStep::Resume,
            FlowStep::ResumeWithAddress,
            FlowStep::Suspend,
            FlowStep::Complete,
            FlowStep::Terminate,
        ];
        for step in all_steps {
            let mut next = db.get("urn:uuid:p").unwrap().unwrap();
            let expected = next.version;
            next.flow_step = Some(step);
            next.state_before_step = Some(TransferState::Started);
            next.lease = Some(Lease {
                holder: "urn:uuid:other-process".to_owned(),
                until: 1_800_000_060,
            });
            next.version += 1;
            assert!(db.replace_if(&next, expected).unwrap());
            assert_eq!(db.get("urn:uuid:p").unwrap(), Some(next));
        }

        // the step ran: nothing to take up any more
        let mut ran = db.get("urn:uuid:p").unwrap().unwrap();
        let expected = ran.version;
        ran.flow_step = None;
        ran.state_before_step = None;
        ran.lease = None;
        ran.version += 1;
        assert!(db.replace_if(&ran, expected).unwrap());
        assert_eq!(db.get("urn:uuid:p").unwrap(), Some(ran));
        assert!(db.pending().unwrap().is_empty());
        assert!(db.recorded_steps().unwrap().is_empty());
    }

    /// DSP 2025-1, "Transfer Request Message": idempotent on
    /// `consumerPid`, for the same consumer.
    #[test]
    fn a_request_is_stored_once_for_each_consumer() {
        on_every_database(a_request_is_stored_once_for_each_consumer_on);
    }

    fn a_request_is_stored_once_for_each_consumer_on(file: &ConnectorStore) {
        let db: &dyn TransferStore = file;
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
        on_every_database(only_finalized_agreements_are_found_on);
    }

    fn only_finalized_agreements_are_found_on(db: &ConnectorStore) {
        let negotiations: &dyn NegotiationStore = db;
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
        on_every_database(the_negotiation_of_an_agreement_is_found_and_replaced_by_version_on);
    }

    fn the_negotiation_of_an_agreement_is_found_and_replaced_by_version_on(db: &ConnectorStore) {
        let negotiations: &dyn NegotiationStore = db;
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
