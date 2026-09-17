//! Arbitrary bytes as a file in the PKI directory: every certificate and CRL the loaders find in
//! it (DER, or any number of PEM blocks) goes through the accessors the connector and
//! `tedge-dot pki` use, and through trust validation against a trust list made of the same
//! input. Nothing may panic.
//!
//! Seed it with real files: `genpki.py /tmp/v` and copy `/tmp/v/scenarios/*/pki/**` and
//! `/tmp/v/scenarios/*/server/*` into `corpus/pki_files/`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use opcua::crypto::trust_list::{
    crl_is_issued_by, parse_certificates, parse_crls, StoredCert, StoredCrl, TrustList,
};
use opcua::crypto::X509;
use std::path::PathBuf;

fuzz_target!(|data: &[u8]| {
    let certs = parse_certificates(data);
    let crls = parse_crls(data);

    for der in &certs {
        if let Ok(cert) = X509::from_der(der) {
            let _ = cert.subject_name();
            let _ = cert.issuer_name();
            let _ = cert.common_name();
            let _ = cert.alternate_names();
            let _ = cert.is_ca();
            let _ = cert.not_before();
            let _ = cert.not_after();
            let _ = cert.key_length();
            let _ = cert.thumbprint();
            let _ = cert.is_hostname_valid("localhost");
            let _ = cert.is_application_uri_valid("urn:x");
            let _ = cert.is_time_valid(&chrono::Utc::now());
        }
        for crl in &crls {
            let _ = crl_is_issued_by(crl, der);
        }
    }

    let stored: Vec<StoredCert> = certs
        .iter()
        .filter_map(|d| StoredCert::from_der(PathBuf::new(), d.clone()))
        .collect();
    let stored_crls: Vec<StoredCrl> = crls
        .iter()
        .filter_map(|d| StoredCrl::from_der(PathBuf::new(), d.clone()))
        .collect();
    // The input is both the trust list and what is validated against it: self-issued chains,
    // loops and CRLs naming their own issuer all reach the path builder.
    let list = TrustList {
        trusted: stored.iter().take(1).cloned().collect(),
        trusted_crls: stored_crls.clone(),
        issuers: stored.iter().skip(1).cloned().collect(),
        issuer_crls: stored_crls,
        ..Default::default()
    };
    let now = chrono::Utc::now();
    for der in &certs {
        let _ = list.verify(der, &now);
    }
    let _ = list.verify(data, &now);
});
