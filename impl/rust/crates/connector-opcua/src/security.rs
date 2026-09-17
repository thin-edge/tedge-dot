//! Security failures as the link status reports them (spec §8): a fixed set of reason
//! categories, each a prefix of the `reason` text, and the OPC UA status codes that map to
//! them. The C connector uses the same table (impl/c/connectors/opcua/connector_opcua.c).

use opcua::types::{EndpointDescription, MessageSecurityMode, StatusCode, UserTokenType};

use crate::config::{DeviceSecurity, Identity, Policy, SecurityMode};

/// Reason prefixes of security failures.
pub const CERTIFICATE_UNTRUSTED: &str = "certificate untrusted:";
pub const CERTIFICATE_INVALID: &str = "certificate invalid:";
pub const CERTIFICATE_REVOKED: &str = "certificate revoked:";
pub const NO_MATCHING_ENDPOINT: &str = "no matching endpoint:";
pub const IDENTITY_REJECTED: &str = "identity rejected:";
pub const IDENTITY_UNSUPPORTED: &str = "identity unsupported:";
pub const PLAINTEXT_PASSWORD_REFUSED: &str = "plaintext password refused:";
pub const APPLICATION_CERTIFICATE: &str = "application certificate:";

/// The reason category of a status code, when it is a security failure about the server
/// certificate or the user identity.
pub fn category(status: StatusCode) -> Option<&'static str> {
    // Compare the code without its info bits.
    let code = StatusCode::from(status.bits() & 0xFFFF_0000);
    Some(match code {
        StatusCode::BadCertificateUntrusted | StatusCode::BadSecurityChecksFailed => {
            CERTIFICATE_UNTRUSTED
        }
        StatusCode::BadCertificateTimeInvalid
        | StatusCode::BadCertificateIssuerTimeInvalid
        | StatusCode::BadCertificateHostNameInvalid
        | StatusCode::BadCertificateUriInvalid
        | StatusCode::BadCertificateInvalid
        | StatusCode::BadCertificatePolicyCheckFailed
        | StatusCode::BadCertificateUseNotAllowed
        | StatusCode::BadCertificateIssuerUseNotAllowed
        | StatusCode::BadCertificateChainIncomplete => CERTIFICATE_INVALID,
        StatusCode::BadCertificateRevoked
        | StatusCode::BadCertificateIssuerRevoked
        | StatusCode::BadCertificateRevocationUnknown
        | StatusCode::BadCertificateIssuerRevocationUnknown => CERTIFICATE_REVOKED,
        StatusCode::BadIdentityTokenRejected
        | StatusCode::BadIdentityTokenInvalid
        | StatusCode::BadUserAccessDenied => IDENTITY_REJECTED,
        _ => return None,
    })
}

/// A short human description of a certificate status code, for the reason text.
pub fn describe(status: StatusCode) -> &'static str {
    let code = StatusCode::from(status.bits() & 0xFFFF_0000);
    match code {
        StatusCode::BadCertificateUntrusted | StatusCode::BadSecurityChecksFailed => {
            "the server certificate is not trusted"
        }
        StatusCode::BadCertificateTimeInvalid => {
            "the server certificate is expired or not yet valid"
        }
        StatusCode::BadCertificateIssuerTimeInvalid => {
            "a CA certificate on the chain is expired or not yet valid"
        }
        StatusCode::BadCertificateHostNameInvalid => {
            "the server certificate does not name the endpoint host"
        }
        StatusCode::BadCertificateUriInvalid => {
            "the server certificate's application URI does not match the server"
        }
        StatusCode::BadCertificatePolicyCheckFailed => {
            "the server certificate's key does not meet the security policy"
        }
        StatusCode::BadCertificateRevoked => "the server certificate is revoked",
        StatusCode::BadCertificateIssuerRevoked => "a CA certificate on the chain is revoked",
        StatusCode::BadCertificateRevocationUnknown => {
            "revocation unknown: no CRL for the issuing CA (add its CRL, an empty one if it revoked nothing)"
        }
        StatusCode::BadCertificateIssuerRevocationUnknown => {
            "revocation unknown: no CRL for a CA on the chain (add its CRL, an empty one if it revoked nothing)"
        }
        StatusCode::BadIdentityTokenRejected
        | StatusCode::BadIdentityTokenInvalid
        | StatusCode::BadUserAccessDenied => "the server rejected the user identity",
        _ => "the server certificate is invalid",
    }
}

