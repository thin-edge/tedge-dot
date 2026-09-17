//! `tedge-dot pki` (doc/connectors/opcua-connector-spec.md §9): inspect and manage the OPC UA
//! PKI directory offline. The binary parses arguments; everything else is here, so the
//! behaviour (texts, JSON fields, exit codes) is testable and shared with nothing but the C
//! build's `pki_cli.c`, which prints the same JSON.
//!
//! Exit codes: 0 success, 1 usage or input error, 2 the named certificate does not exist.

use std::io::Write;
use std::path::{Path, PathBuf};

use opcua::crypto::trust_list::{parse_certificates, parse_crls, TrustList};
use opcua::crypto::X509;
use serde_json::{json, Value};

use crate::config::OpcuaConnection;
use crate::pki::{self, CertEntry, CertificateRequest, FindError, Group, Pki};

/// The configuration read when neither `--pki-dir` nor `--config` is given.
pub const DEFAULT_CONFIG: &str = "/etc/tedge/plugins/ot/opcua.toml";

pub const EXIT_OK: u8 = 0;
pub const EXIT_ERROR: u8 = 1;
pub const EXIT_NOT_FOUND: u8 = 2;

/// Where the PKI directory and the connection settings come from.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub pki_dir: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub json: bool,
}

#[derive(Debug, Clone)]
pub enum Action {
    Show,
    Export {
        pem: bool,
        output: Option<PathBuf>,
    },
    Create {
        application_uri: Option<String>,
        hostnames: Vec<String>,
        days: Option<u32>,
        force: bool,
    },
    List {
        group: Option<Group>,
    },
    Trust {
        target: String,
    },
    Reject {
        thumbprint: String,
    },
    Remove {
        thumbprint: String,
        group: Option<Group>,
    },
    AddIssuer {
        file: PathBuf,
    },
    AddCrl {
        file: PathBuf,
    },
}

/// A failed action: its exit code and message.
struct Failure(u8, String);

/// What the action prints without `--json`.
enum Printed {
    Text(String),
    Bytes(Vec<u8>),
}

impl From<String> for Printed {
    fn from(text: String) -> Printed {
        Printed::Text(text)
    }
}

/// The settings an action works with.
struct Context {
    conn: OpcuaConnection,
    base_dir: Option<PathBuf>,
    pki: Pki,
}

fn error(msg: impl Into<String>) -> Failure {
    Failure(EXIT_ERROR, msg.into())
}

/// Run one action, writing the result to `out` and problems to `err`. Returns the exit code.
pub fn run(opts: &Options, action: &Action, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    match execute(opts, action) {
        Ok((value, printed)) => {
            let _ = match (opts.json, printed) {
                (true, _) => writeln!(out, "{value}"),
                (false, Printed::Text(text)) => write!(out, "{text}"),
                (false, Printed::Bytes(bytes)) => out.write_all(&bytes),
            };
            EXIT_OK
        }
        Err(Failure(code, msg)) => {
            let _ = writeln!(err, "error: {msg}");
            code
        }
    }
}

/// The connection settings and the PKI directory the options select.
fn context(opts: &Options) -> Result<Context, Failure> {
    let config = match &opts.config {
        Some(path) => Some(path.clone()),
        None => Some(PathBuf::from(DEFAULT_CONFIG)).filter(|p| p.exists()),
    };
    let (conn, base_dir) = match config {
        None => (OpcuaConnection::default(), None),
        Some(path) => {
            let cfg = tedge_dot_sdk::library::load(&path).map_err(error)?;
            if cfg.connector.protocol != "opcua" {
                return Err(error(format!(
                    "{} is a {} configuration, not an opcua one",
                    path.display(),
                    cfg.connector.protocol
                )));
            }
            let conn = OpcuaConnection::from_value(&cfg.connection).map_err(error)?;
            (conn, cfg.base_dir)
        }
    };
    let root = match &opts.pki_dir {
        Some(dir) => dir.clone(),
        None => conn.pki_dir(base_dir.as_deref()),
    };
    Ok(Context {
        conn,
        base_dir,
        pki: Pki::new(root),
    })
}

