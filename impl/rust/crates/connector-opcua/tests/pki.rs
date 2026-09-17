//! The PKI directory and server-certificate trust (doc/connectors/opcua-connector-spec.md §4),
//! driven by the vectors shared with the C build (genpki.py).

mod common;

use std::path::Path;

use connector_opcua::pki::{CertificateRequest, FindError, Group, Pki};
use connector_opcua::{security, OpcuaConnection};
use opcua::crypto::{CertificateStore, SecurityPolicy, X509};

/// Validate a scenario's server certificate the way a secured session does.
fn validate(scenario: &Path, expected: &serde_json::Value) -> Result<(), opcua::types::StatusCode> {
    let store = CertificateStore::new(&scenario.join("pki"));
    let der = std::fs::read(scenario.join("server/cert.der")).unwrap();
    let cert = X509::from_der(&der).unwrap();
    store.validate_or_reject_application_instance_cert(
        &cert,
        SecurityPolicy::Basic256Sha256,
        Some(expected["hostnames"][0].as_str().unwrap()),
        Some(expected["application_uri"].as_str().unwrap()),
    )
}

#[test]
fn vectors_match_expected_outcomes() {
    let vectors = common::genpki(&[]);
    let expected = common::expected(&vectors);
    let scenarios = expected["scenarios"].as_object().unwrap();
    assert!(
        scenarios.len() >= 16,
        "too few scenarios: {}",
        scenarios.len()
    );
    let mut failures = Vec::new();
    for (name, want) in scenarios {
        let outcome = want["outcome"].as_str().unwrap();
        let got = validate(&vectors.join("scenarios").join(name), &expected);
        let got_category = match got {
            Ok(()) => "trusted".to_string(),
            Err(status) => security::category(status)
                .map(|c| c.to_string())
                .unwrap_or_else(|| format!("uncategorised {status}")),
        };
        let want_category = match outcome {
            "trusted" => "trusted".to_string(),
            other => expected["reason_prefix"][other]
                .as_str()
                .unwrap()
                .to_string(),
        };
        if got_category != want_category {
            failures.push(format!(
                "{name}: want {want_category}, got {got_category} ({got:?})"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn untrusted_certificate_is_quarantined_then_trusted() {
    let vectors = common::genpki(&[]);
    let expected = common::expected(&vectors);
    let scenario = vectors.join("scenarios/untrusted");
    let pki = Pki::new(scenario.join("pki"));
    let thumbprint = expected["scenarios"]["untrusted"]["thumbprint"]
        .as_str()
        .unwrap();

    assert!(validate(&scenario, &expected).is_err());
    let rejected = pki.list(Group::Rejected);
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0].thumbprint, thumbprint);
    // The canonical name, as genpki.py (and the C build) spell it.
    assert_eq!(
        rejected[0].path.file_name().unwrap().to_string_lossy(),
        format!("sim_unknown_{thumbprint}.der")
    );
    assert_eq!(rejected[0].subject, "CN=sim unknown, O=tedge-dot test");

    // Rejecting twice stores it once.
    assert!(validate(&scenario, &expected).is_err());
    assert_eq!(pki.list(Group::Rejected).len(), 1);

    // `tedge-dot pki trust <prefix>`: the next validation succeeds, nothing restarted.
    let entry = pki
        .find(&thumbprint[..8].to_ascii_uppercase(), &[Group::Rejected])
        .unwrap();
    pki.relocate(&entry, Group::Trusted).unwrap();
    assert!(pki.list(Group::Rejected).is_empty());
    assert_eq!(validate(&scenario, &expected), Ok(()));

    // `tedge-dot pki reject`: untrusted again.
    let entry = pki.find(thumbprint, &[Group::Trusted]).unwrap();
    pki.relocate(&entry, Group::Rejected).unwrap();
    assert_eq!(
        security::category(validate(&scenario, &expected).unwrap_err()),
        Some(security::CERTIFICATE_UNTRUSTED)
    );
}

#[test]
fn trust_any_accepts_without_writing() {
    let vectors = common::genpki(&[]);
    let expected = common::expected(&vectors);
    let scenario = vectors.join("scenarios/untrusted");
    let mut store = CertificateStore::new(&scenario.join("pki"));
    store.set_trust_unknown_certs(true);
    let cert = X509::from_der(&std::fs::read(scenario.join("server/cert.der")).unwrap()).unwrap();
    store
        .validate_or_reject_application_instance_cert(
            &cert,
            SecurityPolicy::Basic256Sha256,
            Some(expected["hostnames"][0].as_str().unwrap()),
            None,
        )
        .unwrap();
    let pki = Pki::new(scenario.join("pki"));
    assert!(pki.list(Group::Trusted).is_empty());
    assert!(pki.list(Group::Rejected).is_empty());
}

#[test]
fn listing_skips_corrupt_files_and_reports_crls() {
    let vectors = common::genpki(&[]);
    let pki = Pki::new(vectors.join("scenarios/corrupt_file/pki"));
    let trusted = pki.list(Group::Trusted);
    let files: Vec<_> = trusted.iter().map(|e| e.path.display().to_string()).collect();
    assert_eq!(trusted.len(), 1, "{files:?}");
    assert!(!trusted[0].is_ca);

    let pki = Pki::new(vectors.join("scenarios/intermediate/pki"));
    let trusted = pki.list(Group::Trusted);
    assert_eq!((trusted[0].is_ca, trusted[0].has_crl), (true, Some(true)));
    let issuers = pki.list(Group::Issuers);
    assert_eq!((issuers[0].is_ca, issuers[0].has_crl), (true, Some(true)));

    let pki = Pki::new(vectors.join("scenarios/ca_no_crl/pki"));
    assert_eq!(pki.list(Group::Trusted)[0].has_crl, Some(false));
}

#[test]
fn find_rejects_short_and_ambiguous_prefixes() {
    let vectors = common::genpki(&[]);
    let pki = Pki::new(vectors.join("scenarios/rejected_listed/pki"));
    let entry = &pki.list(Group::Trusted)[0];
    assert!(matches!(
        pki.find(&entry.thumbprint[..7], &Group::ALL),
        Err(FindError::BadPrefix)
    ));
    assert!(matches!(
        pki.find("zzzzzzzz", &Group::ALL),
        Err(FindError::BadPrefix)
    ));
    assert!(matches!(
        pki.find("00000000", &Group::ALL),
        Err(FindError::NotFound)
    ));
    // The same certificate is both trusted and rejected: two matches across groups.
    match pki.find(&entry.thumbprint, &Group::ALL) {
        Err(FindError::Ambiguous(all)) => assert_eq!(all.len(), 2),
        other => panic!("{other:?}"),
    }
    assert!(pki.find(&entry.thumbprint, &[Group::Trusted]).is_ok());
}

#[test]
fn removing_one_certificate_of_a_bundle_keeps_the_others() {
    let vectors = common::genpki(&[]);
    let pem = |name: &str| {
        std::fs::read_to_string(vectors.join(format!("scenarios/{name}/server/cert.pem"))).unwrap()
    };
    let pki = Pki::new(vectors.join("bundle/pki"));
    pki.ensure_layout().unwrap();
    // Two certificates, one of them twice.
    let bundle = format!(
        "{}{}{}",
        pem("pinned"),
        pem("untrusted"),
        pem("pinned")
    );
    let file = pki.root().join("trusted/certs/site.pem");
    std::fs::write(&file, &bundle).unwrap();
    let entries = pki.list(Group::Trusted);
    assert_eq!(entries.len(), 3);
    let pinned = entries[0].clone();
    let other = entries.iter().find(|e| e.der != pinned.der).unwrap().clone();

    // Moving the duplicated certificate out leaves the other one in the file.
    pki.relocate(&pinned, Group::Rejected).unwrap();
    let left = pki.list(Group::Trusted);
    let files: Vec<_> = left.iter().map(|e| e.thumbprint.clone()).collect();
    assert_eq!(left.len(), 1, "{files:?}");
    assert_eq!(left[0].der, other.der);
    assert_eq!(left[0].path, file);
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(text, pem("untrusted"));
    assert_eq!(pki.list(Group::Rejected).len(), 1);

    // The last certificate takes the file with it.
    pki.remove(&left[0]).unwrap();
    assert!(!file.exists());
}

#[test]
fn add_crl_goes_next_to_its_ca() {
    let vectors = common::genpki(&[]);
    let pki = Pki::new(vectors.join("scenarios/intermediate_no_crl/pki"));
    // The intermediate's CRL, taken from the scenario that has one.
    let crl_dir = vectors.join("scenarios/intermediate/pki/issuers/crl");
    let crl_file = std::fs::read_dir(&crl_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let text = std::fs::read(crl_file).unwrap();
    let der = opcua::crypto::trust_list::parse_crls(&text).remove(0);
    let path = pki.add_crl(&der).unwrap();
    assert!(
        path.starts_with(vectors.join("scenarios/intermediate_no_crl/pki/issuers/crl")),
        "{path:?}"
    );

    // A CRL whose CA is unknown is refused.
    let empty = Pki::new(common::tempdir("pki-empty"));
    empty.ensure_layout().unwrap();
    assert!(empty.add_crl(&der).is_err());
}

fn connection(uri: &str) -> OpcuaConnection {
    OpcuaConnection {
        application_uri: uri.to_string(),
        ..Default::default()
    }
}

#[test]
fn certificate_is_generated_once_and_reused() {
    let dir = common::tempdir("pki-own");
    let pki = Pki::new(dir.join("pki"));
    let conn = connection("urn:tedge-dot:test");
    let first = pki.load_or_create_own(&conn, None).unwrap();
    assert!(first.generated);
    assert!(first
        .certificate
        .alternate_names()
        .contains(&"urn:tedge-dot:test".to_string()));
    assert_eq!(first.certificate.key_length().unwrap(), 2048);
    assert_eq!(first.certificate_path, dir.join("pki/own/certs/cert.der"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&first.private_key_path), 0o600);
        assert_eq!(mode(&dir.join("pki/own/private")), 0o700);
    }

    let again = pki.load_or_create_own(&conn, None).unwrap();
    assert!(!again.generated);
    assert_eq!(
        again.certificate.thumbprint().as_hex_string(),
        first.certificate.thumbprint().as_hex_string()
    );
}

#[test]
fn orphan_key_is_set_aside_and_a_certificate_without_key_refused() {
    let dir = common::tempdir("pki-orphan");
    let pki = Pki::new(dir.join("pki"));
    let conn = connection("urn:tedge-dot:test");
    let first = pki.load_or_create_own(&conn, None).unwrap();

    // Interrupted generation: the key was written, the certificate was not.
    std::fs::remove_file(&first.certificate_path).unwrap();
    let again = pki.load_or_create_own(&conn, None).unwrap();
    assert!(again.generated);
    assert_ne!(
        again.certificate.thumbprint().as_hex_string(),
        first.certificate.thumbprint().as_hex_string()
    );
    let aside: Vec<_> = std::fs::read_dir(dir.join("pki/own/private"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("key.pem."))
        .collect();
    assert_eq!(aside.len(), 1);

    // A certificate without its key is the operator's to fix.
    std::fs::remove_file(&again.private_key_path).unwrap();
    let Err(err) = pki.load_or_create_own(&conn, None) else {
        panic!("a certificate without its key was accepted");
    };
    assert!(err.contains("no private key"), "{err}");
}

#[test]
fn concurrent_first_start_generates_one_certificate() {
    let dir = common::tempdir("pki-race");
    let root = dir.join("pki");
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let root = root.clone();
            std::thread::spawn(move || {
                Pki::new(root)
                    .load_or_create_own(&connection("urn:tedge-dot"), None)
                    .unwrap()
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.generated).count(), 1);
    let thumbprints: std::collections::HashSet<String> = results
        .iter()
        .map(|r| r.certificate.thumbprint().as_hex_string())
        .collect();
    assert_eq!(thumbprints.len(), 1);
}

#[test]
fn uri_mismatch_is_reported() {
    let vectors = common::genpki(&[]);
    let dir = common::tempdir("pki-uri");
    let pki = Pki::new(dir.join("pki"));
    let conn = OpcuaConnection {
        application_uri: "urn:somebody-else".into(),
        certificate: Some(vectors.join("client/cert.der").display().to_string()),
        private_key: Some(vectors.join("client/key.pem").display().to_string()),
        ..Default::default()
    };
    let e = pki.load_or_create_own(&conn, None).err().expect("mismatch");
    assert!(
        e.contains("urn:tedge-dot") && e.contains("urn:somebody-else"),
        "{e}"
    );
    // Nothing was created for an explicit certificate.
    assert!(!dir.join("pki").exists());
}

#[test]
fn key_of_another_certificate_is_refused() {
    let vectors = common::genpki(&[]);
    let pki = Pki::new(common::tempdir("pki-key").join("pki"));
    let conn = OpcuaConnection {
        certificate: Some(vectors.join("client/cert.der").display().to_string()),
        private_key: Some(vectors.join("users/operator.key.pem").display().to_string()),
        ..Default::default()
    };
    let e = pki.load_or_create_own(&conn, None).err().expect("mismatch");
    assert!(e.contains("does not belong"), "{e}");
}

#[test]
fn create_refuses_to_overwrite_unless_forced() {
    let dir = common::tempdir("pki-create");
    let pki = Pki::new(dir.join("pki"));
    let request = CertificateRequest {
        application_name: "gw01".into(),
        application_uri: "urn:gw01".into(),
        hostnames: vec!["gw01.plant.local".into(), "10.1.2.3".into()],
        days: 30,
    };
    let first = pki.create(&request, false).unwrap();
    let names = first.certificate.alternate_names();
    assert_eq!(names, vec!["urn:gw01", "gw01.plant.local", "10.1.2.3"]);

    let e = pki.create(&request, false).err().expect("refused");
    assert!(e.contains("--force"), "{e}");
    let unchanged = pki
        .load_or_create_own(&connection("urn:gw01"), None)
        .unwrap();
    assert_eq!(
        unchanged.certificate.thumbprint().as_hex_string(),
        first.certificate.thumbprint().as_hex_string()
    );

    let second = pki.create(&request, true).unwrap();
    assert_ne!(
        second.certificate.thumbprint().as_hex_string(),
        first.certificate.thumbprint().as_hex_string()
    );
    let backups: Vec<_> = std::fs::read_dir(dir.join("pki/own/certs"))
        .unwrap()
        .chain(std::fs::read_dir(dir.join("pki/own/private")).unwrap())
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("cert.der.") || n.starts_with("key.pem."))
        .collect();
    assert_eq!(backups.len(), 2, "{backups:?}");
}
