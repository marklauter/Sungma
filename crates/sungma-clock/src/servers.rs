//! The NTP servers a clock asks, and how their names resolve.

use std::{
    fmt, io,
    net::{SocketAddr, ToSocketAddrs},
    sync::Arc,
};

use rustls::{ClientConfig, RootCertStore, pki_types::CertificateDer};

use crate::{ClockError, nts};

/// Turns a server's name into the address to ask. A clock resolves each
/// name again on every sync, so a pool's addresses can change.
pub trait Resolve: Send + Sync {
    fn resolve(&self, server: &str) -> io::Result<SocketAddr>;
}

impl<F: Fn(&str) -> io::Result<SocketAddr> + Send + Sync> Resolve for F {
    fn resolve(&self, server: &str) -> io::Result<SocketAddr> {
        self(server)
    }
}

/// The system's resolver: the first address `host:port` resolves to.
#[derive(Clone, Copy, Debug)]
struct Dns;

impl Resolve for Dns {
    fn resolve(&self, server: &str) -> io::Result<SocketAddr> {
        server.to_socket_addrs()?.next().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("{server} has no address"))
        })
    }
}

/// The servers a clock asks, and the resolver that finds them. A plain
/// NTP server is named `host:port`, such as `time.example.com:123`; an NTS
/// server by its host alone, such as `time.cloudflare.com`. A deployment
/// names at least four, from independent operators, none of which smears
/// leap seconds.
#[derive(Clone)]
pub struct Servers {
    pub(crate) names: Vec<String>,
    /// Whether each server is asked over NTS.
    pub(crate) nts: Vec<bool>,
    pub(crate) resolver: Arc<dyn Resolve>,
    /// TLS settings for NTS key exchange, when there are NTS servers.
    pub(crate) tls: Option<Arc<ClientConfig>>,
}

impl Servers {
    /// Plain NTP servers, resolved through the system's resolver.
    pub fn new<S: ToString>(names: impl IntoIterator<Item = S>) -> Self {
        let names: Vec<String> = names.into_iter().map(|name| name.to_string()).collect();
        Self {
            nts: vec![false; names.len()],
            names,
            resolver: Arc::new(Dns),
            tls: None,
        }
    }

    /// NTS servers, by host, whose certificates chain to Mozilla's roots.
    pub fn nts<S: ToString>(hosts: impl IntoIterator<Item = S>) -> Self {
        let mut servers = Self::new(hosts);
        servers.nts.fill(true);
        servers.tls = Some(nts::tls(nts::public_roots()));
        servers
    }

    /// These servers and `others`, resolved through this list's resolver.
    #[must_use]
    pub fn and(mut self, others: Self) -> Self {
        self.names.extend(others.names);
        self.nts.extend(others.nts);
        self.tls = self.tls.or(others.tls);
        self
    }

    /// The same servers, trusting only `certificate`, DER-encoded, for NTS
    /// key exchange: a private deployment's own certificate authority.
    pub fn trusting_only(self, certificate: &[u8]) -> Result<Self, ClockError> {
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from(certificate.to_vec()))
            .map_err(|failed| ClockError::Nts(failed.to_string()))?;
        Ok(Self {
            tls: Some(nts::tls(roots)),
            ..self
        })
    }

    /// The same servers, resolved through `resolver`.
    #[must_use]
    pub fn resolving_with(self, resolver: impl Resolve + 'static) -> Self {
        Self {
            resolver: Arc::new(resolver),
            ..self
        }
    }
}

impl fmt::Debug for Servers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Servers")
            .field("names", &self.names)
            .field("nts", &self.nts)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_resolves_to_its_first_address() {
        let address = Dns.resolve("127.0.0.1:123").unwrap();
        assert_eq!(address, "127.0.0.1:123".parse().unwrap());
        assert!(Dns.resolve("no port").is_err());
    }

    #[test]
    fn a_resolver_can_be_injected() {
        let elsewhere: SocketAddr = "10.0.0.1:123".parse().unwrap();
        let servers = Servers::new(["a.test:123"]).resolving_with(move |_: &str| Ok(elsewhere));
        assert_eq!(servers.resolver.resolve("a.test:123").unwrap(), elsewhere);
        assert_eq!(
            format!("{servers:?}"),
            r#"Servers { names: ["a.test:123"], nts: [false], .. }"#
        );
    }

    #[test]
    fn nts_servers_join_plain_ones_and_bring_their_tls() {
        let servers = Servers::new(["a:123"]).and(Servers::nts(["b", "c"]));
        assert_eq!(servers.names, ["a:123", "b", "c"]);
        assert_eq!(servers.nts, [false, true, true]);
        assert!(servers.tls.is_some());
        assert!(Servers::new(["a:123"]).tls.is_none());
        assert!(
            Servers::nts(["b"])
                .trusting_only(b"not a certificate")
                .is_err()
        );
    }
}
