//! The NTP servers a clock asks, and how their names resolve.

use std::{
    collections::HashMap,
    fmt, io,
    net::{SocketAddr, ToSocketAddrs},
    panic::{self, AssertUnwindSafe},
    sync::{Arc, Condvar, Mutex, PoisonError},
    thread,
    time::Instant,
};

use rustls::{ClientConfig, RootCertStore, pki_types::CertificateDer};

use crate::{ClockError, NtsError, nts};

/// Turns a server's name into the addresses to try, in order. A clock
/// resolves each name again on every sync, so a pool's addresses can
/// change.
pub trait Resolve: Send + Sync {
    fn resolve(&self, server: &str) -> io::Result<Vec<SocketAddr>>;
}

impl<F: Fn(&str) -> io::Result<Vec<SocketAddr>> + Send + Sync> Resolve for F {
    fn resolve(&self, server: &str) -> io::Result<Vec<SocketAddr>> {
        self(server)
    }
}

/// The system's resolver: every address `host:port` resolves to.
#[derive(Clone, Copy, Debug)]
struct Dns;

impl Resolve for Dns {
    fn resolve(&self, server: &str) -> io::Result<Vec<SocketAddr>> {
        let addresses: Vec<_> = server.to_socket_addrs()?.collect();
        if addresses.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{server} has no address"),
            ));
        }
        Ok(addresses)
    }
}

/// What a lookup found: the addresses, or the error's kind and message,
/// since an [`io::Error`] can't be cloned for every caller waiting on it.
type Found = Result<Vec<SocketAddr>, (io::ErrorKind, String)>;

/// One lookup on a thread of its own, and what it found once it finishes.
#[derive(Default)]
struct Lookup {
    found: Mutex<Option<Found>>,
    finished: Condvar,
}

/// A resolver, and the lookups it is running. A system lookup can't be
/// interrupted, so each runs on a thread of its own, which finishes alone
/// if the caller's deadline passes first. A name has one lookup at a time:
/// a caller that finds one running waits on it rather than starting
/// another, so a resolver that hangs holds one thread per name instead of
/// one per name each sync.
#[derive(Clone)]
pub(crate) struct Lookups {
    pub(crate) resolver: Arc<dyn Resolve>,
    running: Arc<Mutex<HashMap<String, Arc<Lookup>>>>,
}

impl Lookups {
    pub(crate) fn new(resolver: impl Resolve + 'static) -> Self {
        Self {
            resolver: Arc::new(resolver),
            running: Arc::default(),
        }
    }

    /// Resolves `server`, giving up at `deadline`.
    pub(crate) fn resolve_by(
        &self,
        server: &str,
        deadline: Instant,
    ) -> Result<Vec<SocketAddr>, ClockError> {
        let lookup = self.start(server);
        let found = lookup.found.lock().unwrap_or_else(PoisonError::into_inner);
        let left = deadline.saturating_duration_since(Instant::now());
        let (found, _) = lookup
            .finished
            .wait_timeout_while(found, left, |found| found.is_none())
            .unwrap_or_else(PoisonError::into_inner);
        match &*found {
            Some(Ok(addresses)) => Ok(addresses.clone()),
            Some(Err((kind, message))) => Err(io::Error::new(*kind, message.clone()).into()),
            None => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("resolving {server} took too long"),
            )
            .into()),
        }
    }

    /// The lookup of `server` already running, or a new one.
    fn start(&self, server: &str) -> Arc<Lookup> {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(lookup) = running.get(server) {
            return lookup.clone();
        }
        let lookup = Arc::new(Lookup::default());
        running.insert(server.to_owned(), lookup.clone());
        let (resolver, all, name, mine) = (
            self.resolver.clone(),
            self.running.clone(),
            server.to_owned(),
            lookup.clone(),
        );
        thread::spawn(move || {
            // A resolver that panics still finishes its lookup, or the name
            // would wait on it forever.
            let found = panic::catch_unwind(AssertUnwindSafe(|| resolver.resolve(&name)))
                .unwrap_or_else(|_| Err(io::Error::other("the resolver panicked")))
                .map_err(|failed| (failed.kind(), failed.to_string()));
            *mine.found.lock().unwrap_or_else(PoisonError::into_inner) = Some(found);
            mine.finished.notify_all();
            // The next caller starts afresh, so a pool's addresses can change.
            // Only this thread removes the entry it was started for, and no
            // other is added while it's there.
            all.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&name);
        });
        lookup
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
    pub(crate) lookups: Lookups,
}

