//! TLS helper functions.
//!
//! This module provides convenience functions for loading
//! certificates and private keys from PEM files and constructing
//! `rustls` client and server configurations.  It also includes
//! utilities for extracting Rabbit IDs from certificates via a
//! custom extension.  The custom extension is not required for
//! TLS itself but can be used to bind transport identities to
//! protocol identities.

use std::fs;
use std::io::BufReader;
use std::path::Path;
use anyhow::{anyhow, Result};
use tokio_rustls::rustls::{Certificate, PrivateKey, ServerConfig, ClientConfig, RootCertStore};
use x509_parser::pem::parse_x509_pem;
use x509_parser::prelude::X509Certificate;
use crate::security::identity_cert::extract_rabbit_id_from_cert;

/// Load a vector of certificates from a PEM file.  Errors are
/// propagated if the file cannot be read or contains invalid
/// certificate data.
pub fn load_certs(path: &Path) -> Result<Vec<Certificate>> {
    let certfile = fs::File::open(path)?;
    let mut reader = BufReader::new(certfile);
    let certs = rustls_pemfile::certs(&mut reader)?
        .into_iter()
        .map(Certificate)
        .collect();
    Ok(certs)
}

/// Load a private key from a PEM file.  Supports RSA keys.
pub fn load_private_key(path: &Path) -> Result<PrivateKey> {
    let keyfile = fs::File::open(path)?;
    let mut reader = BufReader::new(keyfile);
    let keys = rustls_pemfile::rsa_private_keys(&mut reader)?;
    if let Some(k) = keys.into_iter().next() {
        Ok(PrivateKey(k))
    } else {
        Err(anyhow!("no private key found in {}", path.display()))
    }
}

/// Create a TLS client configuration trusting the given root
/// certificates.  The CA file should contain PEM encoded CA
/// certificates.  The returned configuration uses safe defaults.
pub fn make_client_config(ca_path: &Path) -> Result<std::sync::Arc<ClientConfig>> {
    let mut root_store = RootCertStore::empty();
    let certs = load_certs(ca_path)?;
    for cert in certs {
        root_store.add(&cert)?;
    }
    let config = ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    Ok(std::sync::Arc::new(config))
}

/// Create a TLS server configuration from certificate and key
/// PEM files.  Client authentication is not required by default.
pub fn make_server_config(cert_path: &Path, key_path: &Path) -> Result<std::sync::Arc<ServerConfig>> {
    let certs = load_certs(cert_path)?;
    let key = load_private_key(key_path)?;
    let config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    Ok(std::sync::Arc::new(config))
}
