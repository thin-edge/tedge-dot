//! The notification listener (spec §4): one UDP socket, the routing of every datagram to the
//! device it belongs to, the security checks, the acknowledgement of informs, and the samples a
//! notification produces.
//!
//! The socket is bound only while at least one device has a notification point, and without
//! `SO_REUSEADDR`: a port another receiver holds must fail loudly rather than take half of a
//! host's traps.

use crate::config::Point;
use crate::config::PointKind;
use crate::notification::{self, Notification, PduKind, Version};
use crate::value::{dotted_ip, Oid};
use snmp2::v3::{LocalEngine, Security, SecurityLevel};
use snmp2::{MessageType, Pdu};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tedge_dot_sdk::{DataType, Mode, Sample, SampleSink, Value};
use time::OffsetDateTime;
use tokio::net::UdpSocket;
use tracing::{debug, warn};

/// How often one kind of problem is logged for one source or device.
const LOG_INTERVAL: Duration = Duration::from_secs(60);

/// Largest UDP payload.
const MAX_DATAGRAM: usize = 65_535;

/// Everything the receive task needs. Shared with the connector behind a `std` mutex that is
/// never held across an await.
#[derive(Default)]
pub struct Routes {
    /// Resolved device address → index into `devices`.
    pub by_addr: HashMap<IpAddr, usize>,
    /// Resolved addresses of the trusted trap forwarders.
    pub forwarders: HashSet<IpAddr>,
    /// One route per configured device, in configuration order.
    pub devices: Vec<Route>,
    /// This receiver's SNMP engine: authoritative for v3 informs.
    pub engine: Option<LocalEngine>,
    /// Why the receive task stopped, once it has.
    pub listener_error: Option<String>,
}

pub struct Route {
    pub name: String,
    /// Communities accepted on v1/v2c notifications; `None` accepts any.
    pub communities: Option<Vec<Vec<u8>>>,
    pub v3: Option<V3Receive>,
    /// Subscribed notification points, in configuration order.
    pub points: Vec<Point>,
    /// Set by `subscribe`; a device without one is authenticated and acknowledged, but its
    /// samples go nowhere yet.
    pub sink: Option<SampleSink>,
}

/// A device's v3 state for received notifications: the same user in both roles — the sender's
/// engine for traps, ours for informs.
#[derive(Clone)]
pub struct V3Receive {
    pub user: Vec<u8>,
    /// The security level the device's notifications must reach.
    pub level: SecurityLevel,
    /// Keys for the device's own engine (learned from the first authenticated trap when the
    /// configuration does not pin one).
    pub trap: Security,
    /// Keys localized to this receiver's engine, for informs.
    pub inform: Security,
}

/// A notification accepted for a device, and the datagram that answers it.
pub struct Received {
    pub notification: Notification,
    pub reply: Option<Vec<u8>>,
}

/// What one datagram amounts to for one device.
enum Outcome {
    Deliver(Box<Received>),
    /// Nothing to publish, but this datagram goes back to the sender: the USM Report that tells
    /// an inform sender our engine ID, boots and time.
    Reply(Vec<u8>, String),
    Drop(String),
}

pub struct Delivery {
    pub device: String,
    pub sink: Option<SampleSink>,
    pub samples: Vec<Sample>,
    pub reply: Option<Vec<u8>>,
}

pub fn lock(routes: &Mutex<Routes>) -> MutexGuard<'_, Routes> {
    routes.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Bind without `SO_REUSEADDR`. An IPv6 wildcard also receives IPv4, whatever the system default.
pub fn bind(listen: SocketAddr) -> std::io::Result<UdpSocket> {
    let domain = socket2::Domain::for_address(listen);
    let socket = socket2::Socket::new(domain, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))?;
    if listen.is_ipv6() {
        socket.set_only_v6(false)?;
    }
    socket.set_nonblocking(true)?;
    socket.bind(&listen.into())?;
    UdpSocket::from_std(socket.into())
}

/// Compare an IPv4-mapped IPv6 address (what a dual-stack socket reports) as IPv4.
pub fn normalize(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        v4 => v4,
    }
}

/// Receive errors that say nothing about the socket: an interrupted call, or an ICMP error left
/// behind by an earlier send (an acknowledgement to a sender that has gone away).
fn transient(e: &std::io::Error) -> bool {
    use std::io::ErrorKind::*;
    matches!(e.kind(), Interrupted | WouldBlock | ConnectionRefused | ConnectionReset)
}