fn execute(opts: &Options, action: &Action) -> Result<(Value, Printed), Failure> {
    let ctx = context(opts)?;
    let (conn, pki) = (&ctx.conn, &ctx.pki);
    let result = match action {
        Action::Show => show(&ctx).map(text),
        Action::Export { pem, output } => export(&ctx, *pem, output.as_deref(), opts.json),
        Action::Create {
            application_uri,
            hostnames,
            days,
            force,
        } => {
            let request = CertificateRequest {
                application_name: conn.application_name.clone(),
                application_uri: application_uri
                    .clone()
                    .unwrap_or_else(|| conn.application_uri.clone()),
                hostnames: hostnames.clone(),
                days: days.unwrap_or(pki::DEFAULT_CERTIFICATE_DAYS),
            };
            create(conn, pki, &request, *force).map(text)
        }
        Action::List { group } => Ok(text(list(pki, *group))),
        Action::Trust { target } => trust(pki, target).map(text),
        Action::Reject { thumbprint } => find(pki, thumbprint, &[Group::Trusted])
            .and_then(|entry| {
                let der = entry.der.clone();
                let (mut value, mut message) = moved(pki, "reject", &[entry], Group::Rejected)?;
                // rejected/ does not override trust: say so when a CA still vouches for it.
                if TrustList::load(pki.root()).verify(&der, &chrono::Utc::now()).is_ok() {
                    let warning = "the certificate is still trusted through a CA in trusted/; \
                                   revoke it in that CA's CRL to stop trusting it";
                    message.push_str(&format!("warning: {warning}\n"));
                    value["warning"] = json!(warning);
                }
                Ok((value, message))
            })
            .map(text),
        Action::Remove { thumbprint, group } => {
            let groups: Vec<Group> = group
                .map(|g| vec![g])
                .unwrap_or_else(|| Group::ALL.to_vec());
            find(pki, thumbprint, &groups).and_then(|entry| {
                pki.remove(&entry).map_err(error)?;
                let message = format!("removed {} from {}\n", entry.thumbprint, entry.group.name());
                Ok((
                    json!({"action": "remove", "certificates": [cert_json(&entry)]}),
                    message.into(),
                ))
            })
        }
        Action::AddIssuer { file } => add_issuer(pki, file).map(text),
        Action::AddCrl { file } => add_crl(pki, file).map(text),
    };
    adopt_owner(pki.root());
    result
}

fn text((value, text): (Value, String)) -> (Value, Printed) {
    (value, Printed::Text(text))
}

fn find(pki: &Pki, prefix: &str, groups: &[Group]) -> Result<CertEntry, Failure> {
    let names: Vec<&str> = groups.iter().map(|g| g.name()).collect();
    match pki.find(prefix, groups) {
        Ok(entry) => Ok(entry),
        Err(FindError::BadPrefix) => Err(error(format!(
            "'{prefix}' is not a thumbprint (at least 8 hexadecimal digits)"
        ))),
        Err(FindError::NotFound) => Err(Failure(
            EXIT_NOT_FOUND,
            format!(
                "no certificate with thumbprint {prefix} in {}",
                names.join(", ")
            ),
        )),
        Err(FindError::Ambiguous(all)) => {
            let list: Vec<String> = all
                .iter()
                .map(|e| format!("  {} {} {}", e.group.name(), e.thumbprint, e.subject))
                .collect();
            Err(error(format!(
                "thumbprint {prefix} matches {} certificates (give more digits{}):\n{}",
                all.len(),
                if groups.len() > 1 { " or --group" } else { "" },
                list.join("\n")
            )))
        }
    }
}

