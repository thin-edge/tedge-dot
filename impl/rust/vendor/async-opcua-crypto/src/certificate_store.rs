// OPCUA for Rust
// SPDX-License-Identifier: MPL-2.0
// Copyright (C) 2017-2024 Adam Lock

//! The certificate store holds and retrieves private keys and certificates from disk. It is responsible
//! for checking certificates supplied by the remote end to see if they are valid and trusted or not.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use tracing::{debug, error, info, warn};

use opcua_types::status_code::StatusCode;

use crate::PrivateKey;

use super::{
    security_policy::SecurityPolicy,
    x509::{X509Data, X509},
};

// tedge-dot patch: the OPC UA Part 12 (F.1) layout shared with the C build
// (doc/connectors/opcua-connector-spec.md). Upstream uses own/cert.der, private/private.pem and
// flat trusted/ and rejected/ directories.
/// Default path to the applications own certificate
pub const OWN_CERTIFICATE_PATH: &str = "own/certs/cert.der";
/// Default path to the applications own private key
pub const OWN_PRIVATE_KEY_PATH: &str = "own/private/key.pem";
use crate::trust_list::{
    TrustList, ISSUER_CERTS_DIR, ISSUER_CRL_DIR, REJECTED_CERTS_DIR, TRUSTED_CERTS_DIR,
    TRUSTED_CRL_DIR,
};

/// The certificate store manages the storage of a server/client's own certificate & private key
/// and the trust / rejection of certificates from the other end.
pub struct CertificateStore {
    /// Path to the applications own certificate
    own_certificate_path: PathBuf,
    /// Path to the applications own private key
    own_private_key_path: PathBuf,
    /// Path to the certificate store on disk
    pub(crate) pki_path: PathBuf,
    /// Timestamps of the cert are normally checked on the cert to ensure it cannot be used before
    /// or after its limits, but this check can be disabled.
    check_time: bool,
    /// This option lets you skip additional certificate validations (e.g. hostname, application
    /// uri and the not before / after values). Certificates are always checked to see if they are
    /// trusted and have a valid key length.
    skip_verify_certs: bool,
    /// Ordinarily an unknown cert will be dropped into the rejected folder, but it can be dropped
    /// into the trusted folder if this flag is set. Certs in the trusted folder must still pass
    /// validity checks.
    trust_unknown_certs: bool,
}

impl CertificateStore {
    /// Sets up the certificate store to the specified PKI directory.
    /// It is a bad idea to have more than one running instance pointing to the same path
    /// location on disk.
    pub fn new(pki_path: &Path) -> CertificateStore {
        CertificateStore {
            own_certificate_path: PathBuf::from(OWN_CERTIFICATE_PATH),
            own_private_key_path: PathBuf::from(OWN_PRIVATE_KEY_PATH),
            pki_path: pki_path.to_path_buf(),
            check_time: true,
            skip_verify_certs: false,
            trust_unknown_certs: false,
        }
    }

    /// Create a new certificate store with application certificate from the given
    /// `cert_path`.
    pub fn new_with_x509_data<X>(
        pki_path: &Path,
        overwrite: bool,
        cert_path: Option<&Path>,
        pkey_path: Option<&Path>,
        x509_data: Option<X>,
    ) -> (CertificateStore, Option<X509>, Option<PrivateKey>)
    where
        X: Into<X509Data>,
    {
        let mut certificate_store = CertificateStore::new(pki_path);
        if let (Some(cert_path), Some(pkey_path)) = (cert_path, pkey_path) {
            certificate_store.own_certificate_path = cert_path.to_path_buf();
            certificate_store.own_private_key_path = pkey_path.to_path_buf();
        }
        // tedge-dot patch: only a store that may have to generate its certificate creates the
        // PKI directory; reading an existing one (or finding none) changes nothing on disk.
        let (cert, pkey) = if x509_data.is_some() && certificate_store.ensure_pki_path().is_err() {
            error!("Folder for storing certificates cannot be examined so server has no application instance certificate or private key.");
            (None, None)
        } else {
            let cert = certificate_store.read_own_cert();
            let pkey = certificate_store.read_own_pkey();
            match (cert, pkey, x509_data) {
                (Ok(cert), Ok(pkey), _) => (Some(cert), Some(pkey)),
                (_, _, Some(x509_data)) => {
                    info!("Creating sample application instance certificate and private key");
                    let x509_data = x509_data.into();
                    let result = certificate_store
                        .create_and_store_application_instance_cert(&x509_data, overwrite);
                    match result {
                        Ok((cert, pkey)) => (Some(cert), Some(pkey)),
                        Err(err) => {
                            error!("Certificate creation failed, error = {}", err);
                            (None, None)
                        }
                    }
                }
                (Err(e1), Err(e2), _) => {
                    error!("Failed to get cert and private key: {e1}, {e2}");
                    (None, None)
                }
                (Err(e), _, _) | (_, Err(e), _) => {
                    error!("Failed to get cert or private key: {e}");
                    (None, None)
                }
            }
        };
        (certificate_store, cert, pkey)
    }