pub async fn receive_loop(socket: Arc<UdpSocket>, routes: Arc<Mutex<Routes>>) {
    let mut buf = vec![0u8; MAX_DATAGRAM];
    let mut limiter = LogLimiter::default();
    loop {
        let (len, peer) = match socket.recv_from(&mut buf).await {
            Ok(received) => received,
            Err(e) if transient(&e) => continue,
            Err(e) => {
                warn!("SNMP listener failed: {e}");
                lock(&routes).listener_error = Some(format!("SNMP listener failed: {e}"));
                return;
            }
        };
        let ts = OffsetDateTime::now_utc();
        let Some(delivery) = handle(&routes, &mut limiter, &buf[..len], peer, ts) else {
            continue;
        };
        if let Some(reply) = delivery.reply {
            if let Err(e) = socket.send_to(&reply, peer).await {
                warn!(device = %delivery.device, "cannot answer {peer}: {e}");
            }
        }
        let Some(sink) = delivery.sink else { continue };
        for sample in delivery.samples {
            if sink.send(sample).await.is_err() {
                debug!(device = %delivery.device, "sample sink closed");
                break;
            }
        }
    }
}

/// Route one datagram (spec §4.1) and build its samples. `None` when it is dropped.
fn handle(
    routes: &Mutex<Routes>,
    limiter: &mut LogLimiter,
    datagram: &[u8],
    peer: SocketAddr,
    ts: OffsetDateTime,
) -> Option<Delivery> {
    let peer_ip = normalize(peer.ip());
    let mut guard = lock(routes);
    let routes = &mut *guard;
    let direct = routes.by_addr.get(&peer_ip).copied();
    let forwarded = routes.forwarders.contains(&peer_ip);
    if direct.is_none() && !forwarded {
        if limiter.allow(format!("unknown {peer_ip}")) {
            debug!(source = %peer_ip, "notification from a source no device is configured for; ignored");
        }
        return None;
    }

    // From a trusted forwarder the source address is the forwarder's, so the device is the one
    // the notification's last snmpTrapAddress.0 names — which only the decoded message tells us.
    let candidates: Vec<usize> = match (forwarded, direct) {
        (true, _) => (0..routes.devices.len()).collect(),
        (false, Some(index)) => vec![index],
        (false, None) => return None,
    };

    let mut refusal: Option<(usize, Outcome)> = None;
    for index in candidates {
        let saved = routes.devices[index].v3.clone();
        let engine = routes.engine.as_mut();
        let outcome = receive(&mut routes.devices[index], engine, datagram);
        let Outcome::Deliver(received) = outcome else {
            routes.devices[index].v3 = saved;
            // A Report outranks a Drop whichever device produced it. From a forwarder every
            // device is a candidate, so an earlier one refusing the datagram (no v3 table, a
            // different user) used to bury the USM discovery Report the intended device just
            // produced: the sender never learned this engine's ID, boots and time, so its
            // inform was never acknowledged and it re-sent forever (spec §4.1 step 4).
            let outranks = match (&refusal, &outcome) {
                (None, _) => true,
                (Some((_, Outcome::Reply(..))), _) => false,
                (Some(_), Outcome::Reply(..)) => true,
                _ => false,
            };
            if outranks {
                refusal = Some((index, outcome));
            }
            continue;
        };
        let source = if forwarded {
            // Without snmpTrapAddress.0 nothing says whose notification this is: drop it.
            match received.notification.trap_address().map(normalize) {
                Some(address) if routes.by_addr.get(&address) == Some(&index) => address,
                _ => {
                    routes.devices[index].v3 = saved;
                    continue;
                }
            }
        } else {
            peer_ip
        };
        let route = &routes.devices[index];
        return Some(Delivery {
            device: route.name.clone(),
            sink: route.sink.clone(),
            samples: samples_for(
                &route.name,
                &route.points,
                &received.notification,
                source,
                forwarded.then_some(peer_ip),
                ts,
            ),
            reply: received.reply,
        });
    }

    match refusal {
        Some((index, Outcome::Reply(reply, why))) => {
            debug!(device = %routes.devices[index].name, source = %peer_ip, "answering with a report: {why}");
            Some(Delivery {
                device: routes.devices[index].name.clone(),
                sink: None,
                samples: Vec::new(),
                reply: Some(reply),
            })
        }
        Some((index, Outcome::Drop(why))) => {
            if limiter.allow(format!("dropped {peer_ip} {index}")) {
                warn!(device = %routes.devices[index].name, source = %peer_ip, "dropped a notification: {why}");
            }
            None
        }
        _ => {
            if limiter.allow(format!("unrouted {peer_ip}")) {
                warn!(source = %peer_ip, "dropped a forwarded notification no device matches");
            }
            None
        }
    }
}

