//! One request session per device (spec §5): GET, GETBULK and SET over UDP, bounded by
//! `request_timeout` and re-sent `retries` times.
//!
//! `snmp2`'s `AsyncSession` has no timeout of its own — it waits for a datagram forever — so
//! every call is wrapped in [`tokio::time::timeout`]. A cancelled call leaves the library's
//! request-id untouched, so the re-send carries the same id and a late answer to the earlier
//! attempt still matches; answers to requests already given up on are skipped by the patched
//! library (`vendor/snmp2`, patch 4).

use crate::config::{Device, SnmpVersion};
use crate::value::{hex, DecodeError, Oid, VarValue};
use snmp2::v3::AuthErrorKind;
use snmp2::{AsyncSession, Error as SnmpError, Pdu, Value as SnmpValue};
use std::fmt;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::time::timeout;

/// What a response carries, owned (the library's PDU borrows the session's buffer).
#[derive(Debug)]
pub struct Reply {
    pub error_status: u32,
    pub error_index: u32,
    pub varbinds: Vec<(Oid, VarValue)>,
}

/// Why a request produced no response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    /// No answer within `request_timeout`, after every re-send.
    Timeout { after: Duration, attempts: u32 },
    /// USM rejected the exchange (wrong key, unknown user, an engine ID that changed).
    Auth(String),
    /// The socket failed, or the agent's host refused the datagram.
    Transport(String),
    /// The agent answered something that does not decode.
    Protocol(String),
}

impl RequestError {
    /// Whether the rest of this poll cycle is pointless: the device is unreachable or refuses
    /// us, and waiting for another timeout per batch only stretches the cycle.
    pub fn stops_the_cycle(&self) -> bool {
        !matches!(self, RequestError::Protocol(_))
    }
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RequestError::Timeout { after, attempts } => write!(
                f,
                "no answer within {:.3}s ({attempts} attempt(s), connection.request_timeout)",
                after.as_secs_f64()
            ),
            RequestError::Auth(e) => write!(f, "SNMPv3 authentication failed: {e}"),
            RequestError::Transport(e) => write!(f, "transport error: {e}"),
            RequestError::Protocol(e) => write!(f, "undecodable response: {e}"),
        }
    }
}

/// One attempt's outcome.
enum Attempt {
    Done(Reply),
    Failed(SnmpError),
    Undecodable(DecodeError),
    TimedOut,
}

enum Verdict {
    Done(Reply),
    Again,
    Fail(RequestError),
}

/// How many re-sends are left, and whether the v3 clock has been re-synchronised once.
struct Retry {
    attempts: u32,
    resynced: bool,
}

pub struct Session {
    session: AsyncSession,
    version: SnmpVersion,
    timeout: Duration,
    retries: u32,
    /// The engine ID the device must have, when one is configured.
    pinned_engine_id: Option<Vec<u8>>,
    /// The engine ID discovery found (spec §3.2: a v3 trap is accepted from it once known).
    discovered_engine_id: Option<Vec<u8>>,
    /// v3 only: whether discovery has run on this session.
    ready: bool,
}

impl Session {
    /// Open a session to `address`. `community` selects the read or the write community; a v3
    /// session uses the device's USM credentials and discovers the agent's engine on first use.
    pub async fn open(device: &Device, address: SocketAddr, community: &[u8]) -> Result<Session, String> {
        let req_id = starting_req_id();
        let session = match device.version {
            SnmpVersion::V1 => AsyncSession::new_v1(address, community, req_id).await,
            SnmpVersion::V2c => AsyncSession::new_v2c(address, community, req_id).await,
            SnmpVersion::V3 => {
                let v3 = device
                    .v3
                    .as_ref()
                    .ok_or_else(|| "version \"v3\" without credentials".to_string())?;
                AsyncSession::new_v3(address, req_id, v3.security()).await
            }
        }
        .map_err(|e| format!("cannot reach {address}: {e}"))?;
        Ok(Session {
            session,
            version: device.version,
            timeout: device.request_timeout,
            retries: device.retries,
            pinned_engine_id: device.v3.as_ref().and_then(|v3| v3.engine_id.clone()),
            discovered_engine_id: None,
            ready: device.version != SnmpVersion::V3,
        })
    }

    /// The agent's engine ID, once a v3 exchange has discovered (or confirmed) it.
    pub fn engine_id(&self) -> Option<&[u8]> {
        self.discovered_engine_id.as_deref()
    }

