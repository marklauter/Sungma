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
    /// Each server's TLS settings for NTS key exchange, or `None` for a
    /// plain NTP server. Lists joined with [`Servers::and`] keep their own.
    pub(crate) tls: Vec<Option<Arc<ClientConfig>>>,
    pub(crate) resolver: Arc<dyn Resolve>,
}

impl Servers {
    /// Plain NTP servers, resolved through the system's resolver.
    pub fn new<S: ToString>(names: impl IntoIterator<Item = S>) -> Self {
        let names: Vec<String> = names.into_iter().map(|name| name.to_string()).collect();
        Self {
            tls: vec![None; names.len()],
            names,
            resolver: Arc::new(Dns),
        }
    }

    /// NTS servers, by host, whose certificates chain to Mozilla's roots.
    pub fn nts<S: ToString>(hosts: impl IntoIterator<Item = S>) -> Self {
        let mut servers = Self::new(hosts);
        servers.tls.fill(Some(nts::tls(nts::public_roots())));
        servers
    }

    /// These servers and `others`, resolved through this list's resolver.
    #[must_use]
    pub fn and(mut self, others: Self) -> Self {
        self.names.extend(others.names);
        self.tls.extend(others.tls);
        self
    }

    /// The same servers, their NTS servers trusting only `certificate`,
    /// DER-encoded: a private deployment's own certificate authority. Join
    /// other lists after this, with [`Servers::and`], to keep their trust.
    pub fn trusting_only(mut self, certificate: &[u8]) -> Result<Self, ClockError> {
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from(certificate.to_vec()))
            .map_err(|failed| ClockError::Nts(failed.to_string()))?;
        let pinned = nts::tls(roots);
        for tls in self.tls.iter_mut().flatten() {
            *tls = pinned.clone();
        }
        Ok(self)
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
        let nts: Vec<bool> = self.tls.iter().map(Option::is_some).collect();
        f.debug_struct("Servers")
            .field("names", &self.names)
            .field("nts", &nts)
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
    fn each_list_keeps_its_own_trust_when_joined() {
        let certificate = rcgen::generate_simple_self_signed(vec!["p.test".to_owned()])
            .unwrap()
            .cert
            .der()
            .to_vec();
        let private = Servers::nts(["p.test"])
            .trusting_only(&certificate)
            .unwrap();
        let servers = Servers::new(["a:123"])
            .and(private)
            .and(Servers::nts(["q.test"]));
        let tls: Vec<_> = servers.tls.iter().map(Option::as_ref).collect();
        assert!(tls[0].is_none());
        let (pinned, public) = (tls[1].unwrap(), tls[2].unwrap());
        assert!(!Arc::ptr_eq(pinned, public));
        // Pinning a joined list pins its NTS servers and leaves plain ones.
        let pinned = Servers::new(["a:123"])
            .and(Servers::nts(["q.test"]))
            .trusting_only(&certificate)
            .unwrap();
        assert!(pinned.tls[0].is_none());
        assert!(!Arc::ptr_eq(pinned.tls[1].as_ref().unwrap(), public));
    }

    #[test]
    fn nts_servers_join_plain_ones_and_bring_their_tls() {
        let servers = Servers::new(["a:123"]).and(Servers::nts(["b", "c"]));
        assert_eq!(servers.names, ["a:123", "b", "c"]);
        let nts: Vec<bool> = servers.tls.iter().map(Option::is_some).collect();
        assert_eq!(nts, [false, true, true]);
        assert!(Servers::new(["a:123"]).tls.iter().all(Option::is_none));
        assert!(
            Servers::nts(["b"])
                .trusting_only(b"not a certificate")
                .is_err()
        );
    }
}
