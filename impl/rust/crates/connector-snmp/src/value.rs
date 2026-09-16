//! Values as the connector sees them (spec §6): OIDs as 32-bit arcs, varbind values with their
//! canonical content octets, and the conversion of a value to a point datatype.
//!
//! Decoding the wire format is the library's job (`snmp2`, patched in `vendor/snmp2`). What is
//! here is what the library does not provide: OID arcs split correctly for every first arc
//! (asn1-rs reads the first two arcs from a single octet), the canonical content octets `raw`
//! carries, and the datatype conversions shared with the C build through the golden vectors.

use std::borrow::Cow;
use std::fmt;
use tedge_dot_sdk::{DataType, Value};

/// Most arcs an OID may have (spec §3.3, §4.3).
pub const MAX_ARCS: usize = 128;

/// Why a value or a message is not accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError(pub String);

impl DecodeError {
    pub fn new(msg: impl Into<String>) -> DecodeError {
        DecodeError(msg.into())
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DecodeError {}

impl From<snmp2::Error> for DecodeError {
    fn from(e: snmp2::Error) -> DecodeError {
        DecodeError(e.to_string())
    }
}

/// An object identifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Oid(Vec<u32>);

impl Oid {
    pub fn arcs(&self) -> &[u32] {
        &self.0
    }

    /// Parse dotted decimal as a configuration writes it (spec §3.3): an optional leading `.`,
    /// 2 to [`MAX_ARCS`] arcs, the first 0–2, the second below 40 unless the first is 2, every arc
    /// a 32-bit unsigned integer (and, for the second, small enough to encode).
    pub fn parse(text: &str) -> Result<Oid, String> {
        let body = text.strip_prefix('.').unwrap_or(text);
        let mut arcs = Vec::new();
        for part in body.split('.') {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("'{text}' is not a dotted-decimal OID"));
            }
            let arc = part
                .parse::<u32>()
                .map_err(|_| format!("OID '{text}' has an arc larger than 32 bits"))?;
            arcs.push(arc);
        }
        if arcs.len() < 2 {
            return Err(format!("OID '{text}' needs at least two arcs"));
        }
        if arcs.len() > MAX_ARCS {
            return Err(format!("OID '{text}' has more than {MAX_ARCS} arcs"));
        }
        let second_limit = match arcs[0] {
            0 | 1 => 39,
            2 => u32::MAX - 80,
            _ => return Err(format!("OID '{text}' must start with 0, 1 or 2")),
        };
        if arcs[1] > second_limit {
            return Err(format!("OID '{text}' has a second arc out of range"));
        }
        Ok(Oid(arcs))
    }

    /// True when `self` equals `prefix` or lies beneath it.
    pub fn starts_with(&self, prefix: &Oid) -> bool {
        self.0.starts_with(&prefix.0)
    }

    /// The OID without its last arc, when that still has two arcs.
    pub fn parent(&self) -> Option<Oid> {
        (self.0.len() > 2).then(|| Oid(self.0[..self.0.len() - 1].to_vec()))
    }

    /// The content octets of the canonical (minimal) BER encoding.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.0.len() + 4);
        let first = u64::from(self.0.first().copied().unwrap_or(0)) * 40
            + u64::from(self.0.get(1).copied().unwrap_or(0));
        push_subid(&mut out, first);
        for &arc in self.0.iter().skip(2) {
            push_subid(&mut out, u64::from(arc));
        }
        out
    }

    /// The library's form, for a request.
    pub fn to_library(&self) -> snmp2::Oid<'static> {
        snmp2::Oid::new(Cow::Owned(self.encode()))
    }

    /// Split the content octets of an OID into arcs. The library has already checked them
    /// (non-empty, no dangling continuation, 32-bit sub-identifiers); the arc count is ours.
    pub fn from_ber(content: &[u8]) -> Result<Oid, DecodeError> {
        if content.is_empty() {
            return Err(DecodeError::new("empty OID"));
        }
        let mut subids: Vec<u32> = Vec::new();
        let mut value: u64 = 0;
        let mut continued = false;
        for &b in content {
            value = (value << 7) | u64::from(b & 0x7F);
            let Ok(arc) = u32::try_from(value) else {
                return Err(DecodeError::new("OID arc larger than 32 bits"));
            };
            continued = b & 0x80 != 0;
            if !continued {
                if subids.len() >= MAX_ARCS {
                    return Err(DecodeError::new("OID has too many arcs"));
                }
                subids.push(arc);
                value = 0;
            }
        }
        if continued {
            return Err(DecodeError::new("truncated OID"));
        }
        let first = subids[0];
        let mut arcs = Vec::with_capacity(subids.len() + 1);
        match first {
            0..=39 => arcs.extend([0, first]),
            40..=79 => arcs.extend([1, first - 40]),
            _ => arcs.extend([2, first - 80]),
        }
        arcs.extend_from_slice(&subids[1..]);
        if arcs.len() > MAX_ARCS {
            return Err(DecodeError::new("OID has too many arcs"));
        }
        Ok(Oid(arcs))
    }

    pub fn from_library(oid: &snmp2::Oid<'_>) -> Result<Oid, DecodeError> {
        Oid::from_ber(oid.as_bytes())
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, arc) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(".")?;
            }
            write!(f, "{arc}")?;
        }
        Ok(())
    }
}

