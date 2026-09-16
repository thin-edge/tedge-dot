//! Receiver-side SNMPv3: the User-based Security Model of RFC 3414 §3.2 for the messages the
//! client sessions never see. TEDGE-DOT-PATCH(5)
//!
//! Upstream implements USM for a command generator only: the remote agent is always the
//! authoritative engine, and discovery is a client that learns its engine ID and clock. A
//! notification receiver needs the other roles too:
//!
//! - **authoritative** ([`LocalEngine`]): an InformRequest is sent to the receiver's engine ID,
//!   boots and time. The receiver owns them, answers a sender that has not discovered them with
//!   the Reports of RFC 3414 §3.2 (`usmStatsUnknownEngineIDs`, `usmStatsNotInTimeWindows`, ...)
//!   and acknowledges the inform with a Response under its own engine. The same code serves an
//!   agent answering requests.
//! - **non-authoritative** ([`receive_non_authoritative`]): a Trap (or a Response/Report) is
//!   sent under the *sender's* engine; the receiver checks it against the sender's engine ID
//!   and its latest boots/time (RFC 3414 §3.2 step 7b).
//!
//! [`encode`] builds any v3 message from explicit header values, for all of the above.

use std::time::Instant;

use super::{Auth, AuthErrorKind, Security};
use crate::{
    AsnReader, BUFFER_SIZE, Error, MessageType, Oid, Pdu, Result, Value, Varbinds, Version, asn1,
    pdu::{self, Buf},
    snmp::{V3_MSG_FLAGS_AUTH, V3_MSG_FLAGS_PRIVACY, V3_MSG_FLAGS_REPORTABLE},
};

/// RFC 3414 §3.2 step 7: the tolerated difference between engine times, in seconds.
const TIME_WINDOW: i64 = 150;
/// snmpEngineBoots latches at this value (RFC 3414 §2.2.2).
const MAX_ENGINE_BOOTS: i64 = i32::MAX as i64;
/// RFC 3411 SnmpEngineID: 5 to 32 octets (a discovery probe carries none).
const ENGINE_ID_LEN: std::ops::RangeInclusive<usize> = 5..=32;
/// RFC 3412 msgMaxSize lower bound.
const MIN_MAX_SIZE: i64 = 484;

/// The `usmStats` counters (RFC 3414 §5), whose instances the Reports carry.
pub mod usm_stats {
    pub const UNSUPPORTED_SEC_LEVELS: &[u64] = &[1, 3, 6, 1, 6, 3, 15, 1, 1, 1, 0];
    pub const NOT_IN_TIME_WINDOWS: &[u64] = &[1, 3, 6, 1, 6, 3, 15, 1, 1, 2, 0];
    pub const UNKNOWN_USER_NAMES: &[u64] = &[1, 3, 6, 1, 6, 3, 15, 1, 1, 3, 0];
    pub const UNKNOWN_ENGINE_IDS: &[u64] = &[1, 3, 6, 1, 6, 3, 15, 1, 1, 4, 0];
    pub const WRONG_DIGESTS: &[u64] = &[1, 3, 6, 1, 6, 3, 15, 1, 1, 5, 0];
    pub const DECRYPTION_ERRORS: &[u64] = &[1, 3, 6, 1, 6, 3, 15, 1, 1, 6, 0];
}

/// A message's security level (msgFlags).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SecurityLevel {
    NoAuthNoPriv,
    AuthNoPriv,
    AuthPriv,
}

impl SecurityLevel {
    fn from_flags(flags: u8) -> SecurityLevel {
        match (flags & V3_MSG_FLAGS_AUTH != 0, flags & V3_MSG_FLAGS_PRIVACY != 0) {
            (true, true) => SecurityLevel::AuthPriv,
            (true, false) => SecurityLevel::AuthNoPriv,
            _ => SecurityLevel::NoAuthNoPriv,
        }
    }

    fn flags(self) -> u8 {
        match self {
            SecurityLevel::NoAuthNoPriv => 0,
            SecurityLevel::AuthNoPriv => V3_MSG_FLAGS_AUTH,
            SecurityLevel::AuthPriv => V3_MSG_FLAGS_AUTH | V3_MSG_FLAGS_PRIVACY,
        }
    }
}