    /// Set `skip_verify_certs` to not verify incoming certificates.
    pub fn set_skip_verify_certs(&mut self, skip_verify_certs: bool) {
        self.skip_verify_certs = skip_verify_certs;
    }

    /// Set `trust_unknown_certs` to automatically trust valid but
    /// untrusted certificates.
    pub fn set_trust_unknown_certs(&mut self, trust_unknown_certs: bool) {
        self.trust_unknown_certs = trust_unknown_certs;
    }

    /// Check expiration time of incoming certificates.
    pub fn set_check_time(&mut self, check_time: bool) {
        self.check_time = check_time;
    }

    /// Reads a private key from a path on disk.
    pub fn read_pkey(path: &Path) -> Result<PrivateKey, String> {
        if let Ok(pkey) = PrivateKey::read_pem_file(path) {
            return Ok(pkey);
        }

        Err(format!("Cannot read pkey from path {path:?}"))
    }

    /// Reads the store's own certificate
    pub fn read_own_cert(&self) -> Result<X509, String> {
        CertificateStore::read_cert(&self.own_certificate_path()).map_err(|e| {
            format!(
                "Cannot read cert from path {:?}: {e}",
                self.own_certificate_path()
            )
        })
    }

    /// Read own private key from file.
    pub fn read_own_pkey(&self) -> Result<PrivateKey, String> {
        CertificateStore::read_pkey(&self.own_private_key_path()).map_err(|e| {
            format!(
                "Cannot read pkey from path {:?}: {e}",
                self.own_private_key_path()
            )
        })
    }

    /// Create a certificate and key pair to the specified locations
    pub fn create_certificate_and_key(
        args: &X509Data,
        overwrite: bool,
        cert_path: &Path,
        pkey_path: &Path,
    ) -> Result<(X509, PrivateKey), String> {
        let (cert, pkey) = X509::cert_and_pkey(args)?;

        // Write the public cert
        let _ = CertificateStore::store_cert(&cert, cert_path, overwrite)?;

        // Write the private key
        use rsa::pkcs8;
        use x509_cert::der::pem::PemLabel;
        let doc = pkey.to_der().unwrap();
        let pem = doc
            .to_pem(rsa::pkcs8::PrivateKeyInfo::PEM_LABEL, pkcs8::LineEnding::CR)
            .unwrap();
        let _ = CertificateStore::write_to_file(pem.as_bytes(), pkey_path, overwrite)?;
        Ok((cert, pkey))
    }

    /// This function will use the supplied arguments to create an Application Instance Certificate
    /// consisting of a X509v3 certificate and public/private key pair. The cert (including pubkey)
    /// and private key will be written to disk under the pki path.
    pub fn create_and_store_application_instance_cert(
        &self,
        args: &X509Data,
        overwrite: bool,
    ) -> Result<(X509, PrivateKey), String> {
        CertificateStore::create_certificate_and_key(
            args,
            overwrite,
            &self.own_certificate_path(),
            &self.own_private_key_path(),
        )
    }

    /// Validates the cert as trusted and valid. If the cert is unknown, it will be written to
    /// the rejected folder so that the administrator can manually move it to the trusted folder.
    ///
    /// # Errors
    ///
    /// A non `Good` status code indicates a failure in the cert or in some action required in
    /// order to validate it.
    ///
    pub fn validate_or_reject_application_instance_cert(
        &self,
        cert: &X509,
        security_policy: SecurityPolicy,
        hostname: Option<&str>,
        application_uri: Option<&str>,
    ) -> Result<(), StatusCode> {
        self.validate_application_instance_cert(cert, security_policy, hostname, application_uri)
    }