/// Decode and authenticate one datagram for one device.
fn receive(route: &mut Route, engine: Option<&mut LocalEngine>, datagram: &[u8]) -> Outcome {
    if let Ok(header) = snmp2::v3::Header::parse(datagram) {
        return receive_v3(route, engine, datagram, &header);
    }
    let pdu = match Pdu::from_bytes(datagram) {
        Ok(pdu) => pdu,
        Err(e) => return Outcome::Drop(format!("it does not decode: {e}")),
    };
    let version = match pdu.version() {
        Ok(snmp2::Version::V1) => Version::V1,
        Ok(snmp2::Version::V2C) => Version::V2c,
        _ => return Outcome::Drop("an SNMPv3 message with no readable header".into()),
    };
    if let Some(accepted) = &route.communities {
        if !accepted.iter().any(|c| c.as_slice() == pdu.community) {
            return Outcome::Drop("the community is not one this device accepts".into());
        }
    }
    let notification = match notification::from_pdu(&pdu, version) {
        Ok(notification) => notification,
        Err(e) => return Outcome::Drop(e.to_string()),
    };
    let reply = match notification.pdu {
        PduKind::Inform => match notification::inform_response(&pdu) {
            Ok(reply) => Some(reply),
            Err(e) => {
                warn!(device = %route.name, "cannot build the Response to an inform: {e}");
                None
            }
        },
        PduKind::Trap => None,
    };
    Outcome::Deliver(Box::new(Received { notification, reply }))
}

/// A v3 message: addressed to this receiver's engine (an inform, or the discovery that precedes
/// it) when it asks for a Report, and sent under the device's own engine (a trap) otherwise.
fn receive_v3(
    route: &mut Route,
    engine: Option<&mut LocalEngine>,
    datagram: &[u8],
    header: &snmp2::v3::Header<'_>,
) -> Outcome {
    let Some(v3) = route.v3.as_mut() else {
        return Outcome::Drop("the device has no SNMPv3 credentials".into());
    };
    let known_user = header.user_name == v3.user;
    if header.is_reportable() {
        let Some(engine) = engine else {
            return Outcome::Drop("this connector has no SNMP engine of its own".into());
        };
        let level = v3.level;
        match engine.receive(datagram, known_user.then_some(&mut v3.inform)) {
            Ok(message) => {
                if message.header.security_level() < level {
                    return Outcome::Drop(
                        "the inform is below the security level configured for this device".into(),
                    );
                }
                if message.pdu.message_type != MessageType::InformRequest {
                    return Outcome::Drop(format!(
                        "a v3 {:?} is not a notification",
                        message.pdu.message_type
                    ));
                }
                let notification = match notification::from_pdu(&message.pdu, Version::V3) {
                    Ok(notification) => notification,
                    Err(e) => return Outcome::Drop(e.to_string()),
                };
                let reply = match engine.acknowledge(&message) {
                    Ok(reply) => Some(reply),
                    Err(e) => {
                        warn!(device = %route.name, "cannot acknowledge a v3 inform: {e}");
                        None
                    }
                };
                Outcome::Deliver(Box::new(Received { notification, reply }))
            }
            Err(refusal) => match refusal.report {
                Some(report) => Outcome::Reply(report, refusal.error.to_string()),
                None => Outcome::Drop(refusal.error.to_string()),
            },
        }
    } else {
        if !known_user {
            return Outcome::Drop("the v3 user is not the one configured for this device".into());
        }
        let level = v3.level;
        match snmp2::v3::receive_non_authoritative(datagram, &mut v3.trap) {
            Ok(message) => {
                if message.header.security_level() < level {
                    return Outcome::Drop(
                        "the trap is below the security level configured for this device".into(),
                    );
                }
                if message.pdu.message_type != MessageType::Trap {
                    return Outcome::Drop(format!(
                        "a v3 {:?} is not a notification",
                        message.pdu.message_type
                    ));
                }
                match notification::from_pdu(&message.pdu, Version::V3) {
                    Ok(notification) => Outcome::Deliver(Box::new(Received { notification, reply: None })),
                    Err(e) => Outcome::Drop(e.to_string()),
                }
            }
            Err(e) => Outcome::Drop(e.to_string()),
        }
    }
}

