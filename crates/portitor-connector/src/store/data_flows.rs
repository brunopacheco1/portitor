// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The built-in data plane's flows, in the same database (ADR 0013).

use portitor_dps::state::DataFlowState;
use portitor_dsp::transfer::DataAddress;
use portitor_runtime::database::Row;
use portitor_runtime::values;

use super::{ConnectorStore, col};
use crate::auth::Membership;
use crate::flow_membership::{MembershipStore, StoredMembership};
use portitor_dataplane::{Flow, FlowStore, FlowStoreError};

#[allow(clippy::needless_pass_by_value)] // used as map_err(store_error)
fn store_error(e: super::StoreError) -> FlowStoreError {
    FlowStoreError(e.0)
}

/// A flow state as stored: its signaling name, for example `STARTED`.
fn state_text(s: DataFlowState) -> Result<String, FlowStoreError> {
    match serde_json::to_value(s) {
        Ok(serde_json::Value::String(t)) => Ok(t),
        _ => Err(FlowStoreError(format!("cannot store state {s:?}"))),
    }
}

/// A push address as stored: its DSP JSON.
fn address_text(address: Option<&DataAddress>) -> Result<Option<String>, FlowStoreError> {
    address
        .map(|a| serde_json::to_string(a).map_err(|e| FlowStoreError(e.to_string())))
        .transpose()
}

const FLOW_COLUMNS: &str = "id, state, dataset_id, token_hash, expires_at, push_address, source, agreement_id, counter_party_id";

fn flow_from_row(row: &Row) -> Result<Flow, super::StoreError> {
    let state: String = col(row, 1)?;
    let expires_at: i64 = col(row, 4)?;
    let push_address = match col::<Option<String>>(row, 5)? {
        Some(text) => Some(
            serde_json::from_str(&text)
                .map_err(|e| super::StoreError(format!("stored push address: {e}")))?,
        ),
        None => None,
    };
    Ok(Flow {
        id: col(row, 0)?,
        state: serde_json::from_value(serde_json::Value::String(state))
            .map_err(|e| super::StoreError(format!("stored state: {e}")))?,
        dataset_id: col(row, 2)?,
        token_hash: col(row, 3)?,
        expires_at: u64::try_from(expires_at).unwrap_or(0),
        push_address,
        source: col(row, 6)?,
        agreement_id: col(row, 7)?,
        counter_party_id: col(row, 8)?,
    })
}

/// The first column of a row, as text.
fn first_text(row: &Row) -> Result<String, super::StoreError> {
    col(row, 0)
}

/// The first column of a row, as text or NULL.
fn first_text_or_null(row: &Row) -> Result<Option<String>, super::StoreError> {
    col(row, 0)
}

impl FlowStore for ConnectorStore {
    fn get(&self, id: &str) -> Result<Option<Flow>, FlowStoreError> {
        let sql = format!("SELECT {FLOW_COLUMNS} FROM data_flow WHERE id = $1");
        self.query_first(&sql, values![id], flow_from_row)
            .map_err(store_error)
    }

    fn insert(&self, f: &Flow) -> Result<bool, FlowStoreError> {
        // a flow whose id is taken is not written: one statement, so it
        // is atomic
        let expires_at = i64::try_from(f.expires_at).unwrap_or(i64::MAX);
        let sql = format!(
            "INSERT INTO data_flow ({FLOW_COLUMNS})
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT DO NOTHING"
        );
        let added = self
            .execute(
                &sql,
                values![
                    &f.id,
                    state_text(f.state)?,
                    &f.dataset_id,
                    &f.token_hash,
                    expires_at,
                    address_text(f.push_address.as_ref())?,
                    f.source.as_deref(),
                    &f.agreement_id,
                    &f.counter_party_id
                ],
            )
            .map_err(store_error)?;
        Ok(added == 1)
    }

    fn set_state(
        &self,
        id: &str,
        from: DataFlowState,
        to: DataFlowState,
    ) -> Result<bool, FlowStoreError> {
        // compare-and-update: a concurrent change makes the WHERE clause
        // miss. An ended flow keeps no address of the consumer's (ADR 0027)
        let sql = if to.is_final() {
            "UPDATE data_flow SET state = $3, push_address = NULL WHERE id = $1 AND state = $2"
        } else {
            "UPDATE data_flow SET state = $3 WHERE id = $1 AND state = $2"
        };
        let changed = self
            .execute(sql, values![id, state_text(from)?, state_text(to)?])
            .map_err(store_error)?;
        Ok(changed == 1)
    }

    fn set_token(
        &self,
        id: &str,
        state: DataFlowState,
        token_hash: &str,
        expires_at: u64,
    ) -> Result<bool, FlowStoreError> {
        let expires_at = i64::try_from(expires_at).unwrap_or(i64::MAX);
        let changed = self
            .execute(
                "UPDATE data_flow SET token_hash = $3, expires_at = $4
                 WHERE id = $1 AND state = $2",
                values![id, state_text(state)?, token_hash, expires_at],
            )
            .map_err(store_error)?;
        Ok(changed == 1)
    }