/// The wire form of a configured mode.
pub fn message_mode(mode: SecurityMode) -> MessageSecurityMode {
    match mode {
        SecurityMode::None => MessageSecurityMode::None,
        SecurityMode::Sign => MessageSecurityMode::Sign,
        SecurityMode::SignAndEncrypt => MessageSecurityMode::SignAndEncrypt,
    }
}

fn mode_name(mode: MessageSecurityMode) -> &'static str {
    match mode {
        MessageSecurityMode::None => "none",
        MessageSecurityMode::Sign => "sign",
        MessageSecurityMode::SignAndEncrypt => "sign_and_encrypt",
        _ => "invalid",
    }
}

fn token_type(identity: &Identity) -> UserTokenType {
    match identity {
        Identity::Anonymous => UserTokenType::Anonymous,
        Identity::UserName { .. } => UserTokenType::UserName,
        Identity::X509 { .. } => UserTokenType::Certificate,
    }
}

/// Pick the server endpoint a device connects to (spec "Endpoint selection"): the one with the
/// device's policy and mode that offers a user token policy for its identity, preferring the
/// highest `securityLevel`. Its URL is replaced by the configured one, so the session dials the
/// configured host and port, and host-name checks use them too. The error is the link reason.
pub fn select_endpoint(
    endpoints: &[EndpointDescription],
    security: &DeviceSecurity,
    configured_url: &str,
) -> Result<EndpointDescription, String> {
    let uri = security.policy.uri();
    let mode = message_mode(security.mode);
    let matching: Vec<&EndpointDescription> = endpoints
        .iter()
        .filter(|e| e.security_policy_uri.as_ref() == uri && e.security_mode == mode)
        .collect();
    if matching.is_empty() {
        let mut offered: Vec<String> = endpoints
            .iter()
            .map(|e| {
                let policy = Policy::from_uri(e.security_policy_uri.as_ref())
                    .map(|p| p.name().to_string())
                    .unwrap_or_else(|| e.security_policy_uri.to_string());
                format!("{policy}/{}", mode_name(e.security_mode))
            })
            .collect();
        offered.sort();
        offered.dedup();
        return Err(format!(
            "{NO_MATCHING_ENDPOINT} the server offers no {}/{} endpoint (it offers {})",
            security.policy.name(),
            security.mode.name(),
            if offered.is_empty() {
                "none".to_string()
            } else {
                offered.join(", ")
            }
        ));
    }
    let wanted = token_type(&security.identity);
    let mut usable: Vec<&EndpointDescription> = matching
        .into_iter()
        // The endpoint must list a token policy of the identity's type, anonymous included:
        // neither async-opcua nor open62541 can activate a session without one, so an endpoint
        // with no (null or empty) token policies is not usable (the C build's select_endpoint).
        .filter(|e| e.find_policy(wanted).is_some())
        .collect();
    usable.sort_by_key(|e| std::cmp::Reverse(e.security_level));
    let Some(chosen) = usable.first() else {
        return Err(format!(
            "{IDENTITY_UNSUPPORTED} the server's {}/{} endpoint accepts no {} user identity",
            security.policy.name(),
            security.mode.name(),
            security.identity.kind()
        ));
    };
    let mut chosen = (*chosen).clone();
    chosen.endpoint_url = configured_url.into();
    Ok(chosen)
}

/// Whether a password sent to `endpoint` would travel unencrypted: OPC UA Part 4, Table 193 —
/// only on a channel without message security whose username token policy names no security
/// policy (or `None`). On a signed or encrypted channel an unnamed policy means the channel's.
pub fn password_in_plaintext(endpoint: &EndpointDescription) -> bool {
    if endpoint.security_mode != MessageSecurityMode::None {
        return false;
    }
    match endpoint.find_policy(UserTokenType::UserName) {
        None => false,
        Some(policy) => {
            let uri = policy.security_policy_uri.as_ref();
            uri.is_empty() || Policy::from_uri(uri) == Some(Policy::None)
        }
    }
}

/// The host of an `opc.tcp://host:port/path` URL (brackets stripped from an IPv6 literal).
pub fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split('/').next().unwrap_or(rest);
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next().unwrap_or(v6)
    } else {
        authority.rsplit_once(':').map_or(authority, |(h, _)| h)
    };
    (!host.is_empty()).then(|| host.to_string())
}

