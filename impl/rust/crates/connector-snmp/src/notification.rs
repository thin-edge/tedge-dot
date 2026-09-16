//! Notifications (spec §4): a decoded message turned into what the points match against, under
//! the connector-owned semantics of §4.3.
//!
//! Parsing is `snmp2`'s ([`snmp2::Pdu::from_bytes`] for v1/v2c, `snmp2::v3` for v3). On top of
//! it: only Trap-PDU (v1), SNMPv2-Trap-PDU and InformRequest-PDU are notifications; the trap OID
//! (RFC 3584 for v1, the first `snmpTrapOID.0` otherwise); the uptime; the limits.

use crate::value::{DecodeError, Decoded, Oid, ValueType, Varbind, MAX_ARCS};
use snmp2::{MessageType, Pdu};
use std::net::IpAddr;

/// Most variable bindings a notification may carry.
pub const MAX_VARBINDS: usize = 256;
/// `sysUpTime.0`.
pub const SYS_UPTIME: &[u32] = &[1, 3, 6, 1, 2, 1, 1, 3, 0];
/// `snmpTrapOID.0`, whose value names a v2c/v3 notification.
pub const SNMP_TRAP_OID: &[u32] = &[1, 3, 6, 1, 6, 3, 1, 1, 4, 1, 0];
/// `snmpTrapAddress.0`, which a trap forwarder adds with the original sender's address.
pub const SNMP_TRAP_ADDRESS: &[u32] = &[1, 3, 6, 1, 6, 3, 18, 1, 3, 0];
/// The generic traps of RFC 3584 §3.1: v1 `generic-trap` n maps to this prefix plus n + 1.
const GENERIC_TRAPS: &[u32] = &[1, 3, 6, 1, 6, 3, 1, 1, 5];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    V1,
    V2c,
    V3,
}

impl Version {
    pub fn as_str(self) -> &'static str {
        match self {
            Version::V1 => "v1",
            Version::V2c => "v2c",
            Version::V3 => "v3",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PduKind {
    Trap,
    Inform,
}

impl PduKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PduKind::Trap => "trap",
            PduKind::Inform => "inform",
        }
    }
}

/// A decoded notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub version: Version,
    /// v1/v2c: the community; v3: the user name.
    pub community: Vec<u8>,
    pub pdu: PduKind,
    /// v2c/v3 only.
    pub request_id: Option<i64>,
    /// v1 only.
    pub agent_addr: Option<[u8; 4]>,
    /// v1 `time-stamp`, or the first `sysUpTime.0` varbind when it is TimeTicks.
    pub uptime: Option<u32>,
    /// The notification OID (§4.3); v1 traps are mapped per RFC 3584.
    pub trap: Oid,
    pub varbinds: Vec<Varbind>,
}

impl Notification {
    /// The value of the last `snmpTrapAddress.0` varbind, when it is an IpAddress.
    pub fn trap_address(&self) -> Option<IpAddr> {
        self.varbinds
            .iter()
            .rev()
            .find(|v| v.name.arcs() == SNMP_TRAP_ADDRESS)
            .and_then(|v| match v.value.decoded {
                Decoded::IpAddress(ip) => Some(IpAddr::from(ip)),
                _ => None,
            })
    }
}

/// Decode a v1/v2c datagram into a notification. A v3 message needs credentials: see
/// [`crate::listener`].
pub fn decode(datagram: &[u8]) -> Result<Notification, DecodeError> {
    let pdu = Pdu::from_bytes(datagram)?;
    let version = match pdu.version()? {
        snmp2::Version::V1 => Version::V1,
        snmp2::Version::V2C => Version::V2c,
        snmp2::Version::V3 => return Err(DecodeError::new("SNMPv3 message without credentials")),
    };
    from_pdu(&pdu, version)
}

/// The §4.3 semantics over a parsed message of `version`.
pub fn from_pdu(pdu: &Pdu<'_>, version: Version) -> Result<Notification, DecodeError> {
    let kind = match (version, pdu.message_type) {
        (Version::V1, MessageType::TrapV1) => PduKind::Trap,
        (Version::V1, _) => return Err(DecodeError::new("SNMPv1 message is not a Trap-PDU")),
        (_, MessageType::Trap) => PduKind::Trap,
        (_, MessageType::InformRequest) => PduKind::Inform,
        _ => return Err(DecodeError::new("not a notification PDU")),
    };

    let mut varbinds = Vec::new();
    for (name, value) in pdu.varbinds.clone() {
        if varbinds.len() == MAX_VARBINDS {
            return Err(DecodeError::new(format!("more than {MAX_VARBINDS} varbinds")));
        }
        varbinds.push(Varbind::from_library(&name, &value)?);
    }

    let (trap, uptime, agent_addr, request_id) = match &pdu.v1_trap_info {
        Some(v1) => {
            let trap = match v1.generic_trap {
                generic @ 0..=5 => {
                    let mut arcs = GENERIC_TRAPS.to_vec();
                    arcs.push(generic as u32 + 1);
                    arcs
                }
                6 => {
                    let specific = u32::try_from(v1.specific_trap).map_err(|_| {
                        DecodeError::new("specific-trap is not a 32-bit unsigned integer")
                    })?;
                    let mut arcs = Oid::from_library(&v1.enterprise)?.arcs().to_vec();
                    arcs.extend([0, specific]);
                    if arcs.len() > MAX_ARCS {
                        return Err(DecodeError::new("trap OID has too many arcs"));
                    }
                    arcs
                }
                _ => return Err(DecodeError::new("generic-trap out of range")),
            };
            let agent = match v1.agent_addr {
                IpAddr::V4(v4) => v4.octets(),
                IpAddr::V6(_) => return Err(DecodeError::new("agent-addr is not IPv4")),
            };
            let trap = Oid::parse(&dotted(&trap)).map_err(DecodeError::new)?;
            (trap, Some(v1.timestamp), Some(agent), None)
        }
        None => {
            let named = varbinds
                .iter()
                .find(|v| v.name.arcs() == SNMP_TRAP_OID)
                .ok_or_else(|| DecodeError::new("notification has no snmpTrapOID.0"))?;
            let Decoded::Oid(trap) = &named.value.decoded else {
                return Err(DecodeError::new("snmpTrapOID.0 is not an OID"));
            };
            let uptime = varbinds
                .iter()
                .find(|v| v.name.arcs() == SYS_UPTIME)
                .and_then(|v| match (v.value.kind, &v.value.decoded) {
                    (ValueType::TimeTicks, Decoded::Unsigned(t)) => u32::try_from(*t).ok(),
                    _ => None,
                });
            (trap.clone(), uptime, None, Some(i64::from(pdu.req_id)))
        }
    };

    Ok(Notification {
        version,
        community: pdu.community.to_vec(),
        pdu: kind,
        request_id,
        agent_addr,
        uptime,
        trap,
        varbinds,
    })
}

/// The Response that acknowledges a v1/v2c InformRequest: its request-id and varbinds, no error.
pub fn inform_response(pdu: &Pdu<'_>) -> Result<Vec<u8>, DecodeError> {
    let mut response = pdu.clone();
    response.message_type = MessageType::Response;
    response.error_status = 0;
    response.error_index = 0;
    Ok(response.to_bytes()?)
}

fn dotted(arcs: &[u32]) -> String {
    arcs.iter().map(u32::to_string).collect::<Vec<_>>().join(".")
}
