//! `tedge-dot pki` against the built binary (doc/connectors/opcua-connector-spec.md §9), on
//! PKI trees from genpki.py. The C build runs the same command sequence
//! (impl/c/ci/pki-parity.sh) and must print the same JSON.
#![cfg(feature = "opcua")]

#[path = "../crates/connector-opcua/tests/common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn pki(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tedge-dot"))
        .arg("pki")
        .args(args)
        .output()
        .expect("run tedge-dot")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

fn json(out: &Output) -> Value {
    assert_eq!(
        code(out),
        0,
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&out.stdout)))
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

/// A copy of a genpki scenario's PKI directory.
fn scenario(vectors: &Path, name: &str) -> PathBuf {
    let dir = common::tempdir("cli").join("pki");
    copy_dir(&vectors.join("scenarios").join(name).join("pki"), &dir);
    dir
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn list_works_offline_and_reports_cas() {
    let vectors = common::genpki(&[]);
    let dir = scenario(&vectors, "intermediate");
    let out = json(&pki(&["list", "--pki-dir", s(&dir), "--json"]));
    assert_eq!(out["pki_dir"], s(&dir));
    let certs = out["certificates"].as_array().unwrap();
    assert_eq!(certs.len(), 2, "{out}");
    let root = certs.iter().find(|c| c["group"] == "trusted").unwrap();
    assert_eq!(root["ca"], true);
    assert_eq!(root["crl"], true);
    assert_eq!(
        root["subject"],
        "CN=tedge-dot test root CA, O=tedge-dot test"
    );
    assert!(root["not_after"].as_str().unwrap().ends_with('Z'));
    let issuer = certs.iter().find(|c| c["group"] == "issuers").unwrap();
    assert_eq!(issuer["ca"], true);

    // Human output flags a CA without a CRL.
    let dir = scenario(&vectors, "ca_no_crl");
    let out = pki(&["list", "trusted", "--pki-dir", s(&dir)]);
    assert_eq!(code(&out), 0);
    assert!(String::from_utf8_lossy(&out.stdout).contains("NO CRL"));
}

#[test]
fn trust_a_quarantined_certificate_by_prefix() {
    let vectors = common::genpki(&[]);
    let expected = common::expected(&vectors);
    let dir = scenario(&vectors, "untrusted");
    let thumbprint = expected["scenarios"]["untrusted"]["thumbprint"]
        .as_str()
        .unwrap();
    // Quarantine it the way a connector does.
    let cert = std::fs::read(vectors.join("scenarios/untrusted/server/cert.der")).unwrap();
    std::fs::write(dir.join("rejected/certs/whatever.der"), &cert).unwrap();

    let out = json(&pki(&[
        "trust",
        &thumbprint[..8].to_ascii_uppercase(),
        "--pki-dir",
        s(&dir),
        "--json",
    ]));
    assert_eq!(out["action"], "trust");
    assert_eq!(out["certificates"][0]["group"], "trusted");
    assert_eq!(out["certificates"][0]["thumbprint"], thumbprint);
    assert!(out["certificates"][0]["file"]
        .as_str()
        .unwrap()
        .ends_with(&format!("trusted/certs/sim_unknown_{thumbprint}.der")));
    assert!(std::fs::read_dir(dir.join("rejected/certs"))
        .unwrap()
        .next()
        .is_none());

    // Reject it again.
    let out = json(&pki(&[
        "reject",
        thumbprint,
        "--pki-dir",
        s(&dir),
        "--json",
    ]));
    assert_eq!(out["certificates"][0]["group"], "rejected");
    assert!(std::fs::read_dir(dir.join("trusted/certs"))
        .unwrap()
        .next()
        .is_none());
}

#[test]
fn unknown_and_malformed_thumbprints() {
    let vectors = common::genpki(&[]);
    let dir = scenario(&vectors, "pinned");
    let out = pki(&["trust", "0123abcd", "--pki-dir", s(&dir)]);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("0123abcd"), "{}", stderr(&out));
    let out = pki(&["reject", "0123", "--pki-dir", s(&dir)]);
    assert_eq!(code(&out), 1);
}

