// SPDX-License-Identifier: MPL-2.0
// tedge-dot patch (see TEDGE-DOT-PATCH.md): trust lists with CA chains and CRLs.

//! A trust list read from a PKI directory laid out after OPC UA Part 12 (F.1):
//!
//! ```text
//! <pki>/trusted/certs   trusted certificates: pinned leaves and trust anchors (CAs)
//! <pki>/trusted/crl     CRLs of the CAs in trusted/certs
//! <pki>/issuers/certs   CA certificates that may complete a chain but are not trusted alone
//! <pki>/issuers/crl     CRLs of the CAs in issuers/certs
//! <pki>/rejected/certs  certificates that failed validation
//! ```
//!
//! Files are recognised by content (DER, or PEM holding one or more blocks), never by name or
//! extension. A file that does not parse is skipped with a warning.
//!
//! A certificate is trusted when it is itself in `trusted/certs` (pinned), or when it chains
//! through CAs from `trusted/certs` and `issuers/certs` to a CA in `trusted/certs`. For a
//! chained certificate every CA on the path must have a CRL (from either `crl` directory,
//! signed by that CA): no CRL means the revocation status is unknown, which is a failure, as
//! OPC UA Part 4 (6.1.3) requires by default.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rsa::signature::Verifier;
use rsa::RsaPublicKey;
use tracing::{debug, warn};
use x509_cert::crl::CertificateList;
use x509_cert::der::referenced::OwnedToRef;
use x509_cert::der::{Decode, Encode};
use x509_cert::ext::pkix::BasicConstraints;
use x509_cert::spki::AlgorithmIdentifierOwned;
use x509_cert::Certificate;

use opcua_types::status_code::StatusCode;

/// Longest CA path followed before giving up (intermediates plus anchor).
const MAX_CHAIN_DEPTH: usize = 8;

/// Trust-list subdirectories, relative to the PKI directory.
pub const TRUSTED_CERTS_DIR: &str = "trusted/certs";
/// CRLs for the CAs in [`TRUSTED_CERTS_DIR`].
pub const TRUSTED_CRL_DIR: &str = "trusted/crl";
/// CA certificates that can complete a chain but are not trusted by themselves.
pub const ISSUER_CERTS_DIR: &str = "issuers/certs";
/// CRLs for the CAs in [`ISSUER_CERTS_DIR`].
pub const ISSUER_CRL_DIR: &str = "issuers/crl";
/// Certificates that failed validation.
pub const REJECTED_CERTS_DIR: &str = "rejected/certs";

/// A certificate as read from disk, with its DER encoding.
#[derive(Clone)]
pub struct StoredCert {
    /// The file it was read from.
    pub path: PathBuf,
    /// The DER encoding.
    pub der: Vec<u8>,
    pub(crate) cert: Certificate,
}

impl StoredCert {
    /// Decode a DER certificate.
    pub fn from_der(path: PathBuf, der: Vec<u8>) -> Option<StoredCert> {
        let cert = Certificate::from_der(&der).ok()?;
        Some(StoredCert { path, der, cert })
    }

    fn is_ca(&self) -> bool {
        matches!(
            self.cert.tbs_certificate.get::<BasicConstraints>(),
            Ok(Some((_, BasicConstraints { ca: true, .. })))
        )
    }

    fn is_self_issued(&self) -> bool {
        same_name(&self.cert.tbs_certificate.subject, &self.cert.tbs_certificate.issuer)
    }

    fn public_key(&self) -> Option<RsaPublicKey> {
        RsaPublicKey::try_from(
            self.cert
                .tbs_certificate
                .subject_public_key_info
                .owned_to_ref(),
        )
        .ok()
    }

    fn time_valid(&self, now: &DateTime<Utc>) -> bool {
        let validity = &self.cert.tbs_certificate.validity;
        let secs = now.timestamp();
        let nb = validity.not_before.to_unix_duration().as_secs() as i64;
        let na = validity.not_after.to_unix_duration().as_secs() as i64;
        nb <= secs && secs <= na
    }
}

/// A CRL as read from disk.
#[derive(Clone)]
pub struct StoredCrl {
    /// The file it was read from.
    pub path: PathBuf,
    /// The DER encoding.
    pub der: Vec<u8>,
    pub(crate) crl: CertificateList,
}

impl StoredCrl {
    /// Decode a DER CRL.
    pub fn from_der(path: PathBuf, der: Vec<u8>) -> Option<StoredCrl> {
        let crl = CertificateList::from_der(&der).ok()?;
        Some(StoredCrl { path, der, crl })
    }
}

