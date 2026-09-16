use core::fmt;
use std::{mem, ptr};

use crate::{Error, Oid, Result, USIZE_LEN, snmp};

pub const PRIMITIVE: u8 = 0b0000_0000;
pub const CONSTRUCTED: u8 = 0b0010_0000;

pub const CLASS_UNIVERSAL: u8 = 0b0000_0000;
pub const CLASS_APPLICATION: u8 = 0b0100_0000;
pub const CLASS_CONTEXTSPECIFIC: u8 = 0b1000_0000;
#[allow(dead_code)]
pub const CLASS_PRIVATE: u8 = 0b1100_0000;

pub const TYPE_BOOLEAN: u8 = CLASS_UNIVERSAL | PRIMITIVE | 1;
pub const TYPE_INTEGER: u8 = CLASS_UNIVERSAL | PRIMITIVE | 2;
pub const TYPE_OCTETSTRING: u8 = CLASS_UNIVERSAL | PRIMITIVE | 4;
pub const TYPE_NULL: u8 = CLASS_UNIVERSAL | PRIMITIVE | 5;
pub const TYPE_OBJECTIDENTIFIER: u8 = CLASS_UNIVERSAL | PRIMITIVE | 6;
pub const TYPE_SEQUENCE: u8 = CLASS_UNIVERSAL | CONSTRUCTED | 16;
pub const TYPE_SET: u8 = CLASS_UNIVERSAL | CONSTRUCTED | 17;

/// ASN.1/DER decoder iterator.
///
/// Supports:
///
/// - types required by SNMP.
///
/// Does not support:
///
/// - extended tag IDs.
/// - indefinite lengths (disallowed by DER).
/// - INTEGER values not representable by i64.
pub struct AsnReader<'a> {
    inner: &'a [u8],
}

impl<'a> Clone for AsnReader<'a> {
    fn clone(&self) -> AsnReader<'a> {
        AsnReader { inner: self.inner }
    }
}

impl fmt::Debug for AsnReader<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

