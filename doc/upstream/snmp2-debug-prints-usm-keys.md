# snmp2: `Debug` on `Security` prints the USM password and the localized keys

**Project:** [snmp2](https://github.com/roboplc/snmp2) · **Found in:** 0.5.2 ·
**Severity:** disclosure of key material through ordinary logging · **Status:** draft, not filed

## What happens

`Security` and its `AuthoritativeState` both derive `Debug`, and those fields hold secrets:

```rust
#[derive(Debug, Clone)]
pub struct Security {
    pub(crate) authentication_password: Vec<u8>,   // the USM password, verbatim
    pub(crate) authoritative_state: AuthoritativeState,
    pub(crate) plain_buf: Vec<u8>,                 // the decrypted scoped PDU
    ...
}

#[derive(Debug, Clone)]
pub(crate) struct AuthoritativeState {
    auth_key: Vec<u8>,   // localized authentication key
    priv_key: Vec<u8>,   // localized privacy key
    ...
}
```

So any `{:?}` of a value carrying security state writes the password and both localized keys to
wherever that output goes. It is easy to reach without meaning to:

- `println!("Parsed PDU: {:?}", pdu)` — the crate's own tests do this (`src/tests.rs`), which is
  how a static analyser first flagged it for us (CodeQL, *Cleartext logging of sensitive
  information*);
- an application logging a parsed PDU, a session, or an error value at debug level;
- `assert_eq!` / `unwrap()` panic messages in downstream tests, which land in CI logs.

`plain_buf` additionally exposes the decrypted payload of the last authPriv message.

## Suggested fix

Replace both derives with manual implementations that print the public identity and the clock but
never the secrets — length instead of value:

```rust
impl fmt::Debug for AuthoritativeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthoritativeState")
            .field("auth_key", &Redacted(self.auth_key.len()))
            .field("priv_key", &Redacted(self.priv_key.len()))
            .field("engine_id", &self.engine_id)
            .field("engine_boots", &self.engine_boots)
            .field("engine_time", &self.engine_time)
            .finish()
    }
}
```

and the same for `Security` (`authentication_password` and `plain_buf` redacted, `username` and
`context_name` printed as text). Engine ID, boots and time stay visible: they are public protocol
state and the useful part when debugging a time-window rejection.

## How tedge-dot is affected

The C and Rust connectors both handle USM passwords, and tedge-dot's own configuration type keeps
secrets out of `Debug` for exactly this reason. The vendored copy carries this fix as patch 7
(`impl/rust/vendor/snmp2/TEDGE-DOT-PATCH.md`); it can be dropped once a release includes it.
