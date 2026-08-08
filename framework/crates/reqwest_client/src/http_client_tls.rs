use std::sync::OnceLock;

use rustls::ClientConfig;
use rustls_platform_verifier::ConfigVerifierExt;

static TLS_CONFIG: OnceLock<rustls::ClientConfig> = OnceLock::new();

pub fn tls_config() -> ClientConfig {
    TLS_CONFIG
        .get_or_init(|| {
            // rustls uses the `aws_lc_rs` provider by default
            // This only errors if another provider has already been installed.
            if let Err(err) = rustls::crypto::aws_lc_rs::default_provider().install_default() {
                log::debug!("failed to install default rustls provider: {err:?}");
            }

            ClientConfig::with_platform_verifier()
        })
        .clone()
}