/// Everything a validation reads from the PKI directory.
#[derive(Clone, Default)]
pub struct TrustList {
    /// Pinned certificates and trust anchors.
    pub trusted: Vec<StoredCert>,
    /// CRLs next to the trusted certificates.
    pub trusted_crls: Vec<StoredCrl>,
    /// Issuer (intermediate) CAs.
    pub issuers: Vec<StoredCert>,
    /// CRLs next to the issuer certificates.
    pub issuer_crls: Vec<StoredCrl>,
    /// Rejected certificates.
    pub rejected: Vec<StoredCert>,
    /// One line per file that was skipped.
    pub warnings: Vec<String>,
}

/// Why a certificate failed [`TrustList::verify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustFailure {
    /// Listed in `rejected/certs`.
    Rejected,
    /// Neither pinned nor chained to a trusted CA.
    Untrusted,
    /// A certificate on the path is revoked (`issuer` when it is a CA, not the leaf).
    Revoked {
        /// The revoked certificate is a CA rather than the leaf.
        issuer: bool,
    },
    /// A CA on the path has no CRL.
    RevocationUnknown {
        /// The certificate whose revocation is unknown is a CA rather than the leaf.
        issuer: bool,
    },
    /// A CA on the path is outside its validity period.
    IssuerTimeInvalid,
}

impl TrustFailure {
    /// The OPC UA status code for this failure.
    pub fn status_code(self) -> StatusCode {
        match self {
            TrustFailure::Rejected | TrustFailure::Untrusted => StatusCode::BadCertificateUntrusted,
            TrustFailure::Revoked { issuer: false } => StatusCode::BadCertificateRevoked,
            TrustFailure::Revoked { issuer: true } => StatusCode::BadCertificateIssuerRevoked,
            TrustFailure::RevocationUnknown { issuer: false } => {
                StatusCode::BadCertificateRevocationUnknown
            }
            TrustFailure::RevocationUnknown { issuer: true } => {
                StatusCode::BadCertificateIssuerRevocationUnknown
            }
            TrustFailure::IssuerTimeInvalid => StatusCode::BadCertificateIssuerTimeInvalid,
        }
    }
}

impl TrustList {
    /// Read the trust list of the PKI directory `pki`. Missing directories are empty.
    pub fn load(pki: &Path) -> TrustList {
        let mut list = TrustList::default();
        list.trusted = read_certs(&pki.join(TRUSTED_CERTS_DIR), &mut list.warnings);
        list.trusted_crls = read_crls(&pki.join(TRUSTED_CRL_DIR), &mut list.warnings);
        list.issuers = read_certs(&pki.join(ISSUER_CERTS_DIR), &mut list.warnings);
        list.issuer_crls = read_crls(&pki.join(ISSUER_CRL_DIR), &mut list.warnings);
        list.rejected = read_certs(&pki.join(REJECTED_CERTS_DIR), &mut list.warnings);
        for w in &list.warnings {
            warn!("{w}");
        }
        list
    }

    /// Is this exact certificate listed as rejected?
    pub fn is_rejected(&self, der: &[u8]) -> bool {
        self.rejected.iter().any(|c| c.der == der)
    }

    /// Is this exact certificate pinned?
    pub fn is_pinned(&self, der: &[u8]) -> bool {
        self.trusted.iter().any(|c| c.der == der)
    }

    /// Decide whether the DER certificate `der` is trusted, including revocation and the
    /// validity of the CAs on its path. The leaf's own validity period, host name and
    /// application URI are the caller's to check.
    ///
    /// `rejected/certs` is a list for review (OPC UA Part 12), not a deny list: a certificate
    /// that is pinned or chains to a trusted CA is trusted even when a copy is also there,
    /// which is what happens when a server was first seen before its CA was trusted. An
    /// untrusted certificate that is listed there reports [`TrustFailure::Rejected`]. To stop
    /// trusting one certificate a CA issued, revoke it.
    pub fn verify(&self, der: &[u8], now: &DateTime<Utc>) -> Result<(), TrustFailure> {
        match self.verify_trust(der, now) {
            Err(TrustFailure::Untrusted) if self.is_rejected(der) => Err(TrustFailure::Rejected),
            other => other,
        }
    }