#[test]
fn rejecting_a_certificate_a_ca_still_vouches_for_warns() {
    let vectors = common::genpki(&[]);
    let dir = scenario(&vectors, "ca_issued");
    let leaf = vectors.join("scenarios/ca_issued/server/cert.der");
    let out = json(&pki(&["trust", s(&leaf), "--pki-dir", s(&dir), "--json"]));
    let thumbprint = out["certificates"][0]["thumbprint"].as_str().unwrap().to_string();
    let out = json(&pki(&["reject", &thumbprint, "--pki-dir", s(&dir), "--json"]));
    assert_eq!(out["certificates"][0]["group"], "rejected");
    assert!(out["warning"].as_str().unwrap().contains("CRL"), "{out}");
    let out = pki(&["reject", "0123abcd", "--pki-dir", s(&dir)]);
    assert_eq!(code(&out), 2);

    // A pinned certificate with no CA behind it: no warning.
    let dir = scenario(&vectors, "pinned");
    let thumbprint = common::expected(&vectors)["scenarios"]["pinned"]["thumbprint"]
        .as_str()
        .unwrap()
        .to_string();
    let out = json(&pki(&["reject", &thumbprint, "--pki-dir", s(&dir), "--json"]));
    assert!(out.get("warning").is_none(), "{out}");
}

#[test]
fn ambiguous_matches_are_listed() {
    let vectors = common::genpki(&[]);
    let dir = scenario(&vectors, "rejected_listed");
    let thumbprint = common::expected(&vectors)["scenarios"]["rejected_listed"]["thumbprint"]
        .as_str()
        .unwrap()
        .to_string();
    let out = pki(&["remove", &thumbprint[..10], "--pki-dir", s(&dir)]);
    assert_eq!(code(&out), 1);
    let err = stderr(&out);
    assert!(
        err.contains("trusted") && err.contains("rejected") && err.contains("--group"),
        "{err}"
    );

    let out = json(&pki(&[
        "remove",
        &thumbprint,
        "--group",
        "rejected",
        "--pki-dir",
        s(&dir),
        "--json",
    ]));
    assert_eq!(out["certificates"][0]["group"], "rejected");
    assert!(std::fs::read_dir(dir.join("rejected/certs"))
        .unwrap()
        .next()
        .is_none());
    assert!(std::fs::read_dir(dir.join("trusted/certs"))
        .unwrap()
        .next()
        .is_some());
}

#[test]
fn trust_imports_a_file() {
    let vectors = common::genpki(&[]);
    let dir = common::tempdir("cli-import").join("pki");
    let pem = vectors.join("scenarios/pinned/server/cert.pem");
    let out = json(&pki(&["trust", s(&pem), "--pki-dir", s(&dir), "--json"]));
    assert_eq!(
        out["certificates"][0]["subject"],
        "CN=sim pinned, O=tedge-dot test"
    );
    // Stored DER, whatever the input encoding.
    let stored = PathBuf::from(out["certificates"][0]["file"].as_str().unwrap());
    assert_eq!(
        std::fs::read(stored).unwrap(),
        std::fs::read(vectors.join("scenarios/pinned/server/cert.der")).unwrap()
    );
}

#[test]
fn add_issuer_and_add_crl() {
    let vectors = common::genpki(&[]);
    let dir = scenario(&vectors, "intermediate_no_crl");
    // Not a CA.
    let out = pki(&[
        "add-issuer",
        s(&vectors.join("client/cert.der")),
        "--pki-dir",
        s(&dir),
    ]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));

    // The intermediate's CRL goes to issuers/crl.
    let crl_dir = vectors.join("scenarios/intermediate/pki/issuers/crl");
    let crl = std::fs::read_dir(&crl_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let out = json(&pki(&["add-crl", s(&crl), "--pki-dir", s(&dir), "--json"]));
    assert_eq!(out["action"], "add-crl");
    assert!(
        out["crls"][0]["file"]
            .as_str()
            .unwrap()
            .contains("issuers/crl/"),
        "{out}"
    );

    // A CRL of an unknown CA is refused and nothing is written.
    let empty = common::tempdir("cli-empty").join("pki");
    let out = pki(&["add-crl", s(&crl), "--pki-dir", s(&empty)]);
    assert_eq!(code(&out), 1);
    assert!(
        !empty.join("issuers/crl").exists()
            || std::fs::read_dir(empty.join("issuers/crl"))
                .unwrap()
                .next()
                .is_none()
    );

    // add-issuer stores a CA.
    let out = json(&pki(&[
        "add-issuer",
        s(&vectors.join("ca/root.der")),
        "--pki-dir",
        s(&empty),
        "--json",
    ]));
    assert_eq!(out["certificates"][0]["group"], "issuers");
    assert_eq!(out["certificates"][0]["ca"], true);
}