    fn set_push_address(
        &self,
        id: &str,
        state: DataFlowState,
        address: &DataAddress,
    ) -> Result<bool, FlowStoreError> {
        let changed = self
            .execute(
                "UPDATE data_flow SET push_address = $3
                 WHERE id = $1 AND state = $2 AND push_address IS NOT NULL",
                values![id, state_text(state)?, address_text(Some(address))?],
            )
            .map_err(store_error)?;
        Ok(changed == 1)
    }

    fn started_pushes(&self) -> Result<Vec<String>, FlowStoreError> {
        let sql = "SELECT id FROM data_flow
                   WHERE state = $1 AND push_address IS NOT NULL ORDER BY id";
        self.query(
            sql,
            values![state_text(DataFlowState::Started)?],
            first_text,
        )
        .map_err(store_error)
    }
}

/// What the control plane keeps of each flow's consumer (ADR 0027,
/// section 7.2), in its own table.
impl MembershipStore for ConnectorStore {
    fn memberships(&self, id: &str) -> Result<StoredMembership, FlowStoreError> {
        // a row whose `memberships` is NULL: the migration's mark for a
        // flow of 1.0 (migration 12)
        let sql = "SELECT memberships FROM flow_membership WHERE flow_id = $1";
        let found = self
            .query_first(sql, values![id], first_text_or_null)
            .map_err(store_error)?;
        match found {
            None => Ok(StoredMembership::Missing),
            Some(None) => Ok(StoredMembership::StartedBefore1_1),
            Some(Some(text)) => serde_json::from_str(&text)
                .map(StoredMembership::Kept)
                .map_err(|e| FlowStoreError(format!("stored memberships: {e}"))),
        }
    }

    fn keep_memberships(&self, id: &str, memberships: &[Membership]) -> Result<(), FlowStoreError> {
        let text = serde_json::to_string(memberships)
            .map_err(|e| FlowStoreError(format!("memberships: {e}")))?;
        // the first memberships kept for a flow stay
        self.execute(
            "INSERT INTO flow_membership (flow_id, memberships) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
            values![id, text],
        )
        .map_err(store_error)?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::store::on_every_database;

    #[test]
    fn flows_round_trip_and_change_state_only_from_the_expected_one() {
        on_every_database(flows_round_trip_and_change_state_only_from_the_expected_one_on);
    }

    fn flows_round_trip_and_change_state_only_from_the_expected_one_on(db: &ConnectorStore) {
        let store: &dyn FlowStore = db;
        let f = Flow {
            id: "f1".to_owned(),
            state: DataFlowState::Started,
            dataset_id: "d1".to_owned(),
            source: Some("https://upstream.example/d1".to_owned()),
            agreement_id: "urn:uuid:a".to_owned(),
            counter_party_id: "did:web:c".to_owned(),
            token_hash: "ab".repeat(32),
            expires_at: 1_800_000_000,
            push_address: None,
        };
        assert!(store.insert(&f).unwrap());
        assert!(!store.insert(&f).unwrap(), "the id is taken");
        assert_eq!(store.get("f1").unwrap(), Some(f.clone()));
        assert_eq!(store.get("f2").unwrap(), None);
        let (started, suspended) = (DataFlowState::Started, DataFlowState::Suspended);
        assert!(store.set_state("f1", started, suspended).unwrap());
        assert!(!store.set_state("f1", started, suspended).unwrap());
        assert_eq!(store.get("f1").unwrap().unwrap().state, suspended);

        let new_hash = "cd".repeat(32);
        assert!(
            !store.set_token("f1", started, &new_hash, 9).unwrap(),
            "not started"
        );
        assert!(store.set_token("f1", suspended, &new_hash, 9).unwrap());
        let renewed = store.get("f1").unwrap().unwrap();
        assert_eq!((renewed.token_hash, renewed.expires_at), (new_hash, 9));
    }

    #[test]
    fn memberships_are_kept_once_per_flow() {
        on_every_database(memberships_are_kept_once_per_flow_on);
    }

    fn memberships_are_kept_once_per_flow_on(db: &ConnectorStore) {
        let store: &dyn MembershipStore = db;
        assert_eq!(store.memberships("f1").unwrap(), StoredMembership::Missing);
        let entry: portitor_dcp::status_list::StatusEntry = serde_json::from_value(
            serde_json::json!({"type": "BitstringStatusListEntry", "statusPurpose": "revocation",
                "statusListIndex": "5", "statusListCredential": "https://issuer.example/status/1"}),
        )
        .unwrap();
        let kept = vec![Membership {
            issuer: "did:web:issuer.example".to_owned(),
            valid_until: Some(1_900_000_000),
            status: vec![entry],
        }];
        store.keep_memberships("f1", &kept).unwrap();
        store.keep_memberships("f1", &[]).unwrap();
        assert_eq!(
            store.memberships("f1").unwrap(),
            StoredMembership::Kept(kept),
            "the first is kept"
        );
    }
}