/// The unauthenticated outer layers of an SNMPv3 message: msgGlobalData and the USM
/// security parameters. Enough to route a message to the user and engine that can process it.
#[derive(Debug, Clone)]
pub struct Header<'a> {
    pub msg_id: i32,
    pub max_size: i64,
    pub flags: u8,
    pub engine_id: &'a [u8],
    pub engine_boots: i64,
    pub engine_time: i64,
    pub user_name: &'a [u8],
    auth_params: &'a [u8],
    /// Offset of `auth_params` within `message`.
    auth_offset: usize,
    priv_params: &'a [u8],
    /// The message SEQUENCE, without anything after it in the datagram.
    message: &'a [u8],
    /// The plaintext scopedPDU SEQUENCE's content, or the encryptedPDU octets.
    data: &'a [u8],
}

impl<'a> Header<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Header<'a>> {
        let content = AsnReader::from_bytes(bytes).read_raw(asn1::TYPE_SEQUENCE)?;
        let message = &bytes[..offset_in(bytes, content) + content.len()];
        let mut rdr = AsnReader::from_bytes(content);
        if rdr.read_asn_integer()? != Version::V3 as i64 {
            return Err(Error::UnsupportedVersion);
        }

        let mut global = AsnReader::from_bytes(rdr.read_raw(asn1::TYPE_SEQUENCE)?);
        let msg_id = non_negative_i32(global.read_asn_integer()?)?;
        let max_size = global.read_asn_integer()?;
        if !(MIN_MAX_SIZE..=i64::from(i32::MAX)).contains(&max_size) {
            return Err(Error::ValueOutOfRange);
        }
        let flags = match global.read_asn_octetstring()? {
            [flags] => *flags,
            _ => return Err(Error::AsnInvalidLen),
        };
        if flags & V3_MSG_FLAGS_PRIVACY != 0 && flags & V3_MSG_FLAGS_AUTH == 0 {
            return Err(Error::AuthFailure(AuthErrorKind::UnsupportedSecLevel));
        }
        if global.read_asn_integer()? != 3 {
            return Err(Error::AuthFailure(AuthErrorKind::UnsupportedUSM));
        }

        let usm = rdr.read_asn_octetstring()?;
        let mut usm = AsnReader::from_bytes(AsnReader::from_bytes(usm).read_raw(asn1::TYPE_SEQUENCE)?);
        let engine_id = usm.read_asn_octetstring()?;
        if engine_id.len() > *ENGINE_ID_LEN.end() {
            return Err(Error::AsnInvalidLen);
        }
        let engine_boots = i64::from(non_negative_i32(usm.read_asn_integer()?)?);
        let engine_time = i64::from(non_negative_i32(usm.read_asn_integer()?)?);
        let user_name = usm.read_asn_octetstring()?;
        if user_name.len() > 32 {
            return Err(Error::AsnInvalidLen);
        }
        let auth_params = usm.read_asn_octetstring()?;
        let priv_params = usm.read_asn_octetstring()?;

        let data = if flags & V3_MSG_FLAGS_PRIVACY != 0 {
            rdr.read_asn_octetstring()?
        } else {
            rdr.read_raw(asn1::TYPE_SEQUENCE)?
        };

        Ok(Header {
            msg_id,
            max_size,
            flags,
            engine_id,
            engine_boots,
            engine_time,
            user_name,
            auth_params,
            auth_offset: offset_in(message, auth_params),
            priv_params,
            message,
            data,
        })
    }

    pub fn security_level(&self) -> SecurityLevel {
        SecurityLevel::from_flags(self.flags)
    }

    /// The sender expects a Report when the message cannot be processed: set on requests and
    /// informs (confirmed class), never on traps, responses and reports.
    pub fn is_reportable(&self) -> bool {
        self.flags & V3_MSG_FLAGS_REPORTABLE != 0
    }
}

/// A message that passed the security checks, with its scoped PDU.
#[derive(Debug, Clone)]
pub struct Message<'a> {
    pub header: Header<'a>,
    pub context_engine_id: &'a [u8],
    pub context_name: &'a [u8],
    /// `community` holds the user name, `v3_msg_id` the msgID.
    pub pdu: Pdu<'a>,
    /// The user the message was processed with (a Response is sent with it).
    pub security: &'a Security,
}