fn own_paths(ctx: &Context) -> (PathBuf, PathBuf) {
    ctx.pki.own_paths(&ctx.conn, ctx.base_dir.as_deref())
}

fn read_certificate(path: &Path) -> Result<(X509, Vec<u8>), Failure> {
    let bytes = std::fs::read(path).map_err(|_| {
        Failure(
            EXIT_NOT_FOUND,
            format!("no application certificate at {}", path.display()),
        )
    })?;
    let der = parse_certificates(&bytes)
        .into_iter()
        .next()
        .ok_or_else(|| error(format!("{} is not a certificate", path.display())))?;
    let cert = X509::from_der(&der)
        .map_err(|_| error(format!("{} is not a certificate", path.display())))?;
    Ok((cert, der))
}

/// Split a certificate's alternative names into its application URI and its host names.
fn names(cert: &X509) -> (Option<String>, Vec<String>) {
    let mut uri = None;
    let mut hosts = Vec::new();
    for name in cert.alternate_names() {
        if name.contains(':') && name.parse::<std::net::IpAddr>().is_err() {
            uri.get_or_insert(name);
        } else {
            hosts.push(name);
        }
    }
    (uri, hosts)
}

fn describe_own(
    conn: &OpcuaConnection,
    cert: &X509,
    der: &[u8],
    cert_path: &Path,
    key_path: &Path,
) -> (Value, String) {
    let (uri, hosts) = names(cert);
    let uri_matches = cert
        .alternate_names()
        .iter()
        .any(|n| n == &conn.application_uri);
    let not_before = cert.not_before().ok().map(|t| pki::timestamp(&t));
    let not_after = cert.not_after().ok().map(|t| pki::timestamp(&t));
    let thumbprint = pki::thumbprint(der);
    let value = json!({
        "certificate": cert_path.display().to_string(),
        "private_key": key_path.display().to_string(),
        "subject": cert.subject_text(),
        "issuer": cert.issuer_text(),
        "thumbprint": thumbprint,
        "not_before": not_before,
        "not_after": not_after,
        "application_uri": uri,
        "hostnames": hosts,
        "configured_application_uri": conn.application_uri,
        "uri_matches": uri_matches,
    });
    let text = format!(
        "certificate:      {}\nprivate key:      {}\nsubject:          {}\nthumbprint:       {}\nvalid:            {} .. {}\napplication URI:  {}{}\nhost names:       {}\n",
        cert_path.display(),
        key_path.display(),
        cert.subject_text(),
        thumbprint,
        not_before.as_deref().unwrap_or("?"),
        not_after.as_deref().unwrap_or("?"),
        uri.as_deref().unwrap_or("(none)"),
        if uri_matches {
            String::new()
        } else {
            format!("  (does NOT match application_uri {})", conn.application_uri)
        },
        if hosts.is_empty() { "(none)".to_string() } else { hosts.join(", ") },
    );
    (value, text)
}

fn show(ctx: &Context) -> Result<(Value, String), Failure> {
    let (cert_path, key_path) = own_paths(ctx);
    let (cert, der) = read_certificate(&cert_path)?;
    Ok(describe_own(&ctx.conn, &cert, &der, &cert_path, &key_path))
}

fn export(
    ctx: &Context,
    pem: bool,
    output: Option<&Path>,
    json_output: bool,
) -> Result<(Value, Printed), Failure> {
    let (cert_path, _) = own_paths(ctx);
    let (_, der) = read_certificate(&cert_path)?;
    let pem_text = x509_cert::der::pem::encode_string(
        "CERTIFICATE",
        x509_cert::der::pem::LineEnding::LF,
        &der,
    )
    .map_err(|e| error(format!("cannot encode the certificate: {e}")))?;
    let thumbprint = pki::thumbprint(&der);
    let format = if pem { "pem" } else { "der" };
    Ok(match output {
        Some(path) => {
            let bytes = if pem {
                pem_text.as_bytes().to_vec()
            } else {
                der
            };
            pki::write_atomic(path, &bytes, 0o644).map_err(error)?;
            let value = json!({"thumbprint": thumbprint, "format": format, "file": path.display().to_string()});
            (
                value,
                format!("wrote {thumbprint} ({format}) to {}\n", path.display()).into(),
            )
        }
        None if json_output => (
            json!({"thumbprint": thumbprint, "format": "pem", "pem": pem_text}),
            String::new().into(),
        ),
        None if pem => (Value::Null, Printed::Text(pem_text)),
        // Raw DER, as `openssl x509 -outform der` prints it.
        None => (Value::Null, Printed::Bytes(der)),
    })
}