    pub async fn get(&mut self, oids: &[Oid]) -> Result<Reply, RequestError> {
        let owned: Vec<snmp2::Oid<'static>> = oids.iter().map(Oid::to_library).collect();
        let refs: Vec<&snmp2::Oid<'static>> = owned.iter().collect();
        let mut retry = Retry { attempts: 0, resynced: false };
        loop {
            self.discover().await?;
            let attempt = match timeout(self.timeout, self.session.get_many(&refs)).await {
                Ok(Ok(pdu)) => reply(&pdu),
                Ok(Err(e)) => Attempt::Failed(e),
                Err(_) => Attempt::TimedOut,
            };
            match self.judge(attempt, &mut retry) {
                Verdict::Done(reply) => return Ok(reply),
                Verdict::Fail(e) => return Err(e),
                Verdict::Again => continue,
            }
        }
    }

    /// GETBULK with non-repeaters = the whole batch and max-repetitions 0 (spec §5.1): each
    /// requested OID is answered with its lexicographic successor, so the caller asks for the
    /// PARENT of a scalar instance (`x` for `x.0`) and checks the name that comes back.
    pub async fn get_bulk(&mut self, oids: &[Oid]) -> Result<Reply, RequestError> {
        let owned: Vec<snmp2::Oid<'static>> = oids.iter().map(Oid::to_library).collect();
        let refs: Vec<&snmp2::Oid<'static>> = owned.iter().collect();
        let non_repeaters = u32::try_from(refs.len()).unwrap_or(u32::MAX);
        let mut retry = Retry { attempts: 0, resynced: false };
        loop {
            self.discover().await?;
            let attempt = match timeout(
                self.timeout,
                self.session.getbulk(&refs, non_repeaters, 0),
            )
            .await
            {
                Ok(Ok(pdu)) => reply(&pdu),
                Ok(Err(e)) => Attempt::Failed(e),
                Err(_) => Attempt::TimedOut,
            };
            match self.judge(attempt, &mut retry) {
                Verdict::Done(reply) => return Ok(reply),
                Verdict::Fail(e) => return Err(e),
                Verdict::Again => continue,
            }
        }
    }

    pub async fn set(&mut self, values: &[(&snmp2::Oid<'_>, SnmpValue<'_>)]) -> Result<Reply, RequestError> {
        let mut retry = Retry { attempts: 0, resynced: false };
        loop {
            self.discover().await?;
            let attempt = match timeout(self.timeout, self.session.set(values)).await {
                Ok(Ok(pdu)) => reply(&pdu),
                Ok(Err(e)) => Attempt::Failed(e),
                Err(_) => Attempt::TimedOut,
            };
            match self.judge(attempt, &mut retry) {
                Verdict::Done(reply) => return Ok(reply),
                Verdict::Fail(e) => return Err(e),
                Verdict::Again => continue,
            }
        }
    }

    /// SNMPv3 discovery (RFC 3414 §4): learn the agent's engine ID, boots and time before the
    /// first authenticated request, and hold it to the configured engine ID when there is one.
    async fn discover(&mut self) -> Result<(), RequestError> {
        if self.ready {
            return Ok(());
        }
        let mut attempts = 0;
        loop {
            match timeout(self.timeout, self.session.init()).await {
                Ok(Ok(())) => break,
                Ok(Err(e)) => return Err(classify(e)),
                Err(_) => {
                    attempts += 1;
                    if attempts > self.retries {
                        return Err(RequestError::Timeout { after: self.timeout, attempts });
                    }
                }
            }
        }
        let discovered = self
            .session
            .security()
            .map(|security| security.engine_id().to_vec())
            .unwrap_or_default();
        if let Some(pinned) = &self.pinned_engine_id {
            if pinned != &discovered {
                return Err(RequestError::Auth(format!(
                    "the agent's engine ID {} is not the configured {}",
                    hex(&discovered),
                    hex(pinned)
                )));
            }
        }
        self.discovered_engine_id = Some(discovered);
        self.ready = true;
        Ok(())
    }