/// Why a message was not processed, with the Report to send back to its source when the
/// sender asked for one (RFC 3412 §7.2 step 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub error: Error,
    pub report: Option<Vec<u8>>,
}

impl Refusal {
    fn silent(error: Error) -> Refusal {
        Refusal { error, report: None }
    }
}

/// The `usmStats` counters of a [`LocalEngine`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsmStats {
    pub unsupported_sec_levels: u32,
    pub not_in_time_windows: u32,
    pub unknown_user_names: u32,
    pub unknown_engine_ids: u32,
    pub wrong_digests: u32,
    pub decryption_errors: u32,
}

/// This SNMP engine in the authoritative role: the engine ID, boots and time that informs
/// (and requests) are addressed to.
#[derive(Debug, Clone)]
pub struct LocalEngine {
    engine_id: Vec<u8>,
    engine_boots: i64,
    time_at_start: i64,
    started: Instant,
    stats: UsmStats,
}

impl LocalEngine {
    /// `engine_boots` should grow every time the engine restarts with the same ID (senders
    /// reject a smaller value than the one they last saw from it until they re-discover).
    pub fn new(engine_id: &[u8], engine_boots: i64) -> Result<LocalEngine> {
        if !ENGINE_ID_LEN.contains(&engine_id.len()) {
            return Err(Error::AsnInvalidLen);
        }
        if !(0..MAX_ENGINE_BOOTS).contains(&engine_boots) {
            return Err(Error::ValueOutOfRange);
        }
        Ok(LocalEngine {
            engine_id: engine_id.to_vec(),
            engine_boots,
            time_at_start: 0,
            started: Instant::now(),
            stats: UsmStats::default(),
        })
    }

    /// Start the engine clock at `engine_time` seconds instead of 0.
    #[must_use]
    pub fn with_engine_time(mut self, engine_time: i64) -> LocalEngine {
        self.time_at_start = engine_time.clamp(0, MAX_ENGINE_BOOTS);
        self.started = Instant::now();
        self
    }

    pub fn engine_id(&self) -> &[u8] {
        &self.engine_id
    }

    pub fn engine_boots(&self) -> i64 {
        self.engine_boots
    }

    /// snmpEngineTime: seconds since the engine (re)started.
    pub fn engine_time(&self) -> i64 {
        let elapsed = i64::try_from(self.started.elapsed().as_secs()).unwrap_or(i64::MAX);
        self.time_at_start
            .saturating_add(elapsed)
            .min(MAX_ENGINE_BOOTS)
    }

    pub fn stats(&self) -> UsmStats {
        self.stats
    }

    /// A copy of `user` with its keys localized to this engine: what messages addressed to
    /// this engine are authenticated and encrypted with.
    pub fn localize(&self, user: &Security) -> Result<Security> {
        let mut user = user.clone();
        user.reset_engine_id();
        let mut user = user.with_engine_id(&self.engine_id)?;
        user.authoritative_state.engine_boots = self.engine_boots;
        user.authoritative_state.engine_time = self.engine_time();
        Ok(user)
    }

