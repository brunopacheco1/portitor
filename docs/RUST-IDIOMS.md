<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Reading Portitor's Rust

A short guide to the Rust patterns that Portitor uses, for people who read
Rust only now and then. Each section shows the pattern, what it means in
plain words, and where the code uses it. It is not a Rust course. For
that, see [The Rust Book](https://doc.rust-lang.org/book/).

The code aims to be plain. When a pattern below would hide what the code
does, write the longer form instead. Clippy's `pedantic` lints are on
(`Cargo.toml`), and some of them ask for the short forms, so a few of
these patterns are there because the linter asks for them.

## Values and references

| You see | It means |
|---------|----------|
| `String` | Text that this value owns. |
| `&str` | Borrowed text: read it, don't keep it. |
| `&x` | Borrow `x` to read it. |
| `&mut x` | Borrow `x` to change it. Only one such borrow at a time. |
| `x.clone()` | Make an independent copy. |
| `x.to_owned()`, `x.into()` | Turn borrowed data into owned data, or one type into another. |

## `Option` and `Result`

Rust has no `null` and no exceptions. Instead:

- `Option<T>` is either `Some(value)` or `None`.
- `Result<T, E>` is either `Ok(value)` or `Err(error)`.

A function that can fail returns a `Result`. The caller must look at it.
The compiler will not let you forget.

## The `?` operator

```rust
let negotiation = self.load(provider_pid, "", caller)?;
```

"If this is an error, return the error from this function now.
Otherwise, take the value out and go on." It only works inside a
function that itself returns a `Result` (or an `Option`). `x??` does
this twice, for a `Result` inside a `Result`. `serve.rs` (in
`portitor-runtime`) waits for a background task, which can fail, and then
for the server's own result, which can fail too.

### `?` that converts the error

```rust
impl From<DbError> for StoreError { /* ... */ }

let rows = self.db().query(sql, values![holder])?;   // a DbError becomes a StoreError
```

`?` also changes the error's type when the function returns another
error type and a `From` conversion between the two exists
(`portitor-identity/src/store.rs`). So `?` can hide a conversion: look
for the `impl From`. With `thiserror`, `#[from]` writes that `impl`:

```rust
pub enum CredentialError {
    #[error(transparent)]
    Jws(#[from] Error),       // `?` on a JWS error gives CredentialError::Jws
    /* ... */
}
```

The connector's DCP verifier uses this to sort its failures
(`portitor-connector/src/auth.rs`, `VerifyError`): `?` on a text error
gives `VerifyError::Rejected` (HTTP 404, or 401 on the catalog routes),
and `?` on a failed DID resolution gives `Unavailable` (HTTP 503) when
the server did not answer. Other fetches say which one they are with `fetch_failed`.

### A function name in `map_err`

```rust
tx.execute_batch(sql).map_err(sql_error)?;
```

`map_err` takes any function, not only a closure: `map_err(sql_error)`
is the same as `map_err(|e| sql_error(e))` (`portitor-runtime/src/sqlite.rs`).
A tuple struct's name is a function too: `.map_err(StoreError)` wraps the
text in a `StoreError`.

## `match`, `if let`, `let ... else`

```rust
match handled {
    Ok((ack, next)) => { /* use ack and next */ }
    Err(failure) => failed(failure),
}
```

`match` checks a value against each pattern in order and runs the first
one that fits. The compiler checks that every case is covered.
`Some(old) if old.version == expected =>` adds a condition (a
"guard") to a pattern.

```rust
if let Some(sent) = negotiation.pending.clone() { /* ... */ }
```

`if let` is a `match` with one interesting case: "if this is a `Some`,
call its content `sent`".

```rust
let Some(token) = bearer_token_in(&headers) else {
    return unauthorized();
};
```

`let ... else` says "this must be a `Some`; if not, leave now". After it,
`token` is a plain value (`serve` in `portitor-dataplane/src/serving.rs`).

```rust
if let Some(participant) = caller
    && transfer.counter_party != participant
{ /* ... */ }
```

A *let chain* (Rust 2024): `if let` and other conditions joined with
`&&`. The body runs only if all of them hold (`transfer/provider.rs`).

```rust
match answer {
    Ok(address) => add_start_message(&mut recorded, step, address.clone()),
    Err(reason) => { /* ... */ }
}
```

A `match` on a *reference* (`answer` is a
`&Result<Option<DataAddress>, String>`) takes nothing out of the value:
each name in the pattern is a reference into it. `address` is a
`&Option<DataAddress>`, so the code calls `.clone()` to get a value of
its own. With `&mut` the names are `&mut` too: `if let Some(pending) =
&mut old.pending` lets the code change `pending` in place
(`record_that_step_ran` in `flow_steps.rs`;
`portitor-dsp/src/transfer/provider/tests.rs`).

## More patterns

```rust
let [subject] = credential.subjects.as_slice() else {
    return Err(StatusError::NotAList("not exactly one subject"));
};
```

A *slice pattern* matches a list by its shape: `[only]` is "exactly one
item, call it `only`", `[]` is "empty", `[first, ..]` "at least one"
(`status_list.rs`; `DidDocument::key_for` in `did.rs` matches `[only]`,
`[]` and `_`).

```rust
Err(e @ QueryError::DefinitionNotSupported) => { /* use e */ }
```

`name @ pattern` checks the pattern and also gives the whole matched
value a name (`portitor-identity/src/presentation.rs`).

```rust
let Delivery {
    issuer,
    access_token,
    message,
} = delivery;
```

A *struct pattern* takes a value apart into its fields, each in a
variable of the same name (`portitor-identity/src/storage.rs`). In a
`match`, `Mode::Evaluate { now, .. }` takes only `now` and ignores the
rest (`..`, `policy.rs`). A pattern can also give a field another name,
`Caller { id: caller, .. }`, which reads like a type annotation, so
Portitor avoids it.

```rust
if matches!(mode, Mode::Validate(_)) { /* ... */ }
```

`matches!(value, pattern)` is `true` if the value fits the pattern: a
`match` with one case that answers yes or no (`policy.rs`;
`consumer.rs` checks `Transition::Request | Transition::Accept`).

```rust
while let Some(row) = rows.next()? {
    found.push(credential_from(row)?);
}
```

`while let` repeats while the pattern fits: here, "while there is
another row" (`portitor-identity/src/store.rs`). `JtiCache` uses it with a
let chain to drop expired token ids from the front of a sorted set.

```rust
let Ok(response) = app.clone().oneshot(request).await;
```

A `let` with a pattern and no `else`: allowed only when the pattern
always fits. An axum router never fails, so its error type,
`Infallible`, has no values, and `Ok` is the only case
(`portitor-runtime/src/request_size.rs`, `testing`).

## Building values

```rust
TokenFrame {
    jti: &new_uuid(),
    ..frame
}
```

*Struct update*: a new `TokenFrame` with its own `jti`, and every other
field copied from `frame` (`portitor-identity/src/sts.rs`). Tests often
write `..DidDocument::default()` for "all other fields empty".

```rust
let claims = serde_json::from_value::<ContractNegotiation>(state)
```

`::<T>` (the *turbofish*) tells a generic function which type to
produce, when nothing else in the line says it. Writing the type on the
variable does the same and is often easier to read:
`let claims: ContractNegotiation = serde_json::from_value(state)`.

```rust
let jwts: Result<Vec<String>, Error> = items.iter().map(parse).collect();
```

`collect()` into a `Result<Vec<_>, _>` gathers the values and stops at
the first error. It is short, but an early `return` inside the closure
then returns from the closure, not from the function, which reads
wrongly. Portitor writes a plain loop instead (`credentials_of` in
`credential.rs`).

## Closures: `|x| ...`

```rust
.map_err(|e| bad("", &consumer_pid, &e.to_string()))
```

`|e| ...` is a small function without a name. It takes `e` and returns
what follows. `|| ...` takes nothing. `move || ...` takes ownership of the
variables it uses, which background tasks need.

Closures are often passed to helpers on `Option` and `Result`:

| Helper | Plain words |
|--------|-------------|
| `x.map(\|v\| ...)` | If there is a value, change it. |
| `x.map_err(\|e\| ...)` | If it is an error, change the error. |
| `x.and_then(\|v\| ...)` | If there is a value, run a step that may fail or return nothing. |
| `x.ok_or_else(\|\| err)` | Turn `None` into the error `err`. |
| `x.unwrap_or_default()` | The value, or the empty default (`""`, `0`, ...). |
| `x.unwrap_or_else(\|\| ...)` | The value, or compute a replacement. |
| `x.map_or(default, \|v\| ...)` | Change the value, or give `default` when there is none: `previous.map_or(now, \|p\| p.first_seen)` (`harvest/run.rs`). |
| `x.map_or_else(\|\| ..., \|v\| ...)` | The same, with the default computed only when needed (`db_file` in the connector's `lib.rs`). |
| `x.is_some_and(\|v\| ...)` | There is a value, and the test holds for it. |
| `x.is_none_or(\|v\| ...)` | There is no value, or the test holds for it. |
| `x.filter(\|v\| ...)` | Keep the value only if the test holds. |
| `x.transpose()` | Swap `Option<Result<T>>` and `Result<Option<T>>`. |
| `.iter().any(\|v\| ...)` | At least one item passes the test. |
| `.into_iter().filter_map(\|v\| ...).collect()` | Keep the items the closure returns `Some` for, and gather them in a list. |

A function can take a closure as an argument: `change: impl
Fn(&mut Negotiation) -> Result<Write, Failure>` means "any function that
takes a negotiation it may change". The domain services use this for
their read, change, compare-and-write loop (`update`) and pass the change
in. That way the loop is written only once.

There are three kinds of function arguments. `Fn` may be called any
number of times and changes nothing it captured; `FnMut` may change
what it captured (so it is passed as `&mut`); `FnOnce` is called at most
once (`admit: impl FnOnce(&Claims) -> bool` in
`portitor-runtime/src/caller.rs`).

```rust
pub type SignCredentials<'a> = dyn FnMut(&[u64]) -> Result<SignedRequest, String> + 'a;

fn issue(&self, /* ... */ sign: &mut SignCredentials<'_>) -> Result<IssueOutcome, StoreError>;
```

A `type` alias gives a long function type a name. `SignCredentials` is
"a function that gets the chosen status list positions and returns the
signed credentials". The issuer's store calls it in the middle of its
transaction, after it has picked the positions and before it writes
anything, so a position is used only if the credentials are written
(`portitor-issuer/src/store.rs`). `dyn` because the store is a port: it
takes any such function, chosen while the program runs.

```rust
type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

clock: Arc::new(now_secs),
```

A function kept in a field: `Clock` is "a function that returns the
current time in seconds". The DCP verifier reads the time through it
(`portitor-connector/src/auth/dcp.rs`). It is `now_secs` in the
program; a test puts in a closure that returns a later time, to move
time forward without waiting. `Send + Sync` let it be used from several
requests at once.

```rust
fn named<T>(what: &str, text: &str, from_name: fn(&str) -> Option<T>) -> Result<T, StoreError>
```

`fn(&str) -> Option<T>` (small `fn`) is a *function pointer*: a plain,
named function passed as a value, such as `DatasetState::from_name`.
Unlike `impl Fn(...)` it cannot capture variables, so it is a simple,
fixed type (`portitor-connector/src/store/harvest.rs`).
A function pointer can also sit in a struct field, and is then called
with the field in brackets: `(access.conn)(store)`. Without them,
`access.conn(store)` would look for a method named `conn` (`Access` in
`portitor-runtime/src/sqlite.rs`, which tells the shared lock tests how
to reach each store).

Never write `.unwrap()` outside tests. It stops the program when the
value is missing, and the linter refuses it.

```rust
let ended = retry_while_busy::<_, Box<Failure>>(
    |failure| failure.is_busy(),
    || self.service.push_finished(provider_pid, failure).map_err(Box::new),
)
.await;
```

Two closures given to one function: the first says whether a failure
is worth another try, the second is the call itself, made again after a
short wait while the first says yes (`retry_while_busy` in
`portitor-connector/src/outbox.rs`, used in `transfer.rs`). The second
is an `impl FnMut`: a closure that may change what it captured, here a
counter in the tests. `::<_, Box<Failure>>` names the failure type
(the turbofish, below), so the first closure's `failure` has a known
type before the second closure is read.

## Traits: ports and adapters

A `trait` lists methods that a type promises to have, like an interface.
Every port (ADR 0004) is a trait, for example `NegotiationStore`, and
every adapter implements it (`impl NegotiationStore for
ConnectorStore`).

```rust
pub struct ProviderService<S, D, C, E> { /* ... */ }
impl<S: NegotiationStore, D: Decider, C: CatalogDatasets, E: Environment> ProviderService<S, D, C, E>
```

The service is *generic*. `S` is "some type that implements
`NegotiationStore`". The domain crates never name an adapter.

`dyn NegotiationStore` is "a value of some type that implements
`NegotiationStore`, decided while the program runs". The connector picks
its adapters at start (the database for real use, a fake in a test), so it
keeps each one as `Shared<dyn Port + Send + Sync>`. `Shared` wraps an
`Arc` pointer. See `ports.rs`, which explains why the wrapper exists and
holds all the `Any...` names.

```rust
fn scopes(&self, _out: &Self::Out) -> Vec<String> {
    Vec::new()
}
```

A trait method can have a body: a *default*. A type that implements the
trait may write its own version or keep the default. The connector's
`Outbox` trait answers "no extra scopes" by default; the consumer's
outboxes write their own (`outbox.rs`, `consumer.rs`). A name starting
with `_` says the default does not use that argument.

```rust
pub(crate) trait Record: Clone {
    type State: ProcessState;
    type Outgoing: Clone + PartialEq + Message<<Self::State as ProcessState>::Transition>;
    const PARTY: Role;
    fn parts(&self) -> (Self::State, Option<&Self::Outgoing>, u64);
    fn parts_mut(&mut self) -> (&mut Self::State, &mut Option<Self::Outgoing>, &mut u64);
    fn state(&self) -> Self::State {
        self.parts().0
    }
    /* ... */
    fn apply_sent(&mut self, sent: &Self::Outgoing) -> Result<(), TransitionErrorOf<Self>> {
        apply_sent(self, sent)
    }
}

impl Record for Negotiation {
    type State = NegotiationState;
    type Outgoing = Outgoing;
    const PARTY: Role = Role::Provider;
    /* ... */
}
```

A trait can leave types and constants to each type that implements it:
`type State` and `const PARTY` are *associated* items. Each of the four
process records names its own (`portitor-dsp/src/process.rs`, and the
`impl Record for ...` in each service file). Inside the trait,
`Self::State` is "the state type of whichever record this is". This is
what lets one function serve all four:
`fn receive<R: Record>(record: &mut R, transition: TransitionOf<R>)`
works on a `Negotiation` as on a `ConsumerTransfer`, and
`R::State::next_state(...)` calls the state machine of that record's
process (`ProcessState`, implemented by `NegotiationState` and
`TransferState`).

`type TransitionOf<R> = <<R as Record>::State as ProcessState>::Transition;`
is read from the inside out: the record's `State`, then that state's
`Transition`. The alias exists so that the long form is written once.

Each record writes only `parts` and `parts_mut`, which hand out its
three shared fields (state, message on its way, version) as a tuple,
read-only or to change. The getters and setters (`state`, `set_state`,
`pending`, ...) are defaults built on them, so they are written once,
not four times. `*self.parts_mut().0 = state` reads: take the first of
the three mutable references and write through it (`*`).

`apply_sent` has a default body that only calls the free function
`process::apply_sent`. Rust has no way for an override to call the
default it replaces, so the shared step is a plain function: the
provider's `Negotiation` overrides `apply_sent`, calls
`process::apply_sent` and then keeps the agreement or offer it sent.

```rust
fn from_json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, StoreError>
fn to_json(value: &impl serde::Serialize) -> Result<String, StoreError>
```

A generic helper says what it needs of its type: `T: DeserializeOwned`
is "any type serde can read from JSON", `&impl Serialize` "a borrowed
value of any type serde can write" (`portitor-issuer/src/store.rs`). A
struct can be generic too: `NewCredential<'a, C>` holds `claims: &'a C`,
and `sign_credential<C: Serialize>` accepts it for any claims that
serialize to JSON (`portitor-dcp/src/issuance.rs`).

```rust
authenticate(headers, audience, registered, &*issuer.directory).await
```

`issuer.directory` is an `Arc<dyn Directory>`, a shared pointer. `*`
follows the pointer to the directory and `&` borrows it, so `&*` gives
the `&dyn Directory` that `authenticate` asks for, without copying or
taking the pointer (`portitor-issuer/src/requests.rs`).

```rust
pub fn subject_ids(&self) -> impl Iterator<Item = &str> {
```

`impl Iterator<Item = &str>` as a return type: "something you can loop
over that gives `&str` values". The exact type is long and does not
matter to the caller, who writes `for id in credential.subject_ids()`
(`portitor-dcp/src/credential.rs`; `CredentialTypes::iter` in
`policy/credentials.rs`).

`use base64::Engine as _;` brings a trait into scope without a name:
its methods (here `B64.encode(...)` and `.decode(...)`) can be called,
but the name `Engine` is not used in the file. Rust only lets you call a
trait's methods when the trait is in scope.

`Arc::clone(store) as _` (in `Ports::use_database`) makes a new pointer to
the same store. `as _` lets the compiler turn it into the `dyn` type the
field expects.

```rust
#[cfg(test)]
impl UpstreamSources for BTreeMap<String, String> {
    fn upstream_of(&self, dataset_id: &str) -> Option<String> {
        self.get(dataset_id).cloned()
    }
}
```

A crate may implement its own trait for a type it did not write, even a
standard one: here a plain map of dataset id to URL becomes an adapter
of the connector's `UpstreamSources` port, which `SignalingDataFlows`
reads to name each dataset's source in a start message, so unit tests
need no fake type (`portitor-connector/src/harvest/catalog.rs`). The reverse, someone
else's trait for someone else's type, is not allowed.

```rust
let triples: Box<dyn Iterator<Item = Result<Triple, String>>> = match document.format {
    Format::Turtle => Box::new(/* a Turtle parser */),
    Format::NTriples => Box::new(/* an N-Triples parser */),
    /* ... */
};
```

The arms of a `match` must give one type, and each parser is its own
iterator type. `Box<dyn Iterator<Item = X>>` is "some iterator of `X`,
on the heap", one type that holds either (`portitor-shacl/src/graph.rs`).

```rust
pub trait ProviderFlowStore: FlowStore + MembershipStore {}

impl<T: FlowStore + MembershipStore> ProviderFlowStore for T {}
```

A `dyn` type may name only one trait of its own, plus `Send` and
`Sync`: `dyn FlowStore + MembershipStore` is not allowed. So
`ProviderFlowStore` is a trait that only names the other two (they are
its *supertraits*), and the second line, a *blanket impl*, says "every
type that implements both implements this one too". No type writes
`impl ProviderFlowStore` by hand. The connector keeps its flow store as
`Shared<dyn ProviderFlowStore + Send + Sync>` (`AnyFlowStore` in
`portitor-connector/src/ports.rs`): one value that is both the data
plane's `FlowStore` and the control plane's `MembershipStore`.

```rust
pub fn flows_only(&self) -> Arc<dyn FlowStore + Send + Sync> {
    Arc::clone(&self.0) as Arc<dyn FlowStore + Send + Sync>
}
```

*Trait upcasting* (Rust 1.86; Portitor needs 1.89): a `dyn ProviderFlowStore` can be used as
a `dyn FlowStore`, one of its supertraits, with `as`. The pointer is
copied, the store is not: both point to the same database. The data
plane gets only this view, so it cannot read the memberships
(`flows_only` in `ports.rs`).

```rust
// plane.rs
pub struct HttpDataPlane { /* ... */ }
impl HttpDataPlane { pub fn new(/* ... */) /* ... */ }
// serving.rs
impl HttpDataPlane { pub fn source_if_authorized(/* ... */) /* ... */ }
```

One type may have `impl` blocks in several files of its crate. The
struct `HttpDataPlane` is defined in `portitor-dataplane/src/plane.rs`;
`signaling.rs`, `serving.rs`, `sources.rs` and `push.rs` each add the
methods of their topic. To find a method, search the crate for
`fn <name>`; the table in the crate's `lib.rs` says which file holds
which topic.

```rust
impl DataPlane for &Plane { /* ... */ }
```

A trait implemented for a *reference* to a type, not for the type. The
test data plane in `portitor-connector/src/data_flows.rs` is used as
`SignalingDataFlows<&Plane>` (`flows_over`): the adapter holds a borrowed
`&Plane`, and the test can still read `plane` afterwards to see which
signals arrived.

```rust
impl MembershipGate {
    pub async fn check_memberships(&self, flow_id: &str) -> Access { /* ... */ }
}
impl AccessGate for MembershipGate {
    fn check<'a>(&'a self, flow_id: &'a str) -> BoxFuture<'a, Access> {
        Box::pin(self.check_memberships(flow_id))
    }
}
```

The trait method wraps the type's own `async fn` in the box its trait
asks for (`portitor-connector/src/flow_membership.rs`). The two have
different names on purpose: if they had the same name, a call on the
type would pick the type's own method first, and a reader could take
the trait method for one that calls itself.

### A type that names what it does not hold: `PhantomData`

```rust
pub trait ComponentSchema {
    const MIGRATIONS: &'static Migrations;
}
pub struct ComponentStore<S> {
    database: Arc<Database>,
    schema: PhantomData<S>,
}
pub struct IdentitySchema;
impl ComponentSchema for IdentitySchema {
    const MIGRATIONS: &'static Migrations = &MIGRATIONS;
}
pub type DatabaseStore = ComponentStore<IdentitySchema>;
impl IdentityStore for DatabaseStore { /* ... */ }
```

The identity's and the issuer's stores are the same wrapper around the
database; only the migrations differ (TD-71). `ComponentStore`
(`portitor-runtime/src/component_store.rs`) is written once, generic
over `S`, a type that holds nothing and only names the schema:
`S::MIGRATIONS` is a constant of the trait, read by `open` and
`in_memory`. A struct must use each of its type parameters in a field,
so `PhantomData<S>` is a field of size zero that "uses" `S`.

`pub type DatabaseStore = ...` gives the type a short name in each
component, so `DatabaseStore::open(...)` reads as before. Each
component then implements its own port (`IdentityStore`,
`IssuerStore`) for its own `ComponentStore<...>`; Rust allows that
because the port is the component's own trait. The shared port
methods (used token ids, leases) are implemented once, in
`portitor-runtime`, for every `ComponentStore<S>`.

```rust
pub trait WithLeaseTable: ComponentSchema {}
impl<S: WithLeaseTable + Send + Sync> LeaseStore for ComponentStore<S> { /* ... */ }
impl WithLeaseTable for IdentitySchema {}
```

A *marker trait*: it has no methods, it only marks a type. Only the
identity's schema has the `lease` table, so only its store is a
`LeaseStore`; asking the issuer's store for a lease does not compile,
instead of failing at run time with "no such table".

### `BoxFuture` in a trait

A trait whose objects are kept as `dyn Trait` cannot have an `async fn`
yet. Its method returns a `BoxFuture` instead: a boxed task, written
with `Box::pin(async move { ... })` (["`async`, `.await` and background
tasks"](#async-await-and-background-tasks)). The data plane's
`AccessGate` port does this, so the data plane can hold the control
plane's gate as `Arc<dyn AccessGate>` without knowing its type
(`portitor-dataplane/src/access.rs`).

### One function for several inputs: `impl Into<Terms<'a>>`

```rust
impl<'a> From<&'a Offer> for Terms<'a> { /* ... */ }
impl<'a> From<&'a Agreement> for Terms<'a> { /* ... */ }
impl<'a> From<&'a AccessPolicy> for Terms<'a> { /* ... */ }

pub fn credential_scopes<'a>(policy: impl Into<Terms<'a>>, types: &CredentialTypes) -> Vec<String> {
    let terms = policy.into();
    /* ... */
}
```

`Terms` is a view of the rules of an offer, an agreement or an access
policy. It borrows them and copies nothing. Each of the three types
says how it becomes `Terms` (`impl From<...> for Terms`); Rust then
gives `Into` for free, the same conversion seen from the other side. A function
that takes `impl Into<Terms<'a>>` then accepts any of them:
`credential_scopes(&offer, types)` and `credential_scopes(&policy,
types)` both work. Its first line, `let terms = policy.into();`, makes
the view (`portitor-dsp/src/policy.rs`, `policy/access.rs`).

## Sharing between threads: `Arc` and `Mutex`

- `Arc<T>` is a pointer that many owners share. `Arc::clone` copies the
  pointer, not the value.
- `Mutex<T>` lets one thread at a time change `T`. `lock()` waits for
  your turn.
- `lock().unwrap_or_else(PoisonError::into_inner)` keeps going even if
  another thread panicked while it held the lock. Each place that does
  this says in a comment why it is safe there.
- `lock()` returns a *guard*; the lock is held until the guard is
  dropped. `let _one_at_a_time = ...lock()...;` keeps the guard in a
  named variable, so the lock is held until the end of the function
  (`refresh` in `portitor-issuer/src/status_lists.rs`). `let _ =
  ...lock()...;` would drop it, and free the lock, at once: the leading
  `_` in a name keeps the value, the bare `_` does not.
- `Send` and `Sync` in a type mean "safe to move to, or share with,
  another thread". The web server needs them.
- `mutex.try_lock()` takes the lock only if it is free, and does not
  wait. `let Ok(_running) = self.busy.try_lock() else { ... return }`
  skips a harvest run when another one is going on (`DatasetHarvester::run` in
  `portitor-connector/src/harvest.rs`): a timer tick that comes during a
  long run is dropped, not queued.

- `AtomicBool` is a `bool` that threads can share without a lock:
  `store(true, Ordering::SeqCst)` sets it, `load(Ordering::SeqCst)` reads
  it. `SeqCst` is the simplest and strictest ordering, and Portitor's
  default (`started` in `DatasetHarvester`, `portitor-connector/src/harvest.rs`).
- `AtomicU64` is the same for a number. The push sender and the
  receiving route count the bytes of a body as it passes:
  `body.map_frame(...)` sees each frame, and the closure adds its length
  with `fetch_add(n, Ordering::Relaxed)` (`post` in
  `portitor-dataplane/src/push.rs`, `receive` in
  `portitor-connector/src/receiving.rs`). The
  total is read once, after the body has ended, for the audit line.
  `Relaxed` is enough for a counter that nothing else depends on; use
  `SeqCst` everywhere else.

### A snapshot swapped as a whole: `RwLock<Arc<View>>`

```rust
pub struct BundleCopy(Arc<RwLock<Arc<BundleRules>>>);

pub fn current(&self) -> Arc<BundleRules> {
    Arc::clone(&self.0.read().unwrap_or_else(PoisonError::into_inner))
}
```

`RwLock` lets many readers in at once, or one writer. The value inside
is itself an `Arc`, so a reader holds the lock only to copy that
pointer, then works on its own snapshot without any lock. A new bundle
version builds whole new rules and swaps the pointer in one write
(`BundleCopy::replace` in `portitor-connector/src/bundle_harvest.rs`). A
message is never checked with half of a version, and a slow check
never blocks the swap. The old rules are freed when their last reader
drops them.

```rust
let mut in_use = self.0.write().unwrap_or_else(PoisonError::into_inner);
*in_use = Arc::new(rules);
```

`self.0` is the first (here the only) field of a *tuple struct*, a
struct whose fields have no names: `BundleCopy(Arc<...>)`. `StartError(e.0)`
and `Shared(Arc::clone(store) as _)` in the connector use the same
form. `write()` returns a guard; `*guard = value` replaces the value the
guard protects, and the guard is dropped (the lock freed) when it goes
out of use; `BundleCopy::replace` drops it early, with `drop(in_use)`.

## Panics

A panic is Rust's answer to a bug it finds while running (an index out
of range, `expect` on an error). It *unwinds*: the function stops, and
so does its caller, and so on up, and every value on the way is dropped
(locks are freed, an open database transaction is rolled back). Portitor
stops the unwinding at the edge of one piece of work
(`portitor-runtime/src/panics.rs`):

```rust
let mut work: Pin<Box<F>> = Box::pin(work);
std::future::poll_fn(move |context| {
    let step = AssertUnwindSafe(|| work.as_mut().poll(context));
    match std::panic::catch_unwind(step) {
        Ok(Poll::Ready(output)) => Poll::Ready(Ok(output)),
        Ok(Poll::Pending) => Poll::Pending,
        Err(_message) => Poll::Ready(Err(Panicked)),
    }
})
```

- `catch_unwind(closure)` runs the closure and returns `Err` if it
  panicked, instead of letting the panic travel on.
- `AssertUnwindSafe(...)` is a promise to the compiler: "data this code
  was changing may be left half changed, and that is fine here". It is
  fine because shared data sits behind locks that say how they recover
  (the bullet on `PoisonError` above) and in database transactions.
- `poll` runs an async task up to its next wait. `catch_panic` wraps
  each such step, so a task is covered from start to end.
  `std::future::poll_fn(closure)` makes a task out of a closure that is
  called for every step; `Poll::Ready(x)` is "done, with `x`",
  `Poll::Pending` is "not yet".
- `Pin<Box<F>>`, made by `Box::pin(work)`: the task `work` (of type `F`)
  is put on the heap (`Box`) and may not be moved from there (`Pin`). A
  task that is half done can hold pointers into itself, so Rust lets
  `poll` be called only on a task that is pinned. Code that just
  `.await`s never sees this; `catch_panic` calls `poll` by hand, so it
  pins the task first.
- `work.as_mut().poll(context)`: `poll` takes the pinned task by value,
  and `as_mut()` lends it for this one call, so that `work` can be
  polled again at the next step. `context` is how the task says "wake
  me when I can go on"; it is only passed along.

Work that is not async needs none of this: `catch_sync_panic(|| ...)`
runs the closure inside one `catch_unwind`.

Use the helpers, not `catch_unwind` itself:
`answer_500_on_panic(router)` for HTTP handlers (already on every
server, `serve.rs`), `spawn_supervised(name, || task())` instead of
`tokio::spawn` for a task that runs as long as the process (the closure
makes the task again after a panic), `spawn_every(name, every, || round())`
for such a task that does one round every few seconds or minutes, `report_panic(name, task)` around
a task that does one piece of work, and `report_sync_panic(name, || ...)`
around one piece of work in a loop that is not async (one credential
decision of many, `decide_guarded` in `portitor-issuer/src/issue.rs`).

```rust
let mut pause: Option<Duration> = None;
loop {
    let started = tokio::time::Instant::now();
    if catch_panic(start()).await.is_ok() {
        return;
    }
    let wait = next_pause(pause, started.elapsed());
    /* log, count */
    tokio::time::sleep(wait).await;
    pause = Some(wait);
}
```

The restart loop of `spawn_supervised`. `next_pause` is a small pure
function: 5 seconds after the first panic, twice the last pause after
each further one, at most 5 minutes, and 5 seconds again after a run
that lasted longer than that. `started.elapsed()` is how long the run
took. `tokio::time::Instant`, not `std::time::Instant`, so that a test
with a paused clock (`#[tokio::test(start_paused = true)]`) can read
the pauses without waiting for them.

```rust
match catch_panic(send_and_record(outbox.as_ref(), &out)).await {
    Ok(Some(next)) => out = next,
    Ok(None) => return,
    Err(Panicked) => { /* log, count, wait: the loop sends `out` again */ }
}
```

The outbox does the same for one message (`outbox.rs`). The step gets
`&out`, a loan: when it panics, the loop still owns `out` and can send
it again. Had the step taken `out` itself, the message would be gone
with the panic. `outbox.as_ref()` turns the `&Arc<O>` into the plain
`&O` the step takes.

## Database transactions

```rust
let outcome = self.db().transaction(|tx| {
    let Some(holder_did) = holder_if_received(tx, issuer_pid)? else {
        return Ok(IssueOutcome::NotReceived);
    };
    /* ... more checks, then the writes ... */
    tx.execute(
        "INSERT INTO outbox (issuer_pid, message, attempts, next_attempt_at)
             VALUES ($1, $2, 0, $3)",
        values![issuer_pid, &signed.message_json, to_db_int(rules.now)],
    )?;
    Ok(IssueOutcome::Issued)
})?;
```

A transaction groups reads and writes so they happen all together or
not at all. `Database::transaction` (`portitor-runtime/src/database.rs`)
takes the work as a closure, `|tx| { ... }`, and gives it `tx`, the
transaction to read and write through (a `&dyn Executor`). When the
closure returns `Ok`, everything it wrote is kept; when it returns `Err`
(an early `return Err(...)` or a `?`), or panics, nothing is
(`issue` in `portitor-issuer/src/store.rs`).

On PostgreSQL the closure may run more than once: when two transactions
collide, PostgreSQL cancels one of them, and `transaction` runs its
closure again from the start. So the closure must only read and write
through `tx`, with no other effect (no message sent, no counter in
memory changed). This is why the work is a closure, and why it is an
`FnMut` (a closure that can be called again) and not an `FnOnce`.

Before it runs again, `transaction` gives its connection back to the
pool and pauses for a random time, from 0 up to a ceiling that doubles
with each collision (5 ms, 10 ms, ... at most 200 ms): "exponential
backoff with full jitter". With a fixed pause, the two transactions that
collided would wake together and collide again. It stops after 5
seconds or 50 tries and returns the database's error
(`pause_after_collision` in `portitor-runtime/src/database/postgres.rs`).

The same SQL runs on PostgreSQL and on `SQLite`:

- placeholders are `$1`, `$2`, ... (PostgreSQL's form); for `SQLite`
  they are rewritten to `?1`, `?2`, ...;
- `values![a, b, c]` turns each value into a `Value` (text, a whole
  number, bytes or null) and makes the list the placeholders take, in
  order: `a` is `$1`. A list that is built in pieces is a `Vec<Value>`,
  made with `vec![Value::from(a), ...]`;
- numbers are stored as signed 64-bit numbers: `to_db_int(seconds)` and
  `from_db_int(stored)` convert the `u64` seconds and counts.

`query_one` returns `Option<Row>`: the first row, or `None` when there
is none. `row.get::<String>(0)` reads column 0 as text, and fails when
the column holds something else.

```sql
INSERT INTO lease (kind, id, lease_holder, lease_until) VALUES ($1, $2, $3, $4)
ON CONFLICT (kind, id) DO UPDATE
    SET lease_holder = excluded.lease_holder, lease_until = excluded.lease_until
    WHERE lease.lease_until <= $5
```

A check and a write in one statement. The row is inserted; when one
with this kind and id is there already, it is updated instead, but only
if its lease has ended (the `WHERE`). `execute` returns how many rows
were written, and `changed == 1` tells the caller whether it got the
lease. One statement is atomic: two replicas that claim at once cannot
both pass the check, which "look first, then write" in two statements
would allow (`claim` in `portitor-runtime/src/leases.rs`).

### Waiting for PostgreSQL: `block_on`

The store ports are plain functions, but the PostgreSQL client
(`tokio-postgres`) is `async`. `postgres.rs` (in
`portitor-runtime/src/database/`) bridges the two: the connections are
driven by a thread of their own, and a store call waits for its answer
with `block_on(future)`, which parks the calling thread until the
database thread wakes it. The calling thread is often one of tokio's
workers, which waits meanwhile (TD-40). The module doc says why tokio's
own `Handle::block_on` and `spawn_blocking` are not used.

### Passing a call on to the value inside: `Pin::new(&mut self.0)`

```rust
impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for RustlsStream<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>)
        -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}
```

`RustlsStream` (`portitor-runtime/src/database/postgres_tls.rs`) wraps
a TLS stream only to give it a type of Portitor's own, which the
PostgreSQL client's traits can be implemented for. Each method passes
the call on to the stream inside (`self.0`). The methods take
`self: Pin<&mut Self>` (see "A response body written by hand" below);
`Pin::new(&mut self.0)` pins the inner stream again for its own call.
That is allowed because the stream is `Unpin` (it may move), which the
`S: ... + Unpin` bound says.

## Printing for logs: `Debug`

`#[derive(Debug)]` prints every field. Types that hold secrets or large
values write `Debug` by hand instead:

```rust
impl std::fmt::Debug for DcpVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DcpVerifier")
            .field("participant_id", &self.participant_id)
            .finish_non_exhaustive()
    }
}
```

It prints the fields named and `..` for the rest
(`finish_non_exhaustive`), so a key or token never reaches a log
(`auth/dcp.rs`; `SigningKey` in `portitor-dcp/src/keys.rs` prints only
its public key's thumbprint).

## Log fields: `tracing`

```rust
tracing::warn!(%pid, %error, job = id, check = error.kind.name(), "harvest: dataset rejected");
tracing::info!(?access_rights, "dataset checked");
```

Each name before the message is a field of the log line. `job = id`
logs a number or text as it is. `%error` is short for `error = %error`
and logs the value with `Display` (the text meant for people); `?value`
logs it with `Debug` (the developer's view, for example a list or a
struct). A bare name like `outcome` logs the variable `outcome` with its
own name. The harvest's log lines use all three
(`portitor-connector/src/harvest.rs`; `access_rights = ?access_rights` in
`shacl.rs`).

### Spans and log targets

```rust
let span = tracing::info_span!("http.server.request", http.route = route.as_deref(), "error.type" = Empty);
let response = next.run(request).instrument(span.clone()).await;
span.record("error.type", status.to_string());
```

A *span* is a stretch of work with a name and fields; log lines written
while it runs belong to it. `.instrument(span)` runs a task inside the
span every time the task is polled, also across `.await`s (a plain
`span.enter()` would not survive an `.await`). `Empty` declares a field
filled in later with `span.record`. A field name in quotes
(`"error.type"`) is needed when a part is a Rust keyword. `otel.name`,
`otel.kind` and `otel.status_code` are read by the OpenTelemetry layer
as the span's name, kind and status (`portitor-runtime/src/telemetry/http.rs`).

A field whose value is an `Option` is written only when it is `Some`:
`portitor.dsp.role = kind.role_name()` leaves the field out of the span
for an outbox without a role (`outbox.rs`), and `http.route =
route.as_deref()` for a request that matched no route.

`tracing::event!(target: STATE_EVENTS, tracing::Level::INFO, ...)`
writes a log line with a chosen *target* instead of the module path; the
OpenTelemetry layer copies only events with that target into traces
(`portitor-connector/src/state_events.rs`). `event!` with the level
spelled out is used there because `info!(target: ..., a.b = ...)` does
not parse when the first field name has dots.

`#[async_trait::async_trait]` on an `impl` block: the trait (here
opentelemetry-http's `HttpClient`) was declared with that macro, so its
`async fn` is really a function returning a boxed task, as `BoxFuture`
does by hand (`OnTokio` in `portitor-runtime/src/telemetry.rs`).

## `async`, `.await` and background tasks

`async fn` returns a task that runs when someone `.await`s it. While it
waits on the network, the thread does other work. `tokio::spawn(task)`
runs a task in the background. The outbox uses it to send DSP messages
after the HTTP answer has gone out.

`BoxFuture<'a, T>` (`portitor-runtime/src/lib.rs`) is a boxed task that
finishes later with a `T`. Every port with async methods returns one:
the `Sender`, the connector's `CallerVerifier` and `Prover`, the
`Directory` of the identity component and the issuer. Its comment explains why these traits
cannot use `async fn` directly. Read `BoxFuture<'a, Result<Value,
String>>` as "later, a `Value` or an error". (`AuthFuture` in the
connector is another name for the same thing.)

```rust
tokio::select! {
    _ = tokio::signal::ctrl_c() => {}
    _ = term.recv() => {}
}
```

`tokio::select!` waits for several tasks at once and goes on with the
first that finishes; the others are dropped. Here: stop on Ctrl-C or on
SIGTERM, whichever comes first (`portitor-runtime/src/serve.rs`).

```rust
async fn within_deadline<T>(
    &self,
    what: &str,
    work: impl Future<Output = Result<T, String>>,
) -> Result<T, String>
```

A function can take a task as an argument: `impl Future<Output = X>` is
"a task that finishes with an `X`, not started yet". `T` is any type:
the helper does not care what the task produces. Here it runs `work`
with a time limit (`tokio::time::timeout`), and the connector wraps
every caller check in it (`auth/dcp.rs`).

```rust
let exchange = async {
    let response = self.client.request(request).await?;
    /* ... read the body ... */
};
match tokio::time::timeout(DOWNLOAD_TIMEOUT, exchange).await { /* ... */ }
```

An `async { ... }` block is a task written in place, not started yet. Kept
in a variable, it runs only when awaited, here inside `timeout`, which
stops it when the time is up (`UrlSource::get` in
`portitor-bundle/src/source.rs`).

```rust
pub fn schedule(self: &Arc<Self>, every: Duration, leases: &Leases) -> Option<JoinHandle<()>> {
    let harvester = Arc::clone(self);
    let leases = leases.clone();
    let round = move || {
        let harvester = Arc::clone(&harvester);
        let leases = leases.clone();
        async move { run_if_due(&harvester, &leases, every).await }
    };
    Some(spawn_every("dataset_harvest", every, round))
}
```

`round` is a closure that makes the work of one round: `spawn_every`
calls it on each tick of a timer, inside `spawn_supervised` ("Panics"
above). Each call clones the pointers, so every round has its own.

```rust
let first = tokio::time::Instant::now() + every;
let mut ticks = tokio::time::interval_at(first, every);
ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
loop {
    ticks.tick().await;
    round().await;
}
```

The timer inside `spawn_every` (`portitor-runtime/src/panics.rs`),
written once for every loop that ticks. `interval_at(first, every)`
ticks at `first`, then every `every`; the first round is one period
after the start. `Skip`: when a round lasted longer than the period,
the tick that was due runs at once, and the ticks missed meanwhile are
dropped, so rounds never pile up behind a slow one.

`self: &Arc<Self>` is a method that can be called only on a value kept in
an `Arc`. It needs that, as the background task keeps its own pointer
(`Arc::clone(self)`) after the method returns.

`tokio::task::spawn_blocking(move || ...)` runs work that blocks the
thread (reading a file, `SQLite`, a long computation) on a separate pool
of threads, so the async threads stay free for requests. Awaiting it
gives the closure's result, or an error if it panicked
(`run_if_due` in `portitor-connector/src/harvest.rs`).

```rust
tokio::select! {
    () = issuer.outbox.notified() => {}
    () = tokio::time::sleep(wait) => {}
}
```

`tokio::sync::Notify` is a doorbell between tasks: `notify_one()` rings
it, `notified()` waits for a ring. The issuer's outbox sleeps until the
next delivery is due, or until a newly issued request rings
(`portitor-issuer/src/delivery.rs`, `issue.rs`).

```rust
let _running = self.busy.lock().await;
let fetched = self.source.fetch(/* ... */).await;
```

`tokio::sync::Mutex` is a lock that may be held across an `.await`; the
`std::sync::Mutex` of the section above must not be. The bundle
harvester holds it for a whole run, downloads included, so two runs
never overlap (`BundleHarvester` in
`portitor-connector/src/bundle_harvest.rs`).

```rust
let refreshing = async move {
    loop {
        check.refresh().await;
        tokio::time::sleep(IDENTITY_REFRESH).await;
    }
};
spawn_supervised("identity_check", /* a closure that makes `refreshing` */)
```

A background task that runs until the process stops: it checks our
identity every 10 seconds and keeps the result, and the readiness probe
only reads that result (`OwnIdentityCheck` in
`portitor-connector/src/health.rs`). `.instrument(span)` runs the task in
`span`; `Span::none()` is no span at all, so these runs start no trace.

```rust
let mut running = Vec::new();
for (name, check) in &self.checks {
    running.push((name, tokio::spawn(/* the check, with a timeout */)));
}
for (name, task) in running {
    let outcome = task.await; /* ... */
}
```

Start every task first, then await them one by one: they run at the
same time, so the whole takes as long as the slowest, not the sum
(`HealthChecks::run` in `portitor-runtime/src/health.rs`).

```rust
pub(crate) fn deliver<O: Outbox>(outbox: Arc<O>, out: O::Out) -> impl Future<Output = ()> {
    let span = tracing::info_span!("dsp.outbox.deliver", /* ... */);
    send_until_counter_party_acts(outbox, out).instrument(span)
}
```

A plain `fn` that returns `impl Future` instead of an `async fn`: both
give a task to await or spawn, but the body of an `async fn` runs only
when the task first runs, while this body runs at the call. Here that
matters: the span is made while the request that saved the message is
still the current span, and the task, spawned with `tokio::spawn`, runs
later inside it (`outbox.rs`). Spawning a task with
`.instrument(span)` is how a background task keeps the trace of the
request that started it.

```rust
fn post<'a>(&'a self, url: &'a str, body: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
    Box::pin(async move { self.json(json_post(url, body, None)?, url).await })
}
```

`Box::pin(async move { ... })` is how a method that returns a
`BoxFuture` is written: an `async` block (the task), `move` to take its
arguments in, boxed and pinned (`BoxFuture` above). `HttpSender` in
`portitor-runtime/src/http.rs`.

```rust
let exchange = async move {
    let response = client.request(request).await?;
    /* ... */
    Ok::<_, opentelemetry_http::HttpError>(response)
};
```

An `async` block has no declared return type, so when it uses `?` the
compiler cannot tell which error type it returns. `Ok::<_, E>(value)`
names it: `_` lets the compiler fill in the value's type, `E` is the
error (`export` in `telemetry.rs`).

```rust
let response = match tokio::time::timeout(UPSTREAM_TIMEOUT, sent).await {
    Ok(Ok(answer)) => answer,      // in time, and the request worked
    Ok(Err(error)) => /* ... */,   // in time, but the request failed
    Err(_) => /* ... */,           // no answer in time
};
```

`timeout` wraps the task's own `Result` in one of its own, so there are
two: the outer says whether it ended in time, the inner what the task
gave (`fetch` in `portitor-dataplane/src/sources.rs`).

```rust
let (stop, stopped) = tokio::sync::watch::channel(false);
/* each server: */ .with_graceful_shutdown(wait(stopped.clone()))
/* on Ctrl-C or SIGTERM: */ let _ = stop.send(true);

pub async fn wait(mut stopped: tokio::sync::watch::Receiver<bool>) {
    let _ = stopped.wait_for(|s| *s).await;
}
```

`tokio::sync::watch` is one value that many tasks can watch: `stop`
(the sender) sets it, and every copy of `stopped` (the receivers) sees
the latest value. Here the value is "stop now?", `false` at first.
`wait_for(|s| *s)` waits until it is `true`, also when it was set
before the wait began, which a doorbell (`Notify`) would miss. One
`stop.send(true)` so stops the public and the management server
(`serve.rs`).

```rust
management.await.map_err(std::io::Error::other)??;
```

`management` is a background task whose own work returns a `Result`,
so awaiting it gives a result inside a result: the outer one is the
task's (an error if it panicked), the inner one the server's.
`map_err(std::io::Error::other)` gives the outer error the type of the
inner one, the first `?` returns it, and the second `?` returns the
server's error (`serve_with_trusted_routes` and `stop_all_when_ended`
in `serve.rs`).

```rust
fn without_delay(listener: TcpListener) -> impl axum::serve::Listener<Addr = std::net::SocketAddr> {
    use axum::serve::ListenerExt;
    listener.tap_io(|connection| {
        let _ = connection.set_nodelay(true);
    })
}
```

`ListenerExt` is an *extension trait*: a trait that only adds methods
to a type from another crate, here `tap_io` on a `TcpListener`. The
methods exist only where the trait is imported, hence the `use` inside
the function. `tap_io(closure)` gives a listener that works as before
and also calls the closure on every connection it accepts, before the
connection is served. Here the closure sets `TCP_NODELAY`. `let _ =`
drops the result on purpose: the option only changes timing. The
return type `impl Listener<Addr = SocketAddr>` is "some listener whose
addresses are socket addresses"; the real type is long and of no
interest (`serve.rs`).

### Cutting running tasks: a dropped `watch::Sender`

```rust
switches: Mutex<BTreeMap<String, watch::Sender<()>>>
// cut: remove the sender; every receiver wakes up
let removed = self.switches().remove(flow_id);
```

`RunningFlows` (`portitor-dataplane/src/running_flows.rs`) stops every
download of one data flow at once. Each download holds a
`watch::Receiver` (in a `CutSignal`); `cut` removes the flow's
`watch::Sender` from the map, and dropping it is the signal: every
receiver's `changed().await` returns an error, so each download learns
that it must stop. No value is ever sent (the channel carries `()`).
The order matters and is written in the file's comment: a download
watches first and checks the flow's state second; the data plane
changes the state first and cuts second. Then no download can start
between the two and miss the cut.

### A response body written by hand: `impl Body`

```rust
impl hyper::body::Body for CutBody {
    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>)
        -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> { ... }
}
```

A response body is a stream of frames that the server asks for one at
a time with `poll_frame`. `CutBody` (`portitor-dataplane/src/download.rs`) wraps the
upstream's body: on each call it first looks at the cut signal, and if
the flow was cut it answers with an error, which makes the server drop
the connection. Otherwise it passes the upstream's frame on.
`Poll::Pending` means "not yet, I will wake you". `Pin<&mut Self>`
says the value may not move in memory while it is polled; most code
only needs `self.get_mut()` or a pinned box (`Pin<Box<dyn Future>>`) to
poll a future it holds.

The download's membership check runs as a background task next to the
body (`membership_watchdog` in `portitor-dataplane/src/download.rs`). `AbortOnDrop` holds its `JoinHandle` and calls
`abort()` in `Drop`, so the task ends when the body ends, however it
ends.

### A step of hyper's client written by hand: `tower_service::Service`

```rust
/// A task that finishes later with a `T` or an error.
type Later<T> = Pin<Box<dyn Future<Output = Result<T, BoxError>> + Send>>;

impl tower_service::Service<Uri> for CheckedConnector {
    type Response = TokioIo<TcpStream>;
    type Error = BoxError;
    type Future = Later<Self::Response>;
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> { ... }
    fn call(&mut self, uri: Uri) -> Self::Future { Box::pin(async move { ... }) }
}
```

hyper's client opens a connection by calling a "service": `call` with
the URL gives, later, the open connection. Resolving a host name is a
service too (`call` with the name gives the addresses). Read `call` as
an `async fn` whose result type is spelled out (`type Future`); the
body is an `async move` block in a `Box::pin`. `Later<T>` is a type
alias: a short name for that long type, written once and used by both
services. `poll_ready` asks "can
you take a call now?"; ours always can, or asks the connector it wraps.
`CheckedConnector` and `CheckedResolver` in
`portitor-runtime/src/http/connect.rs` use this to check the address a
call connects to (R-17). A refusal travels as a boxed error, and
`refusal_in` finds it again in hyper's error (see
[Finding an error inside another](#finding-an-error-inside-another-source-and-downcast_ref)).

### Set once, later: `OnceLock` and `Weak`

```rust
ender: OnceLock<Arc<dyn EndsTransfers>>,
...
let ender = transfer::EndsThroughProvider(Arc::downgrade(&transfers));
gate.ends_transfers_with(Arc::new(ender));
```

The data plane is built before the provider's transfer service, but
must call it to end a transfer. `OnceLock` is a slot written once
after construction (`set`; a second `set` changes nothing) and read
without a lock (`get`). `Arc::downgrade` gives a `Weak`: a reference
that does not keep the service alive, so the two do not hold each other
forever; `upgrade()` gives an `Arc` back while the service exists
(`flow_membership.rs`, `transfer.rs`).

## Lifetimes: `'a`

`fn post<'a>(&'a self, url: &'a str, ...) -> BoxFuture<'a, ...>` says "the
returned task borrows `self` and `url`, so it must not outlive them".
Lifetimes don't change what the code does. They let the compiler check
borrows.

A struct can borrow too:

```rust
pub struct TokenFrame<'a> {
    pub audience: &'a str,
    pub jti: &'a str,
    /* ... */
}
```

"A `TokenFrame` holds borrowed text, so it must not outlive the strings
it points to" (`portitor-dcp/src/holder.rs`; also `Jws<'a>`, which keeps a
slice of the token it was read from, and `Signer<'a>` in `keys.rs`). It saves copying
the text for a value that only lives during one call.

### `'static`

`&'static str` is text that lives as long as the program: in practice a
literal written in the code, such as `"portitor-connector"`. A function
asks for it when it keeps the text for good (`telemetry::init`, the
service name) or when the value must come from the code, not from a
request (the metric attributes in `telemetry/metrics.rs`).
`&'static [&'static str]` is the same for a list of texts:
`&["/sts/token"]`, the trusted routes of `observe_requests`.

A `'static` bound on a type, as in `F: Fn() -> ... + Send + Sync +
'static` (`health::blocking`), says that the value borrows nothing that
could end before it: it owns what it needs, often taken in with `move`.
`tokio::spawn` and `std::thread::spawn` ask for it, because the task may
outlive the function that started it.

### Higher-ranked bounds: `for<'a>`

```rust
S: Subscriber + for<'lookup> LookupSpan<'lookup>,
```

"For every lifetime `'lookup`": the subscriber can look up spans however
long the borrow is. It appears where Portitor plugs into
tracing-subscriber (`telemetry.rs`), copied from that library's docs,
which ask for it. Nothing to decide there; read it as part of the trait's
name.

## Errors in a `Box`

Many connector functions return `Result<T, Box<Failure>>`. A `Failure`
carries a whole DSP error message, and boxing it keeps every `Result`
small. Clippy's `result_large_err` lint asks for this. Read
`Box<Failure>` as `Failure`.

```rust
) -> Result<JsonLdRemoteDocument, Box<dyn std::error::Error + Send + Sync>> {
    Err(format!("context {url} is not among the local contexts").into())
```

`Box<dyn std::error::Error + Send + Sync>` is "any error type", the type
a library asks for when it accepts errors of every kind (here oxjsonld's
context loader). `.into()` turns a `String` into such a boxed error
(`portitor-shacl/src/jsonld.rs`).

### Finding an error inside another: `source()` and `downcast_ref`

```rust
let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
while let Some(error) = current {
    if let Some(refused) = error.downcast_ref::<InternalAddressRefused>() {
        return Some(refused);
    }
    current = error.source();
}
None
```

A library may wrap our error in its own: hyper's client error holds the
connector's error, which holds ours. `source()` gives the error one
level down, or `None` at the bottom; the loop walks down level by level.
`downcast_ref::<T>()` asks "is this error a `T`?" and gives it back as a
`T` if it is. So `refusal_in` finds our refusal of an internal address
however deep hyper put it (`portitor-runtime/src/http/connect.rs`).

## Attributes and `derive`

- `#[derive(Debug, Clone, PartialEq)]` asks the compiler to write the
  usual methods: printing for logs, copying, comparing.
- `#[serde(rename_all = "camelCase")]` and `#[serde(rename = "@type")]`
  map Rust field names to JSON names. `skip_serializing_if =
  "Option::is_none"` leaves out empty fields.
- `#[serde(deny_unknown_fields)]`: refuse JSON with a field the type
  does not have, instead of ignoring it (the management APIs' request
  types, so a typo is an error; the ODRL policy types `Offer`,
  `Agreement`, `Rule` and `AtomicConstraint`, so a term that is not
  understood is never dropped, ADR 0009).
- `#[serde(default)]`: a missing field gets its empty default (`[]`,
  `""`, `false`); `default = "pull_format"` calls that function instead
  (`portitor-connector/src/management.rs`).
- `#[serde(flatten)]`: the fields of this value sit at the same level
  as the others in the JSON, not inside a field of their own (`Dataset`
  keeps every other metadata property this way, `portitor-dsp/src/lib.rs`).
- `#[serde(untagged)]` on an enum: try each variant in turn and take the
  first that fits the JSON. `Relationship` in `did.rs` is a key id string
  or a whole key object.
  When no variant fits, the error says only "data did not match any
  variant", so the reason is lost. Where the reason matters, the type is
  read by hand instead: `Constraint` in `portitor-dsp/src/lib.rs` has
  its own `impl Deserialize`, which reads the JSON into a
  `serde_json::Value`, looks at the fields to pick the kind, and then
  reads that kind (`read_constraint`). A policy field that is not
  understood is then refused by name. `Offer::deserialize(&value)` reads
  a type from a `&Value` without copying it.
- `#[serde(tag = "kty")]`: the JSON field `kty` says which variant it is
  (`Jwk`: `"EC"` or `"RSA"`).
- `#[serde(deserialize_with = "one_audience")]`: read this field with
  the named function instead of the default (`Claims::aud` in
  `id_token.rs` accepts a string or a list of one).
- `Messages {}` instead of `Messages` in an enum read by serde:
  `deny_unknown_fields` refuses extra fields only in variants written
  with braces, so `{"type": "messages", "extra": 1}` is an error
  (`Role` in `portitor-dcp/src/bundle.rs`). The empty braces are on
  purpose.
- `serde_json::value::RawValue`: JSON kept as the exact text it was read
  from, and written out unchanged. `sign_manifest` in
  `portitor-dcp/src/bundle.rs` signs the manifest's bytes this way, so
  the signature covers exactly the file that is published, not a copy
  that serde wrote again (with other spaces or field order).
- `serde::de::IgnoredAny`: takes any JSON value and keeps nothing of it.
  A field of that type is accepted and not read (`_context` in
  `CatalogRequest`, `portitor-connector/src/management/catalog/request.rs`: the `_` in
  front says the field is never used). A type with only the fields
  that matter skips all the others the same way, without building them
  in memory: `CatalogHead` in
  `portitor-connector/src/catalog_reader/answer.rs` reads the `@type`
  and the `participantId` of a catalog of many megabytes, and
  `DatasetHead` there the `@type` of a dataset.
- `#[must_use]`: the caller should use the return value.
  `#[must_use = "..."]` adds the reason to the compiler's warning:
  `Telemetry` says to call `shutdown()`, and `CallerTrace::join` that the
  span is left when its guard is dropped (`telemetry.rs`,
  `telemetry/http.rs`).
- `#[cfg(test)]`: compiled only for tests. `#[cfg(feature = "tck")]`:
  compiled only in the DSP TCK build.
- `#[allow(clippy::...)]`: switches off one lint here. A comment says
  why.

### Reading a type by hand: `impl<'de> Deserialize<'de>`

```rust
impl<'de> Deserialize<'de> for Constraint {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        read_constraint(value).map_err(D::Error::custom)
    }
}
```

This is what `#[derive(Deserialize)]` writes for other types, written
by hand (`Constraint` in `portitor-dsp/src/lib.rs`):

- `D: Deserializer<'de>` is the reader of some format; for Portitor it
  is always JSON, but serde's trait does not say so.
- `'de` is the lifetime of the text being read. serde needs it for
  types that borrow from that text; `Constraint` borrows nothing, so
  the `'de` is only there because the trait asks for it.
- `D::Error` is the error type of that reader. `D::Error::custom(text)`
  makes one from a message (it comes from the trait
  `serde::de::Error`, imported as `Error as _`), so
  `.map_err(D::Error::custom)` turns the `String` of `read_constraint`
  into the error serde expects. The message then shows up in the
  `serde_json` error with the line and column.

The body first reads the JSON whole into a `serde_json::Value`, then
decides from its fields what it is.

### Reading JSON in two steps: strict for known names, open for others

```rust
impl<'de> Deserialize<'de> for Role {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Box::<RawValue>::deserialize(deserializer)?;       // keep the JSON text
        let RoleTypeOnly { type_name } = serde_json::from_str(text.get())?;  // step 1
        if KNOWN_ROLE_TYPES.contains(&type_name.as_str()) {
            let known: KnownRole = serde_json::from_str(text.get())?;        // step 2
            return Ok(known.into());
        }
        Ok(Role::Unknown { type_name })
    }
}
```

(Shortened from `Role` in `portitor-dcp/src/bundle.rs`; the real code
also turns the errors into `D::Error` and checks the name.)

The rule is "read the roles I know strictly, and keep only the name of
any other". serde's derive cannot say that: `deny_unknown_fields`
refuses every unknown name, and `#[serde(other)]` cannot keep the name
or allow extra fields. So the rule is written out:

- `RawValue` (from `serde_json`) is a piece of JSON kept as text,
  not yet read into a type. `text.get()` gives that text.
- Step 1 reads the same text into a small struct that has only the
  field `type`; serde passes over the other fields.
- Step 2, only for a known name, reads the text again into
  `KnownRole`, an enum with `deny_unknown_fields`, so a field a known
  role does not have is still an error.
- Any other name becomes the variant `Role::Unknown { type_name }`. A
  `match` on `Role` must then say what happens to an unknown role, and
  the compiler checks that it does.

Reading the text twice costs little here: a role is a few dozen bytes.

### `#[serde(try_from = "...")]`: read as written, then check

```rust
#[derive(Deserialize)]
#[serde(try_from = "FileEntryAsWritten")]
pub struct FileEntry { /* ... media_type: MediaType, role: Role */ }

impl TryFrom<FileEntryAsWritten> for FileEntry {
    type Error = String;
    fn try_from(written: FileEntryAsWritten) -> Result<Self, String> { /* ... */ }
}
```

A second, shorter way to read in two steps. serde first reads the JSON
into `FileEntryAsWritten` (the same fields, the media type still a
text), then calls `try_from` to make the real type; its error becomes
the read error. Used where one field's rule depends on another: which
media types a bundle file may have depends on its role
(`crates/portitor-dcp/src/bundle.rs`).

## Macros: `name!(...)`

A name with `!` is a macro: code that writes code at compile time.
`format!`, `vec!` and `tracing::info!` come from libraries. Portitor
defines four: `type_tag!`, `pid_message!` and `process_ack!` in
`portitor-dsp/src/lib.rs`, and `type_name!` in
`portitor-dcp/src/message.rs`. They declare the many DSP and DCP message
types without copying the same fields into each one. Each has a comment
with an example of what it writes. (A few test files define small macros
of their own, for example `schema!`.)

`include_str!("../contexts/dspace.jsonld")` puts the text of a file into
the program when it is built, as a `&'static str`: the DSP context ships
inside the binary, and nothing is read at run time
(`portitor-connector/src/shacl.rs`). The path is relative to the source
file.

Some macros are written as attributes, `#[name]` or `#[name(...)]`, and
rewrite the item below them:

- `#[tokio::main]` on `async fn main` starts the async runtime and runs
  `main` in it (each `main.rs`).
- `#[utoipa::path(post, path = "/v1/credential-requests", ...)]` on a
  handler describes it for the OpenAPI document; it does not change
  what the handler does (`management.rs`).
- `#[derive(ToSchema)]` writes the OpenAPI schema of a request or answer
  type, as `#[derive(Serialize)]` writes its JSON.

## Web handlers (axum)

```rust
pub(crate) async fn token(
    State(identity): State<Arc<Identity>>,
    form: Result<Form<TokenRequest>, FormRejection>,
) -> Response {
```

axum fills each argument of a handler from the request (an
*extractor*): `State(identity)` from the router's shared state, `Form`
from the form body. Normally a body that cannot be read is refused by
axum itself, with its own answer. Taking `Result<Form<...>, ...>`
instead lets the handler see the failure and answer in the format the
API needs (here the STS error list, `portitor-identity/src/sts.rs`).

A *layer* wraps routes with code that runs before the handler (a
middleware, written with `axum::middleware::from_fn_with_state`).
`router.layer(...)` wraps every route of a router;
`post(handler).layer(...)` wraps one route. Both wrap, so both run:
`small_message(post(negotiation::event))` in the connector is behind the
port's size limit and its own (`portitor-runtime/src/request_size.rs`).

## Small things

- `let _ = x;` ignores a result on purpose. A comment says why that is
  safe.
- `drop(guard)` ends a lock (or any value) here instead of at the end of
  the block, so others waiting for it go on at once
  (`BundleCopy::replace` in `bundle_harvest.rs` frees the write lock
  before it records the metrics).
- `impl Trait` in an argument (`var: impl Fn(&str) -> Option<OsString>`)
  means "any type that implements `Trait`".
- `impl Trait` as a return type means "some type that implements
  `Trait`; the caller needs to know no more". `-> impl Iterator<Item =
  (&str, &str)>` (`iter` in `portitor-dsp/src/policy/credentials.rs`)
  gives something to loop over without naming its real type.
- `pub(crate)` is visible inside this crate only.
- `Self` is the type the `impl` is for.
- `x.wrapping_add(1)` adds one and wraps around at the maximum instead of
  stopping the program. Used for version counters.
- `cond.then_some(value)`: `Some(value)` if `cond` is true, else
  `None` (`status_url` in `portitor-identity/src/issuance.rs`).
- `a.saturating_sub(b)` stops at 0 instead of going below;
  `1_u64.checked_shl(n)` is `1 << n`, or `None` when `n` is too large
  (`retry_delay` in `portitor-runtime/src/retry.rs`).
- `[Profile; 2]` is a list of exactly two `Profile`s, fixed when the
  program is built (`PROFILES` in `status_lists.rs`).
- `env!("CARGO_PKG_VERSION")` is the crate's version from `Cargo.toml`,
  read when the program is built (the `OpenAPI` documents).
- `!` on an integer flips every bit, it is not "not": `!0u32` is
  `0xFFFF_FFFF` (CRC-32 in `gzip.rs`). On a `bool`, `!` is "not".
- `static BOOLEAN: LazyLock<Option<Regex>> = LazyLock::new(|| ...)`: a
  value built the first time it is used, then shared by every thread for
  the rest of the program (the XSD patterns in
  `portitor-shacl/src/lexical.rs`). A regular expression is compiled
  once, not on every check. The metric instruments are made the same way
  (`INSTRUMENTS` in `portitor-runtime/src/telemetry/metrics.rs`).
- `list.sort_by_cached_key(f)` sorts by `f(item)` but calls `f` only
  once per item; `sort_by_key` may call it many times. Worth it when the
  key is costly to make, such as the text of an RDF term
  (`portitor-shacl/src/validate.rs`, which sorts so that reports come
  out in the same order every time).
- `Option` values can be compared, and `None` counts as smaller than any
  `Some`, so `a.max(b)` of two `Option`s picks a `Some` over a `None`.
  That is easy to misread, so Portitor spells it out with a `match`
  (`later` and `earlier` in `credential.rs`).
- `map.entry(key).or_default()` gives the value under `key`, first
  inserting an empty default when there is none; `or_insert_with(|| ...)`
  computes the value to insert (`portitor-shacl/src/graph.rs`).
- `usize::from(flag)` is 1 for `true` and 0 for `false`. Adding three of
  them counts how many of three conditions hold
  (`portitor-shacl/src/load.rs`).
- `b"<?xml"` is a *byte string*: bytes, not text, compared with the
  bytes of a file. `bytes.strip_prefix(b"\xEF\xBB\xBF")` drops a UTF-8
  byte order mark if there is one (`portitor-issuer/src/bundle_build.rs`).
- `x as u64` converts a number. Portitor uses `as` only where nothing can
  be lost, such as a `usize` length into a `u64` (`max_bytes as u64` in
  `portitor-bundle/src/source.rs`). Where a value may not fit, it writes
  `u64::try_from(x)`, which gives an error instead of a wrong number,
  and says what to do then (`.unwrap_or(u64::MAX)`, or an error).
- `const fn` is a function that may also run while the program is
  built, so its result can fill a `const` (`sh()` in
  `portitor-shacl/src/vocab.rs`; `as_str` in
  `portitor-dcp/src/bundle.rs`). Called at run time, it is an ordinary
  function.