impl Servers {
    /// Plain NTP servers, resolved through the system's resolver.
    pub fn new<S: ToString>(names: impl IntoIterator<Item = S>) -> Self {
        let names: Vec<String> = names.into_iter().map(|name| name.to_string()).collect();
        Self {
            tls: vec![None; names.len()],
            names,
            lookups: Lookups::new(Dns),
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
            .map_err(NtsError::Certificate)?;
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
            lookups: Lookups::new(resolver),
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
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        time::Duration,
    };

    use super::*;

    #[test]
    fn a_name_resolves_to_its_addresses() {
        let addresses = Dns.resolve("127.0.0.1:123").unwrap();
        assert_eq!(addresses, ["127.0.0.1:123".parse().unwrap()]);
        assert!(Dns.resolve("no port").is_err());
    }

    fn timed_out(resolved: Result<Vec<SocketAddr>, ClockError>) -> bool {
        matches!(resolved, Err(ClockError::Io(failed)) if failed.kind() == io::ErrorKind::TimedOut)
    }

    #[test]
    fn a_slow_resolver_is_given_up_at_the_deadline() {
        let slow = Lookups::new(|_: &str| {
            thread::sleep(Duration::from_secs(2));
            Ok(vec![])
        });
        let started = Instant::now();
        let deadline = started + Duration::from_millis(100);
        assert!(timed_out(slow.resolve_by("slow.test:123", deadline)));
        assert!(started.elapsed() < Duration::from_secs(1));
        let fine = Lookups::new(Dns);
        let later = Instant::now() + Duration::from_secs(5);
        assert_eq!(fine.resolve_by("127.0.0.1:9", later).unwrap().len(), 1);
        assert!(matches!(
            fine.resolve_by("no port", later),
            Err(ClockError::Io(_))
        ));
    }

    #[test]
    fn a_name_has_one_lookup_at_a_time() {
        let address: SocketAddr = "10.0.0.1:123".parse().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        // Every lookup hangs until `release` is dropped.
        let (release, released) = mpsc::channel::<()>();
        let released = Mutex::new(released);
        let counted = calls.clone();
        let hung = Lookups::new(move |_: &str| {
            counted.fetch_add(1, Ordering::SeqCst);
            let _ = released.lock().unwrap().recv();
            Ok(vec![address])
        });
        let soon = || Instant::now() + Duration::from_millis(20);
        for _ in 0..3 {
            assert!(timed_out(hung.resolve_by("hung.test:123", soon())));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Another name has a lookup of its own.
        assert!(timed_out(hung.resolve_by("other.test:123", soon())));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        // A caller waiting on the running lookup gets what it finds.
        drop(release);
        let later = Instant::now() + Duration::from_secs(5);
        assert_eq!(hung.resolve_by("hung.test:123", later).unwrap(), [address]);
    }

    #[test]
    fn a_finished_lookup_makes_way_for_a_fresh_one() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let lookups = Lookups::new(move |_: &str| {
            if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("the first lookup");
            }
            Ok(vec!["10.0.0.2:123".parse().unwrap()])
        });
        let later = || Instant::now() + Duration::from_secs(5);
        let panicked = lookups.resolve_by("a.test:123", later());
        assert!(matches!(panicked, Err(ClockError::Io(failed)) if failed.to_string() == "the resolver panicked"));
        // The panicked lookup leaves the running set once it has finished.
        let gone = Instant::now() + Duration::from_secs(5);
        while !lookups.running.lock().unwrap().is_empty() && Instant::now() < gone {
            thread::yield_now();
        }
        assert!(lookups.running.lock().unwrap().is_empty());
        assert_eq!(lookups.resolve_by("a.test:123", later()).unwrap().len(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_resolver_can_be_injected() {
        let elsewhere: SocketAddr = "10.0.0.1:123".parse().unwrap();
        let servers =
            Servers::new(["a.test:123"]).resolving_with(move |_: &str| Ok(vec![elsewhere]));
        assert_eq!(servers.lookups.resolver.resolve("a.test:123").unwrap(), [elsewhere]);
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