    /// Process a message addressed to this engine (RFC 3414 §3.2 as the authoritative engine).
    ///
    /// `user` is the security state of the user the message names, localized with
    /// [`LocalEngine::localize`], or `None` when the user is unknown. A refused message comes
    /// with the Report to send back to its source when the sender asked for one: a sender that
    /// has not discovered this engine yet gets `usmStatsUnknownEngineIDs` carrying the engine
    /// ID, then (authenticated) `usmStatsNotInTimeWindows` carrying boots and time.
    pub fn receive<'a>(
        &mut self,
        bytes: &'a [u8],
        user: Option<&'a mut Security>,
    ) -> std::result::Result<Message<'a>, Refusal> {
        let header = Header::parse(bytes).map_err(Refusal::silent)?;
        let plaintext_req_id = || {
            (header.flags & V3_MSG_FLAGS_PRIVACY == 0)
                .then(|| parse_scoped(header.data).ok().map(|s| s.req_id))
                .flatten()
                .unwrap_or(0)
        };

        if header.engine_id != self.engine_id.as_slice() {
            self.stats.unknown_engine_ids = self.stats.unknown_engine_ids.wrapping_add(1);
            let count = self.stats.unknown_engine_ids;
            return Err(self.refuse(
                &header,
                AuthErrorKind::EngineIdMismatch,
                usm_stats::UNKNOWN_ENGINE_IDS,
                count,
                plaintext_req_id(),
                None,
            ));
        }
        let user = match user {
            Some(user) if user.username.as_slice() == header.user_name => user,
            _ => {
                self.stats.unknown_user_names = self.stats.unknown_user_names.wrapping_add(1);
                let count = self.stats.unknown_user_names;
                return Err(self.refuse(
                    &header,
                    AuthErrorKind::UsernameMismatch,
                    usm_stats::UNKNOWN_USER_NAMES,
                    count,
                    plaintext_req_id(),
                    None,
                ));
            }
        };
        if user.security_level() > SecurityLevel::NoAuthNoPriv
            && user.engine_id() != self.engine_id.as_slice()
        {
            return Err(Refusal::silent(Error::AuthFailure(
                AuthErrorKind::SecurityNotReady,
            )));
        }
        let level = header.security_level();
        if level > user.security_level() {
            self.stats.unsupported_sec_levels = self.stats.unsupported_sec_levels.wrapping_add(1);
            let count = self.stats.unsupported_sec_levels;
            return Err(self.refuse(
                &header,
                AuthErrorKind::UnsupportedSecLevel,
                usm_stats::UNSUPPORTED_SEC_LEVELS,
                count,
                plaintext_req_id(),
                None,
            ));
        }
        if level >= SecurityLevel::AuthNoPriv {
            if verify(user, &header).is_err() {
                self.stats.wrong_digests = self.stats.wrong_digests.wrapping_add(1);
                let count = self.stats.wrong_digests;
                return Err(self.refuse(
                    &header,
                    AuthErrorKind::SignatureMismatch,
                    usm_stats::WRONG_DIGESTS,
                    count,
                    plaintext_req_id(),
                    None,
                ));
            }
            let now = self.engine_time();
            if header.engine_boots == MAX_ENGINE_BOOTS
                || header.engine_boots != self.engine_boots
                || (header.engine_time - now).abs() > TIME_WINDOW
            {
                self.stats.not_in_time_windows = self.stats.not_in_time_windows.wrapping_add(1);
                let count = self.stats.not_in_time_windows;
                return Err(self.refuse(
                    &header,
                    AuthErrorKind::EngineTimeMismatch,
                    usm_stats::NOT_IN_TIME_WINDOWS,
                    count,
                    plaintext_req_id(),
                    Some(&*user),
                ));
            }
        }
        if level == SecurityLevel::AuthPriv {
            let decrypted = user
                .decrypt_with(
                    header.data,
                    header.priv_params,
                    header.engine_boots,
                    header.engine_time,
                )
                .and_then(|()| scoped_content(&user.plain_buf).map(|_| ()));
            if let Err(error) = decrypted {
                self.stats.decryption_errors = self.stats.decryption_errors.wrapping_add(1);
                let count = self.stats.decryption_errors;
                let mut refusal = self.refuse(
                    &header,
                    AuthErrorKind::NotAuthenticated,
                    usm_stats::DECRYPTION_ERRORS,
                    count,
                    0,
                    None,
                );
                refusal.error = error;
                return Err(refusal);
            }
        }
        let user: &'a Security = user;
        let scoped = if level == SecurityLevel::AuthPriv {
            scoped_content(&user.plain_buf).and_then(parse_scoped)
        } else {
            parse_scoped(header.data)
        }
        .map_err(Refusal::silent)?;
        Ok(message(header, scoped, user))
    }

    /// Answer `request` (a Response to an InformRequest, or an agent's answer to a request) as
    /// this engine, at the request's security level and in its context.
    pub fn respond(
        &self,
        request: &Message<'_>,
        message_type: MessageType,
        error_status: u32,
        error_index: u32,
        varbinds: &[(&Oid, Value)],
    ) -> Result<Vec<u8>> {
        encode(
            request.security,
            &Outgoing {
                msg_id: request.header.msg_id,
                level: request.header.security_level(),
                reportable: false,
                engine_id: &self.engine_id,
                engine_boots: self.engine_boots,
                engine_time: self.engine_time(),
                context_engine_id: request.context_engine_id,
                context_name: request.context_name,
            },
            message_type,
            request.pdu.req_id,
            error_status,
            error_index,
            varbinds,
        )
    }

    /// The Response acknowledging an InformRequest: its request-id and varbinds, no error.
    pub fn acknowledge(&self, inform: &Message<'_>) -> Result<Vec<u8>> {
        let pairs: Vec<(Oid, Value)> = inform.pdu.varbinds.clone().collect();
        let varbinds: Vec<(&Oid, Value)> = pairs.iter().map(|(o, v)| (o, clone_value(v))).collect();
        self.respond(inform, MessageType::Response, 0, 0, &varbinds)
    }

    fn refuse(
        &self,
        header: &Header<'_>,
        kind: AuthErrorKind,
        counter: &[u64],
        count: u32,
        req_id: i32,
        signer: Option<&Security>,
    ) -> Refusal {
        Refusal {
            error: Error::AuthFailure(kind),
            report: header
                .is_reportable()
                .then(|| self.report(header, counter, count, req_id, signer).ok())
                .flatten(),
        }
    }

    /// A Report (RFC 3412 §7.1 step 3): unauthenticated, unless `signer` is given (the time
    /// window Report is authNoPriv so the sender can trust the boots and time it carries).
    fn report(
        &self,
        header: &Header<'_>,
        counter: &[u64],
        count: u32,
        req_id: i32,
        signer: Option<&Security>,
    ) -> Result<Vec<u8>> {
        let anonymous;
        let (user, level) = match signer {
            Some(user) => (user, SecurityLevel::AuthNoPriv),
            None => {
                anonymous = Security::new(header.user_name, &[]).with_auth(Auth::NoAuthNoPriv);
                (&anonymous, SecurityLevel::NoAuthNoPriv)
            }
        };
        let oid = Oid::from(counter).map_err(|_| Error::AsnParse)?;
        encode(
            user,
            &Outgoing {
                msg_id: header.msg_id,
                level,
                reportable: false,
                engine_id: &self.engine_id,
                engine_boots: self.engine_boots,
                engine_time: self.engine_time(),
                context_engine_id: &self.engine_id,
                context_name: &[],
            },
            MessageType::Report,
            req_id,
            0,
            0,
            &[(&oid, Value::Counter32(count))],
        )
    }
}