impl<'a> AsnReader<'a> {
    pub fn from_bytes(bytes: &[u8]) -> AsnReader<'_> {
        AsnReader { inner: bytes }
    }

    pub fn peek_byte(&mut self) -> Result<u8> {
        if self.inner.is_empty() {
            Err(Error::AsnEof)
        } else {
            Ok(self.inner[0])
        }
    }

    pub fn read_byte(&mut self) -> Result<u8> {
        match self.inner.split_first() {
            Some((head, tail)) => {
                self.inner = tail;
                Ok(*head)
            }
            _ => Err(Error::AsnEof),
        }
    }

    pub fn read_length(&mut self) -> Result<usize> {
        if let Some((head, tail)) = self.inner.split_first() {
            let o: usize;
            if *head < 128 {
                // short form
                o = *head as usize;
                self.inner = tail;
                Ok(o)
            } else if head == &0xff {
                Err(Error::AsnInvalidLen) // reserved for future use
            } else {
                // long form
                let length_len = (*head & 0b0111_1111) as usize;
                if length_len == 0 {
                    // Indefinite length. Not allowed in DER.
                    return Err(Error::AsnInvalidLen);
                }

                let mut bytes = [0u8; USIZE_LEN];
                if length_len > USIZE_LEN {
                    return Err(Error::AsnInvalidLen);
                }
                if tail.len() < length_len {
                    return Err(Error::AsnEof);
                }
                bytes[(USIZE_LEN - length_len)..].copy_from_slice(&tail[..length_len]);

                o = usize::from_be_bytes(bytes);
                self.inner = &tail[length_len..];
                Ok(o)
            }
        } else {
            Err(Error::AsnEof)
        }
    }

    pub fn read_i64_type(&mut self, expected_ident: u8) -> Result<i64> {
        let ident = self.read_byte()?;
        if ident != expected_ident {
            return Err(Error::AsnWrongType);
        }
        let val_len = self.read_length()?;
        if val_len > self.inner.len() {
            return Err(Error::AsnInvalidLen);
        }
        let (val, remaining) = self.inner.split_at(val_len);
        self.inner = remaining;
        decode_i64(val)
    }

    pub fn read_raw(&mut self, expected_ident: u8) -> Result<&'a [u8]> {
        let ident = self.read_byte()?;
        if ident != expected_ident {
            return Err(Error::AsnWrongType);
        }
        let val_len = self.read_length()?;
        if val_len > self.inner.len() {
            return Err(Error::AsnInvalidLen);
        }
        //dbg!(self.inner.len());
        let (val, remaining) = self.inner.split_at(val_len);
        self.inner = remaining;
        Ok(val)
    }

    pub fn read_constructed<F>(&mut self, expected_ident: u8, f: F) -> Result<()>
    where
        F: Fn(&mut AsnReader) -> Result<()>,
    {
        let ident = self.read_byte()?;
        if ident != expected_ident {
            return Err(Error::AsnWrongType);
        }
        let seq_len = self.read_length()?;
        if seq_len > self.inner.len() {
            return Err(Error::AsnInvalidLen);
        }
        let (seq_bytes, remaining) = self.inner.split_at(seq_len);
        let mut reader = AsnReader::from_bytes(seq_bytes);
        self.inner = remaining;
        f(&mut reader)
    }

    //
    // ASN
    //

    pub fn read_asn_boolean(&mut self) -> Result<bool> {
        let ident = self.read_byte()?;
        // TEDGE-DOT-PATCH(1): upstream compared against TYPE_NULL, so no BOOLEAN ever decoded.
        if ident != TYPE_BOOLEAN {
            return Err(Error::AsnWrongType);
        }
        let val_len = self.read_length()?;
        if val_len != 1 {
            return Err(Error::AsnInvalidLen);
        }
        match self.read_byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::AsnParse), // DER mandates 1/0 for booleans
        }
    }

    pub fn read_asn_integer(&mut self) -> Result<i64> {
        self.read_i64_type(TYPE_INTEGER)
    }

    pub fn read_asn_octetstring(&mut self) -> Result<&'a [u8]> {
        self.read_raw(TYPE_OCTETSTRING)
    }

    pub fn read_asn_null(&mut self) -> Result<()> {
        let ident = self.read_byte()?;
        if ident != TYPE_NULL {
            return Err(Error::AsnWrongType);
        }
        let null_len = self.read_length()?;
        if null_len == 0 {
            Ok(())
        } else {
            Err(Error::AsnInvalidLen)
        }
    }

    pub fn read_asn_objectidentifier(&mut self) -> Result<Oid<'a>> {
        let ident = self.read_byte()?;
        if ident != TYPE_OBJECTIDENTIFIER {
            return Err(Error::AsnWrongType);
        }
        let val_len = self.read_length()?;
        if val_len > self.inner.len() {
            return Err(Error::AsnInvalidLen);
        }
        let (input, remaining) = self.inner.split_at(val_len);
        self.inner = remaining;
        // TEDGE-DOT-PATCH(2): upstream stored any octets as an OID.
        check_oid(input)?;

        Ok(Oid::new(input.into()))
    }

    pub fn read_asn_sequence<F>(&mut self, f: F) -> Result<()>
    where
        F: Fn(&mut AsnReader) -> Result<()>,
    {
        self.read_constructed(TYPE_SEQUENCE, f)
    }

    // TEDGE-DOT-PATCH(2): upstream read these as i64 and cast, so a Counter32 above 2^32-1
    // was silently narrowed and a Counter64 of 2^63 or more (nine content octets) failed.
    #[allow(clippy::cast_possible_truncation)]
    pub fn read_snmp_counter32(&mut self) -> Result<u32> {
        self.read_unsigned_type(snmp::TYPE_COUNTER32, u64::from(u32::MAX))
            .map(|v| v as u32)
    }

    #[allow(clippy::cast_possible_truncation)]
    pub fn read_snmp_unsigned32(&mut self) -> Result<u32> {
        self.read_unsigned_type(snmp::TYPE_UNSIGNED32, u64::from(u32::MAX))
            .map(|v| v as u32)
    }

    #[allow(clippy::cast_possible_truncation)]
    pub fn read_snmp_timeticks(&mut self) -> Result<u32> {
        self.read_unsigned_type(snmp::TYPE_TIMETICKS, u64::from(u32::MAX))
            .map(|v| v as u32)
    }

    pub fn read_snmp_counter64(&mut self) -> Result<u64> {
        self.read_unsigned_type(snmp::TYPE_COUNTER64, u64::MAX)
    }

    fn read_unsigned_type(&mut self, expected_ident: u8, max: u64) -> Result<u64> {
        let val = self.read_raw(expected_ident)?;
        decode_unsigned(val, max)
    }

    pub fn read_snmp_opaque(&mut self) -> Result<&'a [u8]> {
        self.read_raw(snmp::TYPE_OPAQUE)
    }

    pub fn read_snmp_ipaddress(&mut self) -> Result<[u8; 4]> {
        let val = self.read_raw(snmp::TYPE_IPADDRESS)?;
        if val.len() != 4 {
            return Err(Error::AsnInvalidLen);
        }
        unsafe { Ok(ptr::read(val.as_ptr().cast())) }
    }

    pub fn bytes_left(&self) -> usize {
        self.inner.len()
    }

    /// The octets not read yet (for a constructed value: its content octets).
    // TEDGE-DOT-PATCH(1)
    pub fn remaining(&self) -> &'a [u8] {
        self.inner
    }
}