/// Where a certificate stands against its `notAfter` at `now` (Unix seconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expiry {
    Valid,
    /// Expires within the warning window; whole days left.
    ExpiresSoon(i64),
    Expired,
}

pub fn expiry(not_after: i64, now: i64, warning_days: i64) -> Expiry {
    if now > not_after {
        Expiry::Expired
    } else if not_after - now <= warning_days * 86_400 {
        Expiry::ExpiresSoon((not_after - now) / 86_400)
    } else {
        Expiry::Valid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Secret;
    use opcua::types::{UAString, UserTokenPolicy};

    fn endpoint(
        policy: Policy,
        mode: MessageSecurityMode,
        level: u8,
        tokens: &[(UserTokenType, &str)],
    ) -> EndpointDescription {
        EndpointDescription {
            endpoint_url: "opc.tcp://advertised:4840/".into(),
            security_policy_uri: policy.uri().into(),
            security_mode: mode,
            security_level: level,
            user_identity_tokens: Some(
                tokens
                    .iter()
                    .enumerate()
                    .map(|(i, (t, uri))| UserTokenPolicy {
                        policy_id: UAString::from(format!("p{i}")),
                        token_type: *t,
                        security_policy_uri: UAString::from(*uri),
                        ..Default::default()
                    })
                    .collect(),
            ),
            ..Default::default()
        }
    }

    fn device(policy: Policy, mode: SecurityMode, identity: Identity) -> DeviceSecurity {
        DeviceSecurity {
            policy,
            mode,
            identity,
            trust_any_server_certificate: false,
            allow_plaintext_password: false,
        }
    }

    fn user() -> Identity {
        Identity::UserName {
            user: "u".into(),
            password: Secret::new("p".into()),
        }
    }

    #[test]
    fn selects_matching_endpoint_and_keeps_configured_url() {
        let endpoints = vec![
            endpoint(
                Policy::None,
                MessageSecurityMode::None,
                0,
                &[(UserTokenType::Anonymous, "")],
            ),
            endpoint(
                Policy::Basic256Sha256,
                MessageSecurityMode::Sign,
                1,
                &[(UserTokenType::Anonymous, "")],
            ),
            endpoint(
                Policy::Basic256Sha256,
                MessageSecurityMode::SignAndEncrypt,
                3,
                &[(UserTokenType::Anonymous, "")],
            ),
            endpoint(
                Policy::Basic256Sha256,
                MessageSecurityMode::SignAndEncrypt,
                9,
                &[(UserTokenType::UserName, "")],
            ),
        ];
        let sec = device(
            Policy::Basic256Sha256,
            SecurityMode::SignAndEncrypt,
            Identity::Anonymous,
        );
        let chosen = select_endpoint(&endpoints, &sec, "opc.tcp://192.168.1.20:4840/").unwrap();
        assert_eq!(chosen.security_level, 3);
        assert_eq!(chosen.endpoint_url.as_ref(), "opc.tcp://192.168.1.20:4840/");

        // The username identity picks the higher-level endpoint that offers it.
        let sec = device(Policy::Basic256Sha256, SecurityMode::SignAndEncrypt, user());
        assert_eq!(
            select_endpoint(&endpoints, &sec, "u")
                .unwrap()
                .security_level,
            9
        );
    }

    #[test]
    fn no_matching_endpoint_lists_offers() {
        let endpoints = vec![
            endpoint(Policy::None, MessageSecurityMode::None, 0, &[]),
            endpoint(Policy::Basic256Sha256, MessageSecurityMode::Sign, 1, &[]),
            endpoint(Policy::Basic256Sha256, MessageSecurityMode::Sign, 2, &[]),
        ];
        let sec = device(
            Policy::Aes256Sha256RsaPss,
            SecurityMode::SignAndEncrypt,
            Identity::Anonymous,
        );
        let e = select_endpoint(&endpoints, &sec, "u").unwrap_err();
        assert!(e.starts_with(NO_MATCHING_ENDPOINT), "{e}");
        assert!(
            e.ends_with("(it offers Basic256Sha256/sign, None/none)"),
            "{e}"
        );
    }

    #[test]
    fn unsupported_identity() {
        let endpoints = vec![endpoint(
            Policy::Basic256Sha256,
            MessageSecurityMode::SignAndEncrypt,
            1,
            &[(UserTokenType::Anonymous, "")],
        )];
        let x509 = Identity::X509 {
            certificate: "c".into(),
            private_key: "k".into(),
        };
        let e = select_endpoint(
            &endpoints,
            &device(Policy::Basic256Sha256, SecurityMode::SignAndEncrypt, x509),
            "u",
        )
        .unwrap_err();
        assert!(
            e.starts_with(IDENTITY_UNSUPPORTED) && e.contains("certificate"),
            "{e}"
        );
    }

    #[test]
    fn endpoint_without_token_policies_is_unusable_even_for_anonymous() {
        // Null and empty lists alike (the C build cannot tell them apart), and neither
        // client library can activate a session without a listed policy.
        let empty = endpoint(Policy::Basic256Sha256, MessageSecurityMode::SignAndEncrypt, 5, &[]);
        let null = EndpointDescription {
            user_identity_tokens: None,
            ..empty.clone()
        };
        let anonymous = device(
            Policy::Basic256Sha256,
            SecurityMode::SignAndEncrypt,
            Identity::Anonymous,
        );
        for e in [&empty, &null] {
            let err = select_endpoint(std::slice::from_ref(e), &anonymous, "u").unwrap_err();
            assert!(err.starts_with(IDENTITY_UNSUPPORTED), "{err}");
        }
        // One that lists it is picked over a higher-level one that lists nothing.
        let listed = endpoint(
            Policy::Basic256Sha256,
            MessageSecurityMode::SignAndEncrypt,
            1,
            &[(UserTokenType::Anonymous, "")],
        );
        let chosen = select_endpoint(&[null, listed], &anonymous, "u").unwrap();
        assert_eq!(chosen.security_level, 1);
    }

    #[test]
    fn plaintext_detection_follows_table_193() {
        let none_uri = Policy::None.uri();
        let pw =
            |mode, uri: &str| endpoint(Policy::None, mode, 0, &[(UserTokenType::UserName, uri)]);
        assert!(password_in_plaintext(&pw(MessageSecurityMode::None, "")));
        assert!(password_in_plaintext(&pw(
            MessageSecurityMode::None,
            &none_uri
        )));
        assert!(!password_in_plaintext(&pw(
            MessageSecurityMode::None,
            &Policy::Basic256Sha256.uri()
        )));
        assert!(!password_in_plaintext(&pw(MessageSecurityMode::Sign, "")));
        assert!(!password_in_plaintext(&pw(
            MessageSecurityMode::SignAndEncrypt,
            &none_uri
        )));
    }

    #[test]
    fn hosts_from_urls() {
        assert_eq!(
            url_host("opc.tcp://plc.example.com:4840/x").as_deref(),
            Some("plc.example.com")
        );
        assert_eq!(
            url_host("opc.tcp://10.0.0.5:4840").as_deref(),
            Some("10.0.0.5")
        );
        assert_eq!(
            url_host("opc.tcp://[fe80::1]:4840/").as_deref(),
            Some("fe80::1")
        );
        assert_eq!(
            url_host("opc.tcp://simulator/").as_deref(),
            Some("simulator")
        );
        assert_eq!(url_host("opc.tcp://:4840/"), None);
    }

    #[test]
    fn expiry_windows() {
        let day = 86_400;
        assert_eq!(expiry(100 * day, 0, 30), Expiry::Valid);
        assert_eq!(expiry(30 * day, 0, 30), Expiry::ExpiresSoon(30));
        assert_eq!(expiry(10 * day + 5, 0, 30), Expiry::ExpiresSoon(10));
        assert_eq!(expiry(0, 1, 30), Expiry::Expired);
    }

    #[test]
    fn categories() {
        assert_eq!(
            category(StatusCode::BadCertificateUntrusted),
            Some(CERTIFICATE_UNTRUSTED)
        );
        assert_eq!(
            category(StatusCode::BadCertificateHostNameInvalid),
            Some(CERTIFICATE_INVALID)
        );
        assert_eq!(
            category(StatusCode::BadCertificatePolicyCheckFailed),
            Some(CERTIFICATE_INVALID)
        );
        assert_eq!(
            category(StatusCode::BadCertificateIssuerRevocationUnknown),
            Some(CERTIFICATE_REVOKED)
        );
        assert_eq!(
            category(StatusCode::BadUserAccessDenied),
            Some(IDENTITY_REJECTED)
        );
        assert_eq!(category(StatusCode::BadTimeout), None);
        assert_eq!(category(StatusCode::Good), None);
    }
}