fn push_subid(out: &mut Vec<u8>, mut value: u64) {
    let mut chunk = [0u8; 10];
    let mut n = 0;
    loop {
        chunk[n] = (value & 0x7F) as u8 | if n > 0 { 0x80 } else { 0 };
        n += 1;
        value >>= 7;
        if value == 0 {
            break;
        }
    }
    out.extend(chunk[..n].iter().rev());
}

/// The type of a varbind value (spec §6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    Integer,
    OctetString,
    Null,
    Oid,
    IpAddress,
    Counter32,
    Gauge32,
    TimeTicks,
    Opaque,
    Counter64,
    NoSuchObject,
    NoSuchInstance,
    EndOfMibView,
    /// Any other tag; the tag is kept.
    Unknown(u8),
}

impl ValueType {
    /// The type's name in the golden vectors.
    pub fn name(self) -> &'static str {
        match self {
            ValueType::Integer => "integer",
            ValueType::OctetString => "octet_string",
            ValueType::Null => "null",
            ValueType::Oid => "oid",
            ValueType::IpAddress => "ip_address",
            ValueType::Counter32 => "counter32",
            ValueType::Gauge32 => "gauge32",
            ValueType::TimeTicks => "timeticks",
            ValueType::Opaque => "opaque",
            ValueType::Counter64 => "counter64",
            ValueType::NoSuchObject => "no_such_object",
            ValueType::NoSuchInstance => "no_such_instance",
            ValueType::EndOfMibView => "end_of_mib_view",
            ValueType::Unknown(_) => "unknown",
        }
    }

    pub fn is_exception(self) -> bool {
        matches!(self, ValueType::NoSuchObject | ValueType::NoSuchInstance | ValueType::EndOfMibView)
    }
}

/// What a value decodes to beyond its octets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decoded {
    Signed(i64),
    Unsigned(u64),
    Oid(Oid),
    IpAddress([u8; 4]),
    /// OCTET STRING, Opaque, NULL, the exceptions and unknown types: nothing beyond `raw`.
    Octets,
}

/// One varbind value: its type, its canonical content octets (spec §6), and what they decode to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VarValue {
    pub kind: ValueType,
    pub raw: Vec<u8>,
    pub decoded: Decoded,
}