/// Content octets of an unsigned value (Counter32, Gauge32, TimeTicks, Counter64): big-endian,
/// at most one octet more than the value needs (a leading zero), and not above `max`.
// TEDGE-DOT-PATCH(2)
fn decode_unsigned(i: &[u8], max: u64) -> Result<u64> {
    let max_len = (64 - max.leading_zeros() as usize).div_ceil(8) + 1;
    if i.is_empty() {
        return Err(Error::AsnInvalidLen);
    }
    if i.len() > max_len {
        return Err(Error::AsnIntOverflow);
    }
    let value = i
        .iter()
        .fold(0u128, |acc, &b| (acc << 8) | u128::from(b));
    u64::try_from(value)
        .ok()
        .filter(|v| *v <= max)
        .ok_or(Error::AsnIntOverflow)
}

/// An OBJECT IDENTIFIER's content octets: at least one sub-identifier, none left unfinished
/// (a final octet with the continuation bit), and every sub-identifier within 32 bits.
// TEDGE-DOT-PATCH(2)
pub(crate) fn check_oid(content: &[u8]) -> Result<()> {
    if content.last().is_none_or(|last| last & 0x80 != 0) {
        return Err(Error::AsnParse);
    }
    let mut subid: u64 = 0;
    for &b in content {
        subid = (subid << 7) | u64::from(b & 0x7F);
        if subid > u64::from(u32::MAX) {
            return Err(Error::AsnIntOverflow);
        }
        if b & 0x80 == 0 {
            subid = 0;
        }
    }
    Ok(())
}

fn decode_i64(i: &[u8]) -> Result<i64> {
    // TEDGE-DOT-PATCH(2): an INTEGER has at least one content octet; upstream decoded none as
    // 0 through a shift by 64 (a panic in debug builds).
    if i.is_empty() {
        return Err(Error::AsnInvalidLen);
    }
    if i.len() > mem::size_of::<i64>() {
        return Err(Error::AsnIntOverflow);
    }
    let mut bytes = [0u8; 8];
    bytes[(mem::size_of::<i64>() - i.len())..].copy_from_slice(i);

    let mut ret = i64::from_be_bytes(bytes);
    {
        //sign extend
        let shift_amount = (mem::size_of::<i64>() - i.len()) * 8;
        ret = (ret << shift_amount) >> shift_amount;
    }
    Ok(ret)
}
