//! TLS for the relay client: [`client_config`] trusts the public CAs
//! (webpki roots), the operating system's store (where a company installs
//! its own CA), and the PEM file in `NANDI_RELAY_CA_FILE` if set. The
//! relay serves `wss://` itself or sits behind a TLS-terminating proxy
//! (vaultdbai/nandi-relay).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::CertificateDer;

use super::client::ensure_tls_provider;

/// An extra CA (PEM) the client trusts for the relay
pub const CA_FILE_ENV: &str = "NANDI_RELAY_CA_FILE";

fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    let certs = CertificateDer::pem_file_iter(path)
        .map_err(|e| format!("Couldn't read the TLS certificate {}: {e}", path.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Couldn't read the TLS certificate {}: {e}", path.display()))?;
    if certs.is_empty() {
        return Err(format!("{} has no certificate", path.display()));
    }
    Ok(certs)
}

/// The client side, built once: public CAs, the OS store, and
/// `NANDI_RELAY_CA_FILE`
pub fn client_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: std::sync::OnceLock<Arc<rustls::ClientConfig>> = std::sync::OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let extra = crate::utils::env::var(CA_FILE_ENV)
                .filter(|path| !path.trim().is_empty())
                .map(PathBuf::from);
            build_client_config(extra.as_deref())
        })
        .clone()
}

fn build_client_config(extra_ca: Option<&Path>) -> Arc<rustls::ClientConfig> {
    ensure_tls_provider();
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let native = rustls_native_certs::load_native_certs();
    for error in &native.errors {
        log::debug!("Skipped part of the OS certificate store: {error}");
    }
    let (added, _) = roots.add_parsable_certificates(native.certs);
    log::debug!("Relay TLS trusts {added} certificates from the OS store");
    if let Some(path) = extra_ca {
        match load_certs(path) {
            Ok(certs) => {
                let (added, ignored) = roots.add_parsable_certificates(certs);
                log::info!(
                    "Relay TLS trusts {added} certificates from {} ({ignored} unusable)",
                    path.display()
                );
            }
            Err(e) => log::warn!("{CA_FILE_ENV}: {e}"),
        }
    }
    Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

/// The connector the relay client dials with
pub fn connector() -> tokio_tungstenite::Connector {
    tokio_tungstenite::Connector::Rustls(client_config())
}

/// Whether `url` is a plain `ws://` relay on another machine, so traffic
/// (cards, tasks, results) crosses the network unencrypted
pub fn is_unencrypted_remote(url: &str) -> bool {
    let Some(rest) = url.trim().strip_prefix("ws://") else {
        return false;
    };
    let authority = rest.split(['/', '?']).next().unwrap_or_default();
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => authority
            .rsplit_once(':')
            .map_or(authority, |(host, _)| host),
    };
    !(host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback()))
}

/// For tests: a client that also trusts `ca`
#[cfg(test)]
pub fn client_config_trusting(ca: &Path) -> Arc<rustls::ClientConfig> {
    build_client_config(Some(ca))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_ws_is_flagged_only_off_this_machine() {
        assert!(is_unencrypted_remote("ws://relay.example:8787/relay"));
        assert!(is_unencrypted_remote(
            "ws://192.168.1.10:8787/relay?token=x"
        ));
        assert!(!is_unencrypted_remote("ws://localhost:8787/relay"));
        assert!(!is_unencrypted_remote("ws://127.0.0.1:8787/relay"));
        assert!(!is_unencrypted_remote("ws://[::1]:8787/relay"));
        assert!(!is_unencrypted_remote("wss://relay.example/relay"));
        assert!(!is_unencrypted_remote(""));
    }
}