    fn verify_trust(&self, der: &[u8], now: &DateTime<Utc>) -> Result<(), TrustFailure> {
        if self.is_pinned(der) {
            return Ok(());
        }
        let leaf = StoredCert::from_der(PathBuf::new(), der.to_vec()).ok_or(TrustFailure::Untrusted)?;

        // Build the path first: an untrusted certificate is reported as untrusted, whatever
        // else is wrong with the CAs it names.
        let mut path: Vec<&StoredCert> = Vec::new();
        let mut current = &leaf;
        loop {
            if path.len() >= MAX_CHAIN_DEPTH {
                debug!("certificate path longer than {MAX_CHAIN_DEPTH}");
                return Err(TrustFailure::Untrusted);
            }
            let issuer = self
                .trusted
                .iter()
                .chain(self.issuers.iter())
                .find(|ca| {
                    ca.is_ca()
                        && !path.iter().any(|p| p.der == ca.der)
                        && same_name(&ca.cert.tbs_certificate.subject, &current.cert.tbs_certificate.issuer)
                        && ca
                            .public_key()
                            .is_some_and(|key| signature_valid(&key, &current.cert))
                });
            let Some(issuer) = issuer else {
                return Err(TrustFailure::Untrusted);
            };
            path.push(issuer);
            if self.trusted.iter().any(|t| t.der == issuer.der) {
                break;
            }
            if issuer.is_self_issued() {
                // A root that is only an issuer is not a trust anchor.
                return Err(TrustFailure::Untrusted);
            }
            current = issuer;
        }

        // Revocation of every certificate on the path by its issuer, and the CAs' validity.
        let mut subject = &leaf;
        for (depth, ca) in path.iter().enumerate() {
            let issuer_level = depth > 0;
            let crls: Vec<&StoredCrl> = self
                .trusted_crls
                .iter()
                .chain(self.issuer_crls.iter())
                .filter(|crl| crl_issued_by(crl, ca))
                .collect();
            if crls.is_empty() {
                return Err(TrustFailure::RevocationUnknown { issuer: issuer_level });
            }
            let serial = &subject.cert.tbs_certificate.serial_number;
            let revoked = crls.iter().any(|crl| {
                crl.crl
                    .tbs_cert_list
                    .revoked_certificates
                    .as_ref()
                    .is_some_and(|list| list.iter().any(|r| &r.serial_number == serial))
            });
            if revoked {
                return Err(TrustFailure::Revoked { issuer: issuer_level });
            }
            if !ca.time_valid(now) {
                return Err(TrustFailure::IssuerTimeInvalid);
            }
            subject = ca;
        }
        Ok(())
    }
}

/// Every certificate in `bytes`: one DER certificate, or all `CERTIFICATE` blocks of a PEM text.
pub fn parse_certificates(bytes: &[u8]) -> Vec<Vec<u8>> {
    if let Some(blocks) = pem_blocks(bytes, "CERTIFICATE") {
        return blocks
            .into_iter()
            .filter(|der| Certificate::from_der(der).is_ok())
            .collect();
    }
    match Certificate::from_der(bytes) {
        Ok(_) => vec![bytes.to_vec()],
        Err(_) => Vec::new(),
    }
}

/// Every CRL in `bytes`: one DER CRL, or all `X509 CRL` blocks of a PEM text.
pub fn parse_crls(bytes: &[u8]) -> Vec<Vec<u8>> {
    if let Some(blocks) = pem_blocks(bytes, "X509 CRL") {
        return blocks
            .into_iter()
            .filter(|der| CertificateList::from_der(der).is_ok())
            .collect();
    }
    match CertificateList::from_der(bytes) {
        Ok(_) => vec![bytes.to_vec()],
        Err(_) => Vec::new(),
    }
}

/// The DER of every PEM block labelled `label`, or `None` when `bytes` is not PEM text.
fn pem_blocks(bytes: &[u8], label: &str) -> Option<Vec<Vec<u8>>> {
    let text = std::str::from_utf8(bytes).ok()?;
    if !text.trim_start().starts_with("-----BEGIN") {
        return None;
    }
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(&begin) {
        let Some(stop) = rest[start..].find(&end) else {
            break;
        };
        let block = &rest[start..start + stop + end.len()];
        if let Ok((found, der)) = x509_cert::der::pem::decode_vec(block.as_bytes()) {
            if found == label {
                out.push(der);
            }
        }
        rest = &rest[start + stop + end.len()..];
    }
    Some(out)
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .collect();
    files.sort();
    files
}

fn read_certs(dir: &Path, warnings: &mut Vec<String>) -> Vec<StoredCert> {
    let mut out = Vec::new();
    for path in files(dir) {
        let Ok(bytes) = std::fs::read(&path) else {
            warnings.push(format!("cannot read {}", path.display()));
            continue;
        };
        let ders = parse_certificates(&bytes);
        if ders.is_empty() {
            warnings.push(format!("{} is not a certificate; skipped", path.display()));
        }
        out.extend(ders.into_iter().filter_map(|der| StoredCert::from_der(path.clone(), der)));
    }
    out
}