/// Process a message whose sender is the authoritative engine — a Trap, or a Response or
/// Report — as a non-authoritative receiver (RFC 3414 §3.2, step 7b).
///
/// `security` holds the user and the sender's engine: its ID (learned from this message when
/// empty, and kept only if the message passes) and the latest boots/time received from it. An
/// authenticated message with smaller boots, or the same boots and a time more than 150 s
/// before the latest, is a replay and is rejected. On any error `security` is left as it was.
pub fn receive_non_authoritative<'a>(
    bytes: &'a [u8],
    security: &'a mut Security,
) -> Result<Message<'a>> {
    let header = Header::parse(bytes)?;
    if header.user_name != security.username.as_slice() {
        return Err(Error::AuthFailure(AuthErrorKind::UsernameMismatch));
    }
    if !ENGINE_ID_LEN.contains(&header.engine_id.len()) {
        return Err(Error::AuthFailure(AuthErrorKind::EngineIdMismatch));
    }
    // TEDGE-DOT-PATCH(8): an UNAUTHENTICATED message may not reach the state below.
    //
    // A level above the user's has always been refused. A lower one used to be let through
    // here and dropped by the caller afterwards -- but `authenticate_non_authoritative` has
    // already run by then, and for a sender this engine has not been pinned to it adopts the
    // engine ID the message claims and re-derives the localized keys from it. `verify()` is
    // reached only at AuthNoPriv and above, so an unauthenticated datagram took that branch,
    // returned `Ok`, and escaped the rollback below: one spoofed message permanently replaced
    // a device's trap engine ID, and every genuine trap afterwards failed `EngineIdMismatch`
    // until a restart.
    //
    // The keys are localized TO the engine ID, so the HMAC cannot be checked before the ID is
    // adopted -- which is why this is refused here, before any state is touched, rather than
    // undone afterwards.
    //
    // Only the unauthenticated case is refused, not every lower level: RFC 3414 §4 has the
    // authoritative engine answer an out-of-window inform with an authNoPriv Report (see
    // `report`), which an authPriv sender must be able to read to learn boots and time. That
    // message is authenticated, so it proves its origin and may be trusted with the state; a
    // NoAuthNoPriv user has no keys to protect and is unaffected.
    if header.security_level() > security.security_level() {
        return Err(Error::AuthFailure(AuthErrorKind::UnsupportedSecLevel));
    }
    if header.security_level() < SecurityLevel::AuthNoPriv
        && security.security_level() >= SecurityLevel::AuthNoPriv
    {
        return Err(Error::AuthFailure(AuthErrorKind::UnsupportedSecLevel));
    }
    let saved = security.authoritative_state.clone();
    if let Err(error) = authenticate_non_authoritative(&header, security) {
        security.authoritative_state = saved;
        return Err(error);
    }
    let security: &'a Security = security;
    let scoped = if header.security_level() == SecurityLevel::AuthPriv {
        scoped_content(&security.plain_buf).and_then(parse_scoped)?
    } else {
        parse_scoped(header.data)?
    };
    Ok(message(header, scoped, security))
}