    /// Validates the certificate according to the strictness set in the CertificateStore itself.
    ///
    /// tedge-dot patch: the trust decision reads the PKI directory on every call (pinned
    /// certificates, CA chains through `trusted/` and `issuers/`, CRLs) through
    /// [`TrustList`]; an untrusted certificate is copied to `rejected/certs`. With
    /// `trust_unknown_certs` every certificate is accepted and nothing is written (upstream
    /// copies unknown certificates into the trusted folder).
    ///
    /// A non `Good` status code indicates a failure in the cert or in some action required in
    /// order to validate it:
    /// `BadCertificateUntrusted`, `BadCertificateRevoked`, `BadCertificateIssuerRevoked`,
    /// `BadCertificateRevocationUnknown`, `BadCertificateIssuerRevocationUnknown`,
    /// `BadCertificateIssuerTimeInvalid`, `BadCertificatePolicyCheckFailed` (key length),
    /// `BadCertificateTimeInvalid`, `BadCertificateHostNameInvalid`,
    /// `BadCertificateUriInvalid`, `BadCertificateInvalid`.
    pub fn validate_application_instance_cert(
        &self,
        cert: &X509,
        security_policy: SecurityPolicy,
        hostname: Option<&str>,
        application_uri: Option<&str>,
    ) -> Result<(), StatusCode> {
        let cert_file_name = CertificateStore::cert_file_name(cert);
        debug!("Validating cert {}", cert_file_name);

        if self.trust_unknown_certs {
            warn!(
                "Certificate {} is accepted without verification (all certificates are trusted)",
                cert_file_name
            );
            return Ok(());
        }

        let der = cert.to_der().map_err(|_| StatusCode::BadCertificateInvalid)?;
        let now = chrono::Utc::now();
        let trust = TrustList::load(&self.pki_path);
        if let Err(failure) = trust.verify(&der, &now) {
            let status = failure.status_code();
            if status == StatusCode::BadCertificateUntrusted && !trust.is_rejected(&der) {
                match self.store_rejected_cert(cert) {
                    Ok(path) => warn!(
                        "Certificate {} is untrusted; stored in {}",
                        cert_file_name,
                        path.display()
                    ),
                    Err(e) => warn!("Certificate {} is untrusted; {}", cert_file_name, e),
                }
            } else {
                warn!("Certificate {} failed validation: {}", cert_file_name, status);
            }
            return Err(status);
        }

        // Check that the certificate is the right length for the security policy
        match cert.key_length() {
            Err(_) => {
                error!("Cannot read key length from certificate {}", cert_file_name);
                return Err(StatusCode::BadCertificateInvalid);
            }
            Ok(key_length) => {
                if !security_policy.is_valid_keylength(key_length) {
                    warn!(
                        "Certificate {} has an invalid key length {} for the policy {}",
                        cert_file_name, key_length, security_policy
                    );
                    return Err(StatusCode::BadCertificatePolicyCheckFailed);
                }
            }
        }

        if self.skip_verify_certs {
            debug!(
                "Skipping additional verifications for certificate {}",
                cert_file_name
            );
            return Ok(());
        }

        // Now inspect the cert not before / after values to ensure its validity
        if self.check_time {
            cert.is_time_valid(&now)?;
        }

        // Compare the hostname of the cert against the cert supplied
        if let Some(hostname) = hostname {
            cert.is_hostname_valid(hostname)?;
        }

        // Compare the application / product uri to the supplied application description
        if let Some(application_uri) = application_uri {
            cert.is_application_uri_valid(application_uri)?;
        }
        Ok(())
    }

    /// Returns the file name a certificate is stored under: `<CN>_<thumbprint>.der`, with the
    /// thumbprint in lower-case hex and every character of the common name other than ASCII
    /// letters, digits, `-` and `.` replaced by `_` (tedge-dot patch; upstream uses
    /// `<CN> [<THUMBPRINT>].der`).
    pub fn cert_file_name(cert: &X509) -> String {
        let cn: String = cert
            .common_name_value()
            .unwrap_or_default()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' })
            .collect();
        format!("{}_{}.der", cn, cert.thumbprint().as_hex_string().to_ascii_lowercase())
    }