fn create(
    conn: &OpcuaConnection,
    pki: &Pki,
    request: &CertificateRequest,
    force: bool,
) -> Result<(Value, String), Failure> {
    let own = pki.create(request, force).map_err(error)?;
    let der = own.certificate.to_der().unwrap_or_default();
    let shown = OpcuaConnection {
        application_uri: request.application_uri.clone(),
        ..conn.clone()
    };
    let (mut value, mut text) = describe_own(
        &shown,
        &own.certificate,
        &der,
        &own.certificate_path,
        &own.private_key_path,
    );
    value["created"] = true.into();
    text.insert_str(0, "created the application certificate\n");
    if conn.certificate.is_some() {
        text.push_str("note: the configuration names its own certificate/private_key, so connectors do not use this one\n");
    }
    if force {
        text.push_str("reload (SIGHUP) or restart the connector to use the new certificate\n");
    }
    Ok((value, text))
}

fn cert_json(e: &CertEntry) -> Value {
    json!({
        "group": e.group.name(),
        "thumbprint": e.thumbprint,
        "subject": e.subject,
        "not_after": e.not_after,
        "ca": e.is_ca,
        "crl": e.has_crl,
        "file": e.path.display().to_string(),
    })
}

fn list(pki: &Pki, group: Option<Group>) -> (Value, String) {
    let groups: Vec<Group> = group
        .map(|g| vec![g])
        .unwrap_or_else(|| Group::ALL.to_vec());
    let entries: Vec<CertEntry> = groups.iter().flat_map(|g| pki.list(*g)).collect();
    let mut text = format!("PKI directory {}\n", pki.root().display());
    if entries.is_empty() {
        text.push_str("(no certificates)\n");
    }
    for e in &entries {
        let kind = match (e.is_ca, e.has_crl) {
            (true, Some(true)) => "CA, CRL present",
            (true, _) => "CA, NO CRL (certificates it issued are refused)",
            (false, _) => "certificate",
        };
        text.push_str(&format!(
            "{:<9} {}  {}  (expires {}; {kind})\n",
            e.group.name(),
            e.thumbprint,
            e.subject,
            e.not_after.as_deref().unwrap_or("?")
        ));
    }
    let value = json!({
        "pki_dir": pki.root().display().to_string(),
        "certificates": entries.iter().map(cert_json).collect::<Vec<_>>(),
    });
    (value, text)
}

fn moved(
    pki: &Pki,
    action: &str,
    entries: &[CertEntry],
    to: Group,
) -> Result<(Value, String), Failure> {
    let mut out = Vec::new();
    let mut text = String::new();
    for entry in entries {
        let path = pki.relocate(entry, to).map_err(error)?;
        text.push_str(&format!(
            "{} {} -> {}\n",
            entry.thumbprint,
            entry.subject,
            to.name()
        ));
        let mut moved = entry.clone();
        moved.group = to;
        moved.path = path;
        out.push(cert_json(&moved));
    }
    Ok((json!({"action": action, "certificates": out}), text))
}