fn authenticate_non_authoritative(header: &Header<'_>, security: &mut Security) -> Result<()> {
    let level = header.security_level();
    if security.engine_id().is_empty() {
        security.authoritative_state.engine_id = header.engine_id.to_vec();
        security.authoritative_state.engine_boots = 0;
        security.authoritative_state.engine_time = 0;
        security.update_key()?;
    } else if security.engine_id() != header.engine_id {
        return Err(Error::AuthFailure(AuthErrorKind::EngineIdMismatch));
    }
    let (latest_boots, latest_time) = (
        security.authoritative_state.engine_boots,
        security.authoritative_state.engine_time,
    );
    if level >= SecurityLevel::AuthNoPriv {
        verify(security, header)?;
        if header.engine_boots == MAX_ENGINE_BOOTS
            || header.engine_boots < latest_boots
            || (header.engine_boots == latest_boots
                && header.engine_time + TIME_WINDOW < latest_time)
        {
            return Err(Error::AuthFailure(AuthErrorKind::EngineTimeMismatch));
        }
    }
    if level == SecurityLevel::AuthPriv {
        security.decrypt_with(
            header.data,
            header.priv_params,
            header.engine_boots,
            header.engine_time,
        )?;
        scoped_content(&security.plain_buf).and_then(parse_scoped)?;
    } else {
        parse_scoped(header.data)?;
    }
    if level >= SecurityLevel::AuthNoPriv
        && (header.engine_boots > latest_boots
            || (header.engine_boots == latest_boots && header.engine_time > latest_time))
    {
        security.authoritative_state.engine_boots = header.engine_boots;
        security.authoritative_state.engine_time = header.engine_time;
        security.authoritative_state.start_time = Instant::now();
    }
    Ok(())
}

/// The header values of an outgoing v3 message.
#[derive(Debug, Clone, Copy)]
pub struct Outgoing<'b> {
    pub msg_id: i32,
    pub level: SecurityLevel,
    /// Set for requests and informs, clear for traps, responses and reports.
    pub reportable: bool,
    /// The authoritative engine: the sender's for a trap, response or report, the
    /// receiver's for a request or inform. The user's keys must be localized to it.
    pub engine_id: &'b [u8],
    pub engine_boots: i64,
    pub engine_time: i64,
    pub context_engine_id: &'b [u8],
    pub context_name: &'b [u8],
}