    /// Creates the PKI directory structure
    ///
    /// # Errors
    ///
    /// A string description of any failure
    ///
    pub fn ensure_pki_path(&self) -> Result<(), String> {
        let subdirs = [
            "own/certs",
            "own/private",
            TRUSTED_CERTS_DIR,
            TRUSTED_CRL_DIR,
            ISSUER_CERTS_DIR,
            ISSUER_CRL_DIR,
            REJECTED_CERTS_DIR,
        ];
        for subdir in &subdirs {
            CertificateStore::ensure_dir(&self.pki_path.join(subdir))?;
        }
        Ok(())
    }

    /// Ensure the directory exists, creating it if necessary
    ///
    /// # Errors
    ///
    /// A string description of any failure
    ///
    fn ensure_dir(path: &Path) -> Result<(), String> {
        if path.exists() {
            if !path.is_dir() {
                Err(format!("{} is not a directory ", path.display()))
            } else {
                Ok(())
            }
        } else {
            std::fs::create_dir_all(path)
                .map_err(|_| format!("Cannot make directories for {}", path.display()))
        }
    }

    /// Get path to application instance certificate
    pub fn own_certificate_path(&self) -> PathBuf {
        let mut path = PathBuf::from(&self.pki_path);
        path.push(&self.own_certificate_path);
        path
    }

    /// Get path to application instance private key
    pub fn own_private_key_path(&self) -> PathBuf {
        let mut path = PathBuf::from(&self.pki_path);
        path.push(&self.own_private_key_path);
        path
    }

    /// Get the path to the rejected certs dir
    pub fn rejected_certs_dir(&self) -> PathBuf {
        let mut path = PathBuf::from(&self.pki_path);
        path.push(REJECTED_CERTS_DIR);
        path
    }

    /// Get the path to the trusted certs dir
    pub fn trusted_certs_dir(&self) -> PathBuf {
        let mut path = PathBuf::from(&self.pki_path);
        path.push(TRUSTED_CERTS_DIR);
        path
    }

    /// Write a cert to the rejected directory. If the write succeeds, the function
    /// returns a path to the written file.
    ///
    /// # Errors
    ///
    /// A string description of any failure
    ///
    pub fn store_rejected_cert(&self, cert: &X509) -> Result<PathBuf, String> {
        // Store the cert in the rejected folder where untrusted certs go
        let cert_file_name = CertificateStore::cert_file_name(cert);
        let mut cert_path = self.rejected_certs_dir();
        cert_path.push(&cert_file_name);
        let _ = CertificateStore::store_cert(cert, &cert_path, true)?;
        Ok(cert_path)
    }

    /// Writes a cert to the specified directory
    ///
    /// # Errors
    ///
    /// A string description of any failure
    ///
    fn store_cert(cert: &X509, path: &Path, overwrite: bool) -> Result<usize, String> {
        let der = cert.to_der().unwrap();
        info!("Writing X509 cert to {}", path.display());
        CertificateStore::write_to_file(&der, path, overwrite)
    }

    /// Reads an X509 certificate in .def or .pem format from disk
    ///
    /// # Errors
    ///
    /// A string description of any failure
    ///
    pub fn read_cert(path: &Path) -> Result<X509, String> {
        // tedge-dot patch: recognised by content (DER or PEM), not by file extension.
        let bytes = std::fs::read(path)
            .map_err(|_| format!("Could not open cert file {}", path.display()))?;
        crate::trust_list::parse_certificates(&bytes)
            .first()
            .and_then(|der| X509::from_der(der).ok())
            .ok_or_else(|| format!("Could not read cert from cert file {}", path.display()))
    }

    /// Writes bytes to file and returns the size written, or an error reason for failure.
    ///
    /// # Errors
    ///
    /// A string description of any failure
    ///
    fn write_to_file(bytes: &[u8], file_path: &Path, overwrite: bool) -> Result<usize, String> {
        if !overwrite && file_path.exists() {
            Err(format!("File {} already exists and will not be overwritten. Enable overwrite to disable this safeguard.", file_path.display()))
        } else {
            if let Some(parent) = file_path.parent() {
                CertificateStore::ensure_dir(parent)?;
            }
            match File::create(file_path) {
                Ok(mut file) => file
                    .write(bytes)
                    .map_err(|_| format!("Could not write bytes to file {}", file_path.display())),
                Err(_) => Err(format!("Could not create file {}", file_path.display())),
            }
        }
    }
}