fn trust(pki: &Pki, target: &str) -> Result<(Value, String), Failure> {
    let path = Path::new(target);
    if !path.is_file() {
        let entry = find(pki, target, &[Group::Rejected])?;
        return moved(pki, "trust", &[entry], Group::Trusted);
    }
    // Import a file: every certificate in it becomes trusted (and leaves rejected/).
    let bytes =
        std::fs::read(path).map_err(|e| error(format!("cannot read {}: {e}", path.display())))?;
    let ders = parse_certificates(&bytes);
    if ders.is_empty() {
        return Err(error(format!("{} holds no certificate", path.display())));
    }
    pki.ensure_layout().map_err(error)?;
    let rejected = pki.list(Group::Rejected);
    let mut out = Vec::new();
    let mut text = String::new();
    for der in ders {
        for r in rejected.iter().filter(|r| r.der == der) {
            pki.remove(r).map_err(error)?;
        }
        let stored = pki.store(Group::Trusted, &der).map_err(error)?;
        let entry = pki
            .list(Group::Trusted)
            .into_iter()
            .find(|e| e.path == stored)
            .ok_or_else(|| error("the imported certificate cannot be read back"))?;
        text.push_str(&format!(
            "{} {} -> trusted\n",
            entry.thumbprint, entry.subject
        ));
        out.push(cert_json(&entry));
    }
    Ok((json!({"action": "trust", "certificates": out}), text))
}

fn add_issuer(pki: &Pki, file: &Path) -> Result<(Value, String), Failure> {
    let bytes =
        std::fs::read(file).map_err(|e| error(format!("cannot read {}: {e}", file.display())))?;
    let ders = parse_certificates(&bytes);
    if ders.is_empty() {
        return Err(error(format!("{} holds no certificate", file.display())));
    }
    for der in &ders {
        let cert = X509::from_der(der).map_err(|_| error("unreadable certificate"))?;
        if !cert.is_ca() {
            return Err(error(format!(
                "{} is not a CA certificate",
                cert.subject_text()
            )));
        }
    }
    pki.ensure_layout().map_err(error)?;
    let mut out = Vec::new();
    let mut text = String::new();
    for der in ders {
        let stored = pki.store(Group::Issuers, &der).map_err(error)?;
        if let Some(entry) = pki
            .list(Group::Issuers)
            .into_iter()
            .find(|e| e.path == stored)
        {
            text.push_str(&format!(
                "{} {} -> issuers\n",
                entry.thumbprint, entry.subject
            ));
            out.push(cert_json(&entry));
        }
    }
    Ok((json!({"action": "add-issuer", "certificates": out}), text))
}

fn add_crl(pki: &Pki, file: &Path) -> Result<(Value, String), Failure> {
    let bytes =
        std::fs::read(file).map_err(|e| error(format!("cannot read {}: {e}", file.display())))?;
    let crls = parse_crls(&bytes);
    if crls.is_empty() {
        return Err(error(format!("{} holds no CRL", file.display())));
    }
    let mut out = Vec::new();
    let mut text = String::new();
    for crl in crls {
        let path = pki.add_crl(&crl).map_err(error)?;
        text.push_str(&format!("CRL -> {}\n", path.display()));
        out.push(json!({"file": path.display().to_string()}));
    }
    Ok((json!({"action": "add-crl", "crls": out}), text))
}

/// Run as root, hand everything under the PKI directory to the directory's owner, so the
/// connector (running as that user) can read what an administrator added with `sudo`.
fn adopt_owner(root: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        let Ok(meta) = std::fs::metadata(root) else {
            return;
        };
        if meta.uid() == 0 {
            return;
        }
        fn walk(dir: &Path, uid: u32, gid: u32) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if let Ok(m) = std::fs::symlink_metadata(&path) {
                    if m.uid() == 0 {
                        let _ = std::os::unix::fs::lchown(&path, Some(uid), Some(gid));
                    }
                    if m.is_dir() {
                        walk(&path, uid, gid);
                    }
                }
            }
        }
        walk(root, meta.uid(), meta.gid());
    }
    #[cfg(not(unix))]
    let _ = root;
}
