// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The shared core on a small record of its own: a transfer as the
//! consumer keeps it, with only the fields the core uses. The four
//! services' tests check the same steps on the real records.

use std::cell::{Cell, RefCell};

use super::*;
use crate::transfer::{TransferState, Transition};

/// Our message: what it does, and whether it is the first request.
#[derive(Debug, Clone, PartialEq)]
struct Sent {
    transition: Transition,
    first: bool,
}

/// A transfer as the consumer keeps it, reduced to what the core reads.
#[derive(Debug, Clone, PartialEq)]
struct Kept {
    state: TransferState,
    pending: Option<Sent>,
    version: u64,
    /// Set by `before_write`.
    checked: bool,
}

impl Record for Kept {
    type State = TransferState;
    type Outgoing = Sent;
    const PARTY: Role = Role::Consumer;

    fn state(&self) -> TransferState {
        self.state
    }
    fn set_state(&mut self, state: TransferState) {
        self.state = state;
    }
    fn pending(&self) -> Option<&Sent> {
        self.pending.as_ref()
    }
    fn set_pending(&mut self, pending: Option<Sent>) {
        self.pending = pending;
    }
    fn version(&self) -> u64 {
        self.version
    }
    fn set_version(&mut self, version: u64) {
        self.version = version;
    }
    fn transition_of(message: &Sent) -> Transition {
        message.transition
    }
    fn is_first_request(message: &Sent) -> bool {
        message.first
    }
}

fn kept(state: TransferState, pending: Option<Sent>) -> Kept {
    Kept {
        state,
        pending,
        version: 0,
        checked: false,
    }
}

fn sent(transition: Transition) -> Sent {
    Sent {
        transition,
        first: false,
    }
}

#[test]
fn a_message_that_fits_the_state_moves_it() {
    let mut transfer = kept(TransferState::Requested, None);
    assert!(receive(&mut transfer, Transition::Start).is_ok());
    assert_eq!(transfer.state, TransferState::Started);
}

#[test]
fn a_message_that_fits_only_after_our_pending_one_proves_it_arrived() {
    // we suspended; the provider starts again before our suspension's
    // acknowledgement is recorded: it must have had our suspension
    let mut transfer = kept(TransferState::Started, Some(sent(Transition::Suspend)));
    assert!(receive(&mut transfer, Transition::Start).is_ok());
    assert_eq!(transfer.state, TransferState::Started);
    assert_eq!(transfer.pending, None);
}

#[test]
fn a_message_that_fits_neither_state_is_refused_and_says_why() {
    let mut transfer = kept(TransferState::Requested, Some(sent(Transition::Terminate)));
    let refused = receive(&mut transfer, Transition::Complete);
    assert!(matches!(refused, Err(Refused::Ending)));
    let mut transfer = kept(TransferState::Requested, None);
    let Err(Refused::NotAllowed(error)) = receive(&mut transfer, Transition::Complete) else {
        panic!("a completion of a requested transfer is not allowed");
    };
    assert_eq!(
        error.to_string(),
        "Complete is not allowed in state Requested"
    );
}

#[test]
fn while_our_termination_is_on_its_way_only_a_termination_is_taken() {
    let mut transfer = kept(TransferState::Started, Some(sent(Transition::Terminate)));
    assert!(receive(&mut transfer, Transition::Terminate).is_ok());
    assert_eq!(transfer.state, TransferState::Terminated);
}

#[test]
fn any_provider_message_proves_the_first_request_arrived() {
    let request = Sent {
        transition: Transition::Request,
        first: true,
    };
    let mut transfer = kept(TransferState::Requested, Some(request.clone()));
    assert!(receive(&mut transfer, Transition::Start).is_ok());
    assert_eq!(transfer.pending, None);
    // its acknowledgement changes no state: the request created it
    let mut transfer = kept(TransferState::Requested, Some(request.clone()));
    assert!(transfer.apply_sent(&request).is_ok());
    assert_eq!(transfer.state, TransferState::Requested);
    assert_eq!(transfer.pending, None);
}

#[test]
fn a_report_about_a_message_counts_only_while_it_is_pending() {
    let suspension = sent(Transition::Suspend);
    let transfer = kept(TransferState::Started, Some(suspension.clone()));
    assert!(is_pending(&transfer, &suspension));
    assert!(can_abort(&transfer, &suspension));
    assert!(!is_pending(&transfer, &sent(Transition::Complete)));
    let ended = kept(TransferState::Terminated, Some(suspension.clone()));
    assert!(!can_abort(&ended, &suspension));
    let messages = pending_messages(vec![transfer, kept(TransferState::Started, None)]);
    assert_eq!(messages, vec![suspension]);
}

/// One stored record; `ahead` other writes come first.
struct Store {
    stored: RefCell<Kept>,
    ahead: Cell<u32>,
}

impl Store {
    fn new(ahead: u32) -> Self {
        Self {
            stored: RefCell::new(kept(TransferState::Requested, None)),
            ahead: Cell::new(ahead),
        }
    }
}

impl Records for Store {
    type Record = Kept;
    type Failure = String;

    fn read(&self, lookup: &Lookup<'_>) -> Result<Kept, String> {
        match lookup.caller {
            Some("someone else") => Err("not found".to_owned()),
            _ => Ok(self.stored.borrow().clone()),
        }
    }

    fn replace_if(&self, record: &Kept, expected: u64) -> Result<bool, String> {
        let mut stored = self.stored.borrow_mut();
        if self.ahead.get() > 0 {
            // another write comes first
            self.ahead.set(self.ahead.get() - 1);
            stored.version += 1;
        }
        if stored.version != expected {
            return Ok(false);
        }
        *stored = record.clone();
        Ok(true)
    }

    fn keeps_changing(&self, lookup: &Lookup<'_>) -> String {
        format!("{} keeps changing", lookup.pid)
    }

    fn before_write(&self, record: &mut Kept) {
        record.checked = true;
    }
}

/// A change for `update`: the transfer starts.
#[allow(clippy::unnecessary_wraps)] // the shape `update` asks for
fn start(transfer: &mut Kept) -> Result<Write, String> {
    transfer.state = TransferState::Started;
    Ok(Write::Save)
}

#[test]
fn update_writes_only_over_the_version_it_read() {
    let store = Store::new(1);
    let written = update(&store, &Lookup::own("t"), start).unwrap();
    let written = written.unwrap();
    // read twice: the first write lost against the other one
    assert_eq!(written.version, 2);
    assert!(written.checked);
    assert_eq!(*store.stored.borrow(), written);
}

#[test]
fn update_gives_up_when_other_writes_keep_coming_first() {
    let store = Store::new(u32::try_from(WRITE_ATTEMPTS).unwrap());
    let failed = update(&store, &Lookup::own("t"), start);
    assert_eq!(failed, Err("t keeps changing".to_owned()));
    assert_eq!(store.stored.borrow().state, TransferState::Requested);
}

#[test]
fn update_writes_nothing_when_told_to_skip() {
    let store = Store::new(0);
    let skipped = update(&store, &Lookup::own("t"), |_| Ok(Write::Skip));
    assert_eq!(skipped, Ok(None));
    assert_eq!(store.stored.borrow().version, 0);
    let lookup = Lookup::from_caller("t", "c", "someone else");
    assert_eq!(update(&store, &lookup, start), Err("not found".to_owned()));
}
