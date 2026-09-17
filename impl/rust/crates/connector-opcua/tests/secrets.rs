//! Property: a password never leaks through configuration handling (spec "Secrets are never
//! disclosed"). Whatever else is wrong with a device, neither the validation error nor the
//! `Debug` of anything configure produces may contain the password or a password file's
//! contents.

use connector_opcua::{OpcuaConnector, OpcuaEndpoint};
use proptest::prelude::*;
use tedge_dot_sdk::{library, Connector};

/// A device fragment that makes validation fail (or, for the last one, pass) for a reason
/// unrelated to the password.
const BREAKAGES: &[&str] = &[
    r#"security_policy = "Basic999""#,
    r#"security_policy = "Basic256Sha256", security_mode = "none""#,
    r#"security_policy = "None", security_mode = "sign""#,
    r#"security_policy = "Basic256""#,
    r#"security_mode = "bogus""#,
    r#"user_certificate = "u.der""#,
    r#"password_file = "/nonexistent/pw""#,
    r#"endpoint = 5"#,
    r#"trust_any_server_certificate = "yes""#,
    r#"security_policy = "None""#,
];

fn config(address: &str) -> String {
    format!(
        r#"
[connector]
protocol = "opcua"

[[device]]
name = "plc"
protocol_address = {{ {address} }}

  [[device.point]]
  id = "t"
  datatype = "float64"
  address = {{ node_id = "ns=2;s=T" }}
"#
    )
}

fn toml_string(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

fn configure(text: &str, dir: &std::path::Path) -> String {
    let mut out = String::new();
    match library::resolve(text, dir) {
        Err(e) => out.push_str(&e),
        Ok(cfg) => {
            out.push_str(&format!("{cfg:?}"));
            let mut connector = OpcuaConnector::default();
            if let Err(e) = connector.configure(&cfg) {
                out.push_str(&format!("{e} {e:?}"));
            }
            if let Some(d) = cfg.devices.first() {
                if let Ok(ep) = serde_json::from_value::<OpcuaEndpoint>(d.protocol_address.clone())
                {
                    out.push_str(&format!("{ep:?}"));
                }
            }
        }
    }
    out
}

#[test]
fn syntax_error_does_not_quote_the_line() {
    let text =
        config(r#"endpoint = "opc.tcp://plc:4840/", user = "u", password = "hunter2-secret" oops"#);
    let e = library::resolve(&text, &std::env::temp_dir()).unwrap_err();
    assert!(!e.contains("hunter2-secret"), "{e}");
    assert!(e.contains("line 7"), "{e}");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn inline_password_never_leaks(
        password in "[A-Za-z0-9!#%&*+./:=?@^_~-]{6,24}",
        breakage in prop::sample::select(BREAKAGES),
    ) {
        let dir = std::env::temp_dir();
        let address = format!(
            r#"endpoint = "opc.tcp://plc:4840/", user = "operator", password = {}, {breakage}"#,
            toml_string(&password)
        );
        // `endpoint = 5` duplicates the key: keep the first to stay valid TOML.
        let address = address.replacen("endpoint = 5", "application = 5", 1);
        let out = configure(&config(&address), &dir);
        prop_assert!(!out.contains(&password), "password leaked: {out}");
    }

    #[test]
    fn password_file_contents_never_leak(
        password in "[A-Za-z0-9!#%&*+./:=?@^_~-]{6,24}",
        trailer in "[A-Za-z0-9]{6,12}",
        breakage in prop::sample::select(BREAKAGES),
    ) {
        let dir = std::env::temp_dir().join(format!("opcua-secret-prop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pw"), format!("{password}\n{trailer}\n")).unwrap();
        let breakage = breakage.replace(r#"password_file = "/nonexistent/pw""#, r#"security_mode = "x""#);
        let address = format!(
            r#"endpoint = "opc.tcp://plc:4840/", user = "operator", password_file = "pw", {breakage}"#
        );
        let out = configure(&config(&address), &dir);
        prop_assert!(!out.contains(&password), "password leaked: {out}");
        prop_assert!(!out.contains(&trailer), "file contents leaked: {out}");
    }

    #[test]
    fn non_string_password_is_not_echoed(n in 100_000i64..i64::MAX) {
        let address = format!(r#"endpoint = "opc.tcp://plc:4840/", user = "operator", password = {n}"#);
        let out = configure(&config(&address), &std::env::temp_dir());
        prop_assert!(!out.contains(&n.to_string()), "value leaked: {out}");
    }
}