fn read_crls(dir: &Path, warnings: &mut Vec<String>) -> Vec<StoredCrl> {
    let mut out = Vec::new();
    for path in files(dir) {
        let Ok(bytes) = std::fs::read(&path) else {
            warnings.push(format!("cannot read {}", path.display()));
            continue;
        };
        let ders = parse_crls(&bytes);
        if ders.is_empty() {
            warnings.push(format!("{} is not a CRL; skipped", path.display()));
        }
        out.extend(ders.into_iter().filter_map(|der| StoredCrl::from_der(path.clone(), der)));
    }
    out
}

/// Whether the DER CRL `crl` was issued (named and signed) by the DER CA certificate `ca`.
pub fn crl_is_issued_by(crl: &[u8], ca: &[u8]) -> bool {
    let (Ok(crl), Some(ca)) = (
        CertificateList::from_der(crl),
        StoredCert::from_der(PathBuf::new(), ca.to_vec()),
    ) else {
        return false;
    };
    crl_issued_by(
        &StoredCrl { path: PathBuf::new(), der: Vec::new(), crl },
        &ca,
    )
}

fn same_name(a: &x509_cert::name::Name, b: &x509_cert::name::Name) -> bool {
    match (a.to_der(), b.to_der()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn crl_issued_by(crl: &StoredCrl, ca: &StoredCert) -> bool {
    if !same_name(&crl.crl.tbs_cert_list.issuer, &ca.cert.tbs_certificate.subject) {
        return false;
    }
    let (Some(key), Ok(tbs)) = (ca.public_key(), crl.crl.tbs_cert_list.to_der()) else {
        return false;
    };
    verify(&key, &crl.crl.signature_algorithm, &tbs, crl.crl.signature.raw_bytes())
}

fn signature_valid(key: &RsaPublicKey, cert: &Certificate) -> bool {
    let Ok(tbs) = cert.tbs_certificate.to_der() else {
        return false;
    };
    verify(key, &cert.signature_algorithm, &tbs, cert.signature.raw_bytes())
}

/// Verify an RSA signature (PKCS#1 v1.5 with SHA-1/SHA-256/SHA-384/SHA-512, or RSASSA-PSS
/// with SHA-256/SHA-384/SHA-512).
fn verify(key: &RsaPublicKey, alg: &AlgorithmIdentifierOwned, msg: &[u8], sig: &[u8]) -> bool {
    use const_oid::db::rfc5912::{
        ID_RSASSA_PSS, ID_SHA_256, ID_SHA_384, ID_SHA_512, SHA_1_WITH_RSA_ENCRYPTION,
        SHA_256_WITH_RSA_ENCRYPTION, SHA_384_WITH_RSA_ENCRYPTION, SHA_512_WITH_RSA_ENCRYPTION,
    };
    use rsa::{pkcs1v15, pss};

    fn check<V: Verifier<S>, S>(verifier: V, msg: &[u8], sig: S) -> bool {
        verifier.verify(msg, &sig).is_ok()
    }
    let key = key.clone();
    match alg.oid {
        SHA_256_WITH_RSA_ENCRYPTION => pkcs1v15::Signature::try_from(sig)
            .is_ok_and(|s| check(pkcs1v15::VerifyingKey::<sha2::Sha256>::new(key), msg, s)),
        SHA_384_WITH_RSA_ENCRYPTION => pkcs1v15::Signature::try_from(sig)
            .is_ok_and(|s| check(pkcs1v15::VerifyingKey::<sha2::Sha384>::new(key), msg, s)),
        SHA_512_WITH_RSA_ENCRYPTION => pkcs1v15::Signature::try_from(sig)
            .is_ok_and(|s| check(pkcs1v15::VerifyingKey::<sha2::Sha512>::new(key), msg, s)),
        SHA_1_WITH_RSA_ENCRYPTION => pkcs1v15::Signature::try_from(sig)
            .is_ok_and(|s| check(pkcs1v15::VerifyingKey::<sha1::Sha1>::new(key), msg, s)),
        ID_RSASSA_PSS => {
            let Some(params) = alg.parameters.as_ref().and_then(|p| p.to_der().ok()) else {
                return false;
            };
            let Ok(params) = rsa::pkcs1::RsaPssParams::try_from(params.as_slice()) else {
                return false;
            };
            let salt = params.salt_len as usize;
            let Ok(s) = pss::Signature::try_from(sig) else {
                return false;
            };
            match params.hash.oid {
                ID_SHA_256 => check(pss::VerifyingKey::<sha2::Sha256>::new_with_salt_len(key, salt), msg, s),
                ID_SHA_384 => check(pss::VerifyingKey::<sha2::Sha384>::new_with_salt_len(key, salt), msg, s),
                ID_SHA_512 => check(pss::VerifyingKey::<sha2::Sha512>::new_with_salt_len(key, salt), msg, s),
                _ => false,
            }
        }
        _ => false,
    }
}
