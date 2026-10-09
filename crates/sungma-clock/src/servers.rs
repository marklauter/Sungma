//! The NTP servers a clock asks, and how their names resolve.

use std::{
    fmt, io,
    net::{SocketAddr, ToSocketAddrs},
    sync::Arc,
};

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

/// The servers a clock asks, by name, such as `time.cloudflare.com:123`,
/// and the resolver that finds them. A deployment names at least four,
/// from independent operators, none of which smears leap seconds.
#[derive(Clone)]
pub struct Servers {
    pub(crate) names: Vec<String>,
    pub(crate) resolver: Arc<dyn Resolve>,
}

impl Servers {
    /// Servers resolved through the system's resolver.
    pub fn new<S: ToString>(names: impl IntoIterator<Item = S>) -> Self {
        Self {
            names: names.into_iter().map(|name| name.to_string()).collect(),
            resolver: Arc::new(Dns),
        }
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
        f.debug_tuple("Servers").field(&self.names).finish()
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
        assert_eq!(format!("{servers:?}"), r#"Servers(["a.test:123"])"#);
    }
}