/// Encode a complete SNMPv3 message for `user` with explicit header values. For a
/// GetBulkRequest `error_status` and `error_index` are non-repeaters and max-repetitions.
pub fn encode(
    user: &Security,
    out: &Outgoing<'_>,
    message_type: MessageType,
    req_id: i32,
    error_status: u32,
    error_index: u32,
    varbinds: &[(&Oid, Value)],
) -> Result<Vec<u8>> {
    if message_type == MessageType::TrapV1 {
        return Err(Error::AsnWrongType);
    }
    let auth = out.level >= SecurityLevel::AuthNoPriv;
    let privacy = out.level == SecurityLevel::AuthPriv;
    if out.level > user.security_level() {
        return Err(Error::AuthFailure(AuthErrorKind::SecurityNotProvided));
    }
    if auth && user.engine_id() != out.engine_id {
        return Err(Error::AuthFailure(AuthErrorKind::SecurityNotReady));
    }

    let mut scoped = Buf::default();
    scoped.push_sequence(|buf| {
        // build_inner writes its fourth argument third and its fifth second.
        pdu::build_inner(req_id, message_type.ident(), varbinds, error_index, error_status, buf);
        buf.push_octet_string(out.context_name);
        buf.push_octet_string(out.context_engine_id);
    });

    let (data, priv_params) = if privacy {
        let (encrypted, salt) = user.encrypt_with(&scoped, out.engine_boots, out.engine_time)?;
        let mut buf = Buf::default();
        buf.push_octet_string(&encrypted);
        (buf.to_vec(), salt)
    } else {
        (scoped.to_vec(), Vec::new())
    };

    let auth_len = if auth {
        user.auth_protocol.truncation_length()
    } else {
        0
    };
    let mut usm = Buf::default();
    usm.push_sequence(|buf| {
        buf.push_octet_string(&priv_params);
        buf.push_octet_string(&vec![0u8; auth_len]);
        buf.push_octet_string(user.username());
        buf.push_integer(out.engine_time);
        buf.push_integer(out.engine_boots);
        buf.push_octet_string(out.engine_id);
    });

    let mut flags = out.level.flags();
    if out.reportable {
        flags |= V3_MSG_FLAGS_REPORTABLE;
    }
    let mut msg = Buf::default();
    msg.push_sequence(|buf| {
        buf.push_chunk(&data);
        buf.push_octet_string(&usm);
        buf.push_sequence(|global| {
            global.push_integer(3); // msgSecurityModel: USM
            global.push_octet_string(&[flags]);
            global.push_integer(BUFFER_SIZE as i64); // msgMaxSize
            global.push_integer(out.msg_id.into());
        });
        buf.push_integer(Version::V3 as i64);
    });

    let mut bytes = msg.to_vec();
    if auth {
        // The authentication parameters end where the privacy parameters' element starts,
        // which is that element plus the message data before the end of the message.
        let end = bytes.len() - data.len() - tlv_len(priv_params.len());
        let mac = user.calculate_hmac(&bytes)?;
        if mac.len() < auth_len {
            return Err(Error::AuthFailure(AuthErrorKind::KeyLengthMismatch));
        }
        bytes[end - auth_len..end].copy_from_slice(&mac[..auth_len]);
    }
    Ok(bytes)
}

struct Scoped<'a> {
    context_engine_id: &'a [u8],
    context_name: &'a [u8],
    message_type: MessageType,
    req_id: i32,
    error_status: u32,
    error_index: u32,
    varbinds: Varbinds<'a>,
}

/// The content of the scopedPDU SEQUENCE a decryption produced (followed by padding).
fn scoped_content(plaintext: &[u8]) -> Result<&[u8]> {
    AsnReader::from_bytes(plaintext).read_raw(asn1::TYPE_SEQUENCE)
}

fn parse_scoped(content: &[u8]) -> Result<Scoped<'_>> {
    let mut rdr = AsnReader::from_bytes(content);
    let context_engine_id = rdr.read_asn_octetstring()?;
    let context_name = rdr.read_asn_octetstring()?;
    let ident = rdr.peek_byte()?;
    let message_type = MessageType::from_ident(ident)?;
    if message_type == MessageType::TrapV1 {
        return Err(Error::AsnWrongType);
    }
    let mut pdu = AsnReader::from_bytes(rdr.read_raw(ident)?);
    let req_id = i32::try_from(pdu.read_asn_integer()?).map_err(|_| Error::ValueOutOfRange)?;
    let error_status =
        u32::try_from(pdu.read_asn_integer()?).map_err(|_| Error::ValueOutOfRange)?;
    let error_index =
        u32::try_from(pdu.read_asn_integer()?).map_err(|_| Error::ValueOutOfRange)?;
    let varbinds = Varbinds::from_bytes(pdu.read_raw(asn1::TYPE_SEQUENCE)?);
    varbinds.check()?;
    Ok(Scoped {
        context_engine_id,
        context_name,
        message_type,
        req_id,
        error_status,
        error_index,
        varbinds,
    })
}