#[test]
fn create_show_export() {
    let dir = common::tempdir("cli-own");
    let config = dir.join("opcua.toml");
    std::fs::write(
        &config,
        "[connector]\nprotocol = \"opcua\"\n\n[connection]\napplication_uri = \"urn:gw01\"\npki_dir = \"pki\"\n",
    )
    .unwrap();

    // Nothing yet.
    let out = pki(&["show", "--config", s(&config)]);
    assert_eq!(code(&out), 2, "{}", stderr(&out));

    let out = json(&pki(&[
        "create",
        "--hostname",
        "gw01.plant.local",
        "--hostname",
        "10.1.2.3",
        "--config",
        s(&config),
        "--json",
    ]));
    assert_eq!(out["created"], true);
    assert_eq!(out["application_uri"], "urn:gw01");
    assert_eq!(
        out["hostnames"],
        serde_json::json!(["gw01.plant.local", "10.1.2.3"])
    );
    assert_eq!(out["uri_matches"], true);
    // The relative pki_dir resolved against the configuration file.
    assert_eq!(out["certificate"], s(&dir.join("pki/own/certs/cert.der")));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join("pki/own/private/key.pem"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    let shown = json(&pki(&["show", "--config", s(&config), "--json"]));
    assert_eq!(shown["thumbprint"], out["thumbprint"]);
    assert_eq!(shown["configured_application_uri"], "urn:gw01");

    // Refuse to overwrite.
    let refused = pki(&["create", "--config", s(&config)]);
    assert_eq!(code(&refused), 1);
    assert!(stderr(&refused).contains("--force"));
    assert_eq!(
        json(&pki(&["show", "--config", s(&config), "--json"]))["thumbprint"],
        out["thumbprint"]
    );

    // Export: PEM to a file, never the key.
    let exported = dir.join("tedge-dot.pem");
    let res = json(&pki(&[
        "export",
        "--pem",
        "--output",
        s(&exported),
        "--config",
        s(&config),
        "--json",
    ]));
    assert_eq!(res["thumbprint"], out["thumbprint"]);
    let text = std::fs::read_to_string(&exported).unwrap();
    assert!(text.starts_with("-----BEGIN CERTIFICATE-----"));
    assert!(!text.contains("PRIVATE KEY"));
    // DER to stdout.
    let der = pki(&["export", "--config", s(&config)]);
    assert_eq!(code(&der), 0);
    assert_eq!(
        der.stdout,
        std::fs::read(dir.join("pki/own/certs/cert.der")).unwrap()
    );

    // Renew with --force keeps the old pair.
    let renewed = json(&pki(&[
        "create",
        "--force",
        "--config",
        s(&config),
        "--json",
    ]));
    assert_ne!(renewed["thumbprint"], out["thumbprint"]);
    let backups = std::fs::read_dir(dir.join("pki/own/certs"))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("cert.der.")
        })
        .count();
    assert_eq!(backups, 1);

    // A certificate for another URI is flagged.
    let other = dir.join("other.toml");
    std::fs::write(
        &other,
        "[connector]\nprotocol = \"opcua\"\n\n[connection]\napplication_uri = \"urn:other\"\npki_dir = \"pki\"\n",
    )
    .unwrap();
    assert_eq!(
        json(&pki(&["show", "--config", s(&other), "--json"]))["uri_matches"],
        false
    );
}

#[test]
fn usage_errors_exit_1() {
    assert_eq!(code(&pki(&["list", "bogus-group"])), 1);
    assert_eq!(code(&pki(&["frobnicate"])), 1);
    assert_eq!(code(&pki(&["--help"])), 0);
    // A configuration of another protocol.
    let dir = common::tempdir("cli-proto");
    let config = dir.join("modbus.toml");
    std::fs::write(&config, "[connector]\nprotocol = \"modbus\"\n").unwrap();
    assert_eq!(code(&pki(&["list", "--config", s(&config)])), 1);
}