    fn judge(&mut self, attempt: Attempt, retry: &mut Retry) -> Verdict {
        match attempt {
            Attempt::Done(reply) => Verdict::Done(reply),
            Attempt::Undecodable(e) => Verdict::Fail(RequestError::Protocol(e.to_string())),
            Attempt::TimedOut => {
                retry.attempts += 1;
                if retry.attempts > self.retries {
                    Verdict::Fail(RequestError::Timeout {
                        after: self.timeout,
                        attempts: retry.attempts,
                    })
                } else {
                    Verdict::Again
                }
            }
            Attempt::Failed(e) => match &e {
                // The agent's clock moved on (or it rebooted): the library has taken the new
                // boots and time from its Report, so the same request succeeds now.
                SnmpError::AuthUpdated
                | SnmpError::AuthFailure(AuthErrorKind::EngineTimeMismatch)
                | SnmpError::AuthFailure(AuthErrorKind::EngineBootsMismatch)
                    if !retry.resynced =>
                {
                    retry.resynced = true;
                    Verdict::Again
                }
                SnmpError::AuthFailure(_) | SnmpError::Crypto(_) => {
                    // Discover again on the next request: an agent that restarted with another
                    // engine ID answers nothing else.
                    self.ready = self.version != SnmpVersion::V3;
                    Verdict::Fail(RequestError::Auth(e.to_string()))
                }
                SnmpError::Send | SnmpError::Receive => {
                    retry.attempts += 1;
                    if retry.attempts > self.retries {
                        Verdict::Fail(RequestError::Transport(e.to_string()))
                    } else {
                        Verdict::Again
                    }
                }
                _ => Verdict::Fail(RequestError::Protocol(e.to_string())),
            },
        }
    }
}

fn reply(pdu: &Pdu<'_>) -> Attempt {
    let mut varbinds = Vec::new();
    for (name, value) in pdu.varbinds.clone() {
        let name = match Oid::from_library(&name) {
            Ok(oid) => oid,
            Err(e) => return Attempt::Undecodable(e),
        };
        match VarValue::from_library(&value) {
            Ok(value) => varbinds.push((name, value)),
            Err(e) => return Attempt::Undecodable(e),
        }
    }
    Attempt::Done(Reply {
        error_status: pdu.error_status,
        error_index: pdu.error_index,
        varbinds,
    })
}

fn classify(e: SnmpError) -> RequestError {
    match e {
        SnmpError::AuthFailure(_) | SnmpError::AuthUpdated | SnmpError::Crypto(_) => {
            RequestError::Auth(e.to_string())
        }
        SnmpError::Send | SnmpError::Receive => RequestError::Transport(e.to_string()),
        _ => RequestError::Protocol(e.to_string()),
    }
}

/// The name of an SNMP error-status (RFC 3416 §3), for the `reason` of a failed write and the
/// `error` of a bad sample.
pub fn error_name(status: u32) -> &'static str {
    match status {
        0 => "noError",
        1 => "tooBig",
        2 => "noSuchName",
        3 => "badValue",
        4 => "readOnly",
        5 => "genErr",
        6 => "noAccess",
        7 => "wrongType",
        8 => "wrongLength",
        9 => "wrongEncoding",
        10 => "wrongValue",
        11 => "noCreation",
        12 => "inconsistentValue",
        13 => "resourceUnavailable",
        14 => "commitFailed",
        15 => "undoFailed",
        16 => "authorizationError",
        17 => "notWritable",
        18 => "inconsistentName",
        _ => "unknownError",
    }
}

/// Request-ids start somewhere unpredictable, so a restarted connector does not reuse the ids
/// an agent (or a spoofer) has just seen.
fn starting_req_id() -> i32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(1);
    (nanos & 0x3FFF_FFFF) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_that_stop_a_cycle_are_the_ones_about_the_device() {
        assert!(RequestError::Timeout { after: Duration::from_secs(1), attempts: 2 }.stops_the_cycle());
        assert!(RequestError::Auth("wrong digest".into()).stops_the_cycle());
        assert!(RequestError::Transport("refused".into()).stops_the_cycle());
        assert!(!RequestError::Protocol("bad varbind".into()).stops_the_cycle());
        assert!(RequestError::Timeout { after: Duration::from_millis(1500), attempts: 2 }
            .to_string()
            .contains("1.500s"));
    }

    #[test]
    fn snmp_error_names_are_the_rfc_ones() {
        assert_eq!(error_name(0), "noError");
        assert_eq!(error_name(2), "noSuchName");
        assert_eq!(error_name(17), "notWritable");
        assert_eq!(error_name(99), "unknownError");
    }
}