fn message<'a>(header: Header<'a>, scoped: Scoped<'a>, security: &'a Security) -> Message<'a> {
    Message {
        security,
        context_engine_id: scoped.context_engine_id,
        context_name: scoped.context_name,
        pdu: Pdu {
            version: Version::V3 as i64,
            community: header.user_name,
            message_type: scoped.message_type,
            req_id: scoped.req_id,
            error_status: scoped.error_status,
            error_index: scoped.error_index,
            varbinds: scoped.varbinds,
            v1_trap_info: None,
            v3_msg_id: header.msg_id,
        },
        header,
    }
}

/// Check msgAuthenticationParameters: the HMAC of the whole message with them zeroed.
fn verify(user: &Security, header: &Header<'_>) -> Result<()> {
    let len = user.auth_protocol.truncation_length();
    if header.auth_params.len() != len {
        return Err(Error::AuthFailure(AuthErrorKind::SignatureMismatch));
    }
    let mut signed = header.message.to_vec();
    signed[header.auth_offset..header.auth_offset + len].fill(0);
    let mac = user.calculate_hmac(&signed)?;
    let equal = mac.len() >= len
        && mac[..len]
            .iter()
            .zip(header.auth_params)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0;
    if equal {
        Ok(())
    } else {
        Err(Error::AuthFailure(AuthErrorKind::SignatureMismatch))
    }
}

fn non_negative_i32(value: i64) -> Result<i32> {
    i32::try_from(value)
        .ok()
        .filter(|v| *v >= 0)
        .ok_or(Error::ValueOutOfRange)
}

/// Where `inner`, a sub-slice of `outer`, starts within it.
fn offset_in(outer: &[u8], inner: &[u8]) -> usize {
    inner.as_ptr() as usize - outer.as_ptr() as usize
}

/// The encoded size of an element whose content is `len` octets (one-octet tag).
fn tlv_len(len: usize) -> usize {
    let length_octets = if len < 0x80 {
        1
    } else {
        1 + (usize::BITS - len.leading_zeros()).div_ceil(8) as usize
    };
    1 + length_octets + len
}

fn clone_value<'a>(value: &Value<'a>) -> Value<'a> {
    match value {
        Value::Boolean(v) => Value::Boolean(*v),
        Value::Null => Value::Null,
        Value::Integer(v) => Value::Integer(*v),
        Value::OctetString(v) => Value::OctetString(v),
        Value::ObjectIdentifier(v) => Value::ObjectIdentifier(v.clone()),
        Value::Sequence(v) => Value::Sequence(v.clone()),
        Value::Set(v) => Value::Set(v.clone()),
        Value::Constructed(t, v) => Value::Constructed(*t, v.clone()),
        Value::IpAddress(v) => Value::IpAddress(*v),
        Value::Counter32(v) => Value::Counter32(*v),
        Value::Unsigned32(v) => Value::Unsigned32(*v),
        Value::Timeticks(v) => Value::Timeticks(*v),
        Value::Opaque(v) => Value::Opaque(v),
        Value::Counter64(v) => Value::Counter64(*v),
        Value::EndOfMibView => Value::EndOfMibView,
        Value::NoSuchObject => Value::NoSuchObject,
        Value::NoSuchInstance => Value::NoSuchInstance,
        Value::Unknown(t, v) => Value::Unknown(*t, v),
        Value::GetRequest(v) => Value::GetRequest(v.clone()),
        Value::GetNextRequest(v) => Value::GetNextRequest(v.clone()),
        Value::GetBulkRequest(v) => Value::GetBulkRequest(v.clone()),
        Value::Response(v) => Value::Response(v.clone()),
        Value::SetRequest(v) => Value::SetRequest(v.clone()),
        Value::InformRequest(v) => Value::InformRequest(v.clone()),
        Value::Trap(v) => Value::Trap(v.clone()),
        Value::Report(v) => Value::Report(v.clone()),
    }
}