impl VarValue {
    /// Take a value the library decoded. `raw` becomes the canonical content octets: minimal
    /// two's complement for every integer type (so an unsigned value with its top bit set gets a
    /// leading zero), the canonical OID encoding, and anything else as received.
    pub fn from_library(value: &snmp2::Value<'_>) -> Result<VarValue, DecodeError> {
        use snmp2::Value as V;
        let unsigned = |kind, v: u64| (kind, encode_unsigned(v), Decoded::Unsigned(v));
        let (kind, raw, decoded) = match value {
            V::Integer(v) => (ValueType::Integer, encode_integer(*v), Decoded::Signed(*v)),
            V::OctetString(s) => (ValueType::OctetString, s.to_vec(), Decoded::Octets),
            V::Null => (ValueType::Null, Vec::new(), Decoded::Octets),
            V::ObjectIdentifier(oid) => {
                let oid = Oid::from_library(oid)?;
                (ValueType::Oid, oid.encode(), Decoded::Oid(oid))
            }
            V::IpAddress(ip) => (ValueType::IpAddress, ip.to_vec(), Decoded::IpAddress(*ip)),
            V::Counter32(v) => unsigned(ValueType::Counter32, u64::from(*v)),
            V::Unsigned32(v) => unsigned(ValueType::Gauge32, u64::from(*v)),
            V::Timeticks(v) => unsigned(ValueType::TimeTicks, u64::from(*v)),
            V::Counter64(v) => unsigned(ValueType::Counter64, *v),
            V::Opaque(s) => (ValueType::Opaque, s.to_vec(), Decoded::Octets),
            V::NoSuchObject => (ValueType::NoSuchObject, Vec::new(), Decoded::Octets),
            V::NoSuchInstance => (ValueType::NoSuchInstance, Vec::new(), Decoded::Octets),
            V::EndOfMibView => (ValueType::EndOfMibView, Vec::new(), Decoded::Octets),
            V::Boolean(b) => (ValueType::Unknown(0x01), vec![u8::from(*b)], Decoded::Octets),
            V::Unknown(tag, content) => (ValueType::Unknown(*tag), content.to_vec(), Decoded::Octets),
            V::Sequence(r) => (ValueType::Unknown(0x30), r.remaining().to_vec(), Decoded::Octets),
            V::Set(r) => (ValueType::Unknown(0x31), r.remaining().to_vec(), Decoded::Octets),
            V::Constructed(tag, r) => (ValueType::Unknown(*tag), r.remaining().to_vec(), Decoded::Octets),
            V::GetRequest(r)
            | V::GetNextRequest(r)
            | V::GetBulkRequest(r)
            | V::Response(r)
            | V::SetRequest(r)
            | V::InformRequest(r)
            | V::Trap(r)
            | V::Report(r) => {
                let tag = match value {
                    V::GetRequest(_) => 0xA0,
                    V::GetNextRequest(_) => 0xA1,
                    V::Response(_) => 0xA2,
                    V::SetRequest(_) => 0xA3,
                    V::GetBulkRequest(_) => 0xA5,
                    V::InformRequest(_) => 0xA6,
                    V::Trap(_) => 0xA7,
                    _ => 0xA8,
                };
                (ValueType::Unknown(tag), r.remaining().to_vec(), Decoded::Octets)
            }
        };
        Ok(VarValue { kind, raw, decoded })
    }

    /// The value's text form in the golden vectors: decimal for integers, dotted for OIDs and
    /// addresses, hex for octet runs, `None` for NULL, the exceptions and unknown types.
    pub fn text(&self) -> Option<String> {
        match (&self.decoded, self.kind) {
            (Decoded::Signed(v), _) => Some(v.to_string()),
            (Decoded::Unsigned(v), _) => Some(v.to_string()),
            (Decoded::Oid(oid), _) => Some(oid.to_string()),
            (Decoded::IpAddress(ip), _) => Some(dotted_ip(ip)),
            (Decoded::Octets, ValueType::OctetString | ValueType::Opaque) => Some(hex(&self.raw)),
            (Decoded::Octets, _) => None,
        }
    }

    /// Convert to a point datatype (spec §6). `Err` carries the reason for a `bad` sample.
    pub fn convert(&self, datatype: DataType) -> Result<Value, String> {
        let number: i128 = match &self.decoded {
            Decoded::Signed(v) => i128::from(*v),
            Decoded::Unsigned(v) => i128::from(*v),
            Decoded::Oid(oid) if datatype == DataType::String => return Ok(Value::Text(oid.to_string())),
            Decoded::IpAddress(ip) if datatype == DataType::String => {
                return Ok(Value::Text(dotted_ip(ip)))
            }
            Decoded::Octets if self.kind == ValueType::OctetString && datatype == DataType::String => {
                return std::str::from_utf8(&self.raw)
                    .map(|s| Value::Text(s.to_string()))
                    .map_err(|_| {
                        "OCTET STRING is not valid UTF-8; read it with mode = \"raw\"".to_string()
                    });
            }
            _ => return Err(self.cannot(datatype)),
        };
        let range: (i128, i128) = match datatype {
            DataType::Bool => return Ok(Value::Bool(number != 0)),
            DataType::Float32 => return Ok(Value::Number(number as f64 as f32 as f64)),
            DataType::Float64 => return Ok(Value::Number(number as f64)),
            DataType::String => return Ok(Value::Text(number.to_string())),
            DataType::Int8 => (i8::MIN.into(), i8::MAX.into()),
            DataType::Uint8 => (0, u8::MAX.into()),
            DataType::Int16 => (i16::MIN.into(), i16::MAX.into()),
            DataType::Uint16 => (0, u16::MAX.into()),
            DataType::Int32 => (i32::MIN.into(), i32::MAX.into()),
            DataType::Uint32 => (0, u32::MAX.into()),
            DataType::Int64 => (i64::MIN.into(), i64::MAX.into()),
            DataType::Uint64 => (0, u64::MAX.into()),
            DataType::Bytes => return Err(self.cannot(datatype)),
        };
        if number < range.0 || number > range.1 {
            return Err(format!(
                "{} value {number} is out of range for {}",
                self.kind.name(),
                datatype_name(datatype)
            ));
        }
        const SAFE: i128 = (1 << 53) - 1;
        if number.abs() > SAFE {
            Ok(Value::Text(number.to_string()))
        } else {
            Ok(Value::Number(number as f64))
        }
    }