/// The samples one notification produces, in configuration order (spec §3.3, §4.1).
pub fn samples_for(
    device: &str,
    points: &[Point],
    n: &Notification,
    source: IpAddr,
    forwarder: Option<IpAddr>,
    ts: OffsetDateTime,
) -> Vec<Sample> {
    let mut out = Vec::new();
    for p in points {
        let matches = p.traps().iter().any(|t| n.trap.starts_with(t));
        if !matches {
            continue;
        }
        match &p.kind {
            PointKind::Object { .. } => {}
            PointKind::Trap { .. } => {
                let value = match p.datatype {
                    Some(DataType::Bool) => Ok(Value::Bool(true)),
                    _ => Ok(Value::Text(n.trap.to_string())),
                };
                out.push(crate::sample(
                    device,
                    p,
                    ts,
                    address(n, source, forwarder, None),
                    n.trap.encode(),
                    value,
                ));
            }
            PointKind::Varbind { oid, .. } => match n.varbinds.iter().find(|v| v.name.starts_with(oid)) {
                Some(vb) => {
                    let value = match p.datatype {
                        // As in the polled path: raw mode never runs the typed conversion, so a
                        // point that has both `mode = "raw"` and a `datatype` is not reported
                        // bad for a value it was never going to decode.
                        Some(_) if p.mode == Mode::Raw => Ok(Value::Bool(false)),
                        Some(datatype) => vb.value.convert(datatype).map(|v| p.transform.apply(v)),
                        // raw mode: the value is never looked at.
                        None => Ok(Value::Bool(false)),
                    };
                    out.push(crate::sample(
                        device,
                        p,
                        ts,
                        address(n, source, forwarder, Some(&vb.name)),
                        vb.value.raw.clone(),
                        value,
                    ));
                }
                None => out.push(crate::sample(
                    device,
                    p,
                    ts,
                    address(n, source, forwarder, None),
                    Vec::new(),
                    Err(format!("notification {} carried no varbind under {oid}", n.trap)),
                )),
            },
        }
    }
    out
}

/// The `addr` of a notification sample (spec §6).
fn address(
    n: &Notification,
    source: IpAddr,
    forwarder: Option<IpAddr>,
    varbind: Option<&Oid>,
) -> serde_json::Value {
    let mut addr = serde_json::json!({
        "source": source.to_string(),
        "version": n.version.as_str(),
        "pdu": n.pdu.as_str(),
        "trap": n.trap.to_string(),
    });
    if let Some(oid) = varbind {
        addr["oid"] = serde_json::Value::String(oid.to_string());
    }
    if let Some(forwarder) = forwarder {
        addr["forwarder"] = serde_json::Value::String(forwarder.to_string());
    }
    if let Some(agent) = &n.agent_addr {
        addr["agent_addr"] = serde_json::Value::String(dotted_ip(agent));
    }
    addr
}

/// Logs one kind of problem at most once per [`LOG_INTERVAL`] per key, so a device spraying
/// malformed datagrams, or a scanner probing port 162, cannot flood the journal.
#[derive(Default)]
pub struct LogLimiter {
    last: HashMap<String, Instant>,
}

/// The most keys held at once. A key is one source address or device, so a real deployment
/// stays far below it; the cap exists for a flood of forged sources, where dropping the whole
/// table costs at most one extra log line per key afterwards.
const MAX_KEYS: usize = 4096;

impl LogLimiter {
    pub fn allow(&mut self, key: String) -> bool {
        let now = Instant::now();
        if self.last.len() >= MAX_KEYS {
            // Expiring by age alone bounds nothing: under a sustained flood of distinct
            // sources nothing is old enough to drop, so the table kept growing AND every
            // datagram paid a full scan of it -- the limiter became the amplifier it exists
            // to prevent. Clearing is O(n) once per MAX_KEYS keys rather than per datagram.
            self.last.retain(|_, at| now.duration_since(*at) < LOG_INTERVAL);
            if self.last.len() >= MAX_KEYS {
                self.last.clear();
            }
        }
        match self.last.get(&key) {
            Some(at) if now.duration_since(*at) < LOG_INTERVAL => false,
            _ => {
                self.last.insert(key, now);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_mapped_sources_compare_as_ipv4() {
        let mapped: IpAddr = "::ffff:192.168.1.20".parse().unwrap();
        assert_eq!(normalize(mapped), "192.168.1.20".parse::<IpAddr>().unwrap());
        let v6: IpAddr = "2001:db8::1".parse().unwrap();
        assert_eq!(normalize(v6), v6);
    }

    #[test]
    fn the_log_limiter_allows_one_per_key() {
        let mut limiter = LogLimiter::default();
        assert!(limiter.allow("a".into()));
        assert!(!limiter.allow("a".into()));
        assert!(limiter.allow("b".into()));
    }
}