    fn cannot(&self, datatype: DataType) -> String {
        format!("cannot convert a {} value to {}", self.kind.name(), datatype_name(datatype))
    }
}

pub fn datatype_name(datatype: DataType) -> String {
    serde_json::to_value(datatype)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{datatype:?}"))
}

/// One variable binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Varbind {
    pub name: Oid,
    pub value: VarValue,
}

impl Varbind {
    pub fn from_library(name: &snmp2::Oid<'_>, value: &snmp2::Value<'_>) -> Result<Varbind, DecodeError> {
        Ok(Varbind { name: Oid::from_library(name)?, value: VarValue::from_library(value)? })
    }
}

/// Minimal two's-complement content octets of an INTEGER.
pub fn encode_integer(value: i64) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let mut start = 0;
    while start < 7 {
        let redundant = (bytes[start] == 0x00 && bytes[start + 1] & 0x80 == 0)
            || (bytes[start] == 0xFF && bytes[start + 1] & 0x80 != 0);
        if !redundant {
            break;
        }
        start += 1;
    }
    bytes[start..].to_vec()
}

/// Minimal content octets of a non-negative value, as DER encodes the unsigned SNMP types: a
/// leading zero when the top bit of the first octet is set.
pub fn encode_unsigned(value: u64) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let skip = bytes.iter().take_while(|&&b| b == 0).count().min(7);
    let mut out = Vec::with_capacity(9);
    if bytes[skip] & 0x80 != 0 {
        out.push(0);
    }
    out.extend_from_slice(&bytes[skip..]);
    out
}

pub fn dotted_ip(ip: &[u8; 4]) -> String {
    format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oid_parse_accepts_a_leading_dot_and_rejects_nonsense() {
        assert_eq!(Oid::parse(".1.3.6").unwrap(), Oid::parse("1.3.6").unwrap());
        for bad in ["", "1", "3.1", "1.40", "1..3", "1.3.x", "1.3.4294967296", "1.3.", "2.4294967216"] {
            assert!(Oid::parse(bad).is_err(), "{bad} should be rejected");
        }
        assert!(Oid::parse("2.999").is_ok());
        assert!(Oid::parse(&vec!["1"; 129].join(".")).is_err());
        assert!(Oid::parse(&vec!["1"; 128].join(".")).is_ok());
    }

    #[test]
    fn oid_subtree_matching_is_by_arc_not_by_text() {
        let prefix = Oid::parse("1.3.6.1.2.1.2.2.1.1").unwrap();
        assert!(Oid::parse("1.3.6.1.2.1.2.2.1.1.3").unwrap().starts_with(&prefix));
        assert!(Oid::parse("1.3.6.1.2.1.2.2.1.1").unwrap().starts_with(&prefix));
        assert!(!Oid::parse("1.3.6.1.2.1.2.2.1.10").unwrap().starts_with(&prefix));
    }

    #[test]
    fn arcs_are_split_correctly_where_the_library_does_not() {
        let big = Oid::parse("2.999.3").unwrap();
        assert_eq!(big.encode(), vec![0x88, 0x37, 0x03]);
        assert_eq!(Oid::from_library(&big.to_library()).unwrap(), big);
        assert_eq!(Oid::parse("1.3.6.1.2.1.1.3.0").unwrap().parent().unwrap().to_string(), "1.3.6.1.2.1.1.3");
        assert_eq!(Oid::parse("1.3").unwrap().parent(), None);
    }

    #[test]
    fn integer_encodings_are_minimal() {
        assert_eq!(encode_integer(0), vec![0]);
        assert_eq!(encode_integer(127), vec![0x7F]);
        assert_eq!(encode_integer(128), vec![0x00, 0x80]);
        assert_eq!(encode_integer(-1), vec![0xFF]);
        assert_eq!(encode_integer(-129), vec![0xFF, 0x7F]);
        assert_eq!(encode_integer(i64::MIN), i64::MIN.to_be_bytes().to_vec());
        assert_eq!(encode_unsigned(0), vec![0]);
        assert_eq!(encode_unsigned(0xFFFF_FFFF), vec![0, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(encode_unsigned(0x7F), vec![0x7F]);
        assert_eq!(encode_unsigned(u64::MAX), [vec![0], vec![0xFF; 8]].concat());
    }
}
