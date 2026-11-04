use std::time::Duration;

use sungma_bff::Config;

/// How Author runs, read from its environment:
///
/// - `SUNGMA_AUTHOR_ISSUER`, `SUNGMA_AUTHOR_CLIENT_ID` and
///   `SUNGMA_AUTHOR_CLIENT_SECRET`: the identity provider and the client
///   it registered for Author. Required.
/// - `SUNGMA_AUTHOR_ADDR`: where Author listens, `127.0.0.1:8090` by
///   default.
/// - `SUNGMA_AUTHOR_ORIGIN`: Author's origin as the browser sees it,
///   `http://localhost:8090` by default.
/// - `SUNGMA_AUTHOR_PROVIDER`: the identity provider's name on the
///   sign-in button, `your organization` by default.
pub struct Settings {
    pub addr: String,
    pub provider: String,
    pub bff: Config,
}

/// A required setting that isn't set.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0} isn't set")]
pub struct Missing(pub &'static str);

impl Settings {
    /// The settings from `var`, which looks up an environment variable.
    pub fn from(var: impl Fn(&str) -> Option<String>) -> Result<Self, Missing> {
        let required = |name: &'static str| var(name).ok_or(Missing(name));
        let or = |name: &str, default: &str| var(name).unwrap_or_else(|| default.to_owned());
        Ok(Self {
            addr: or("SUNGMA_AUTHOR_ADDR", "127.0.0.1:8090"),
            provider: or("SUNGMA_AUTHOR_PROVIDER", "your organization"),
            bff: Config {
                issuer: required("SUNGMA_AUTHOR_ISSUER")?,
                client_id: required("SUNGMA_AUTHOR_CLIENT_ID")?,
                client_secret: required("SUNGMA_AUTHOR_CLIENT_SECRET")?,
                origin: or("SUNGMA_AUTHOR_ORIGIN", "http://localhost:8090"),
                scopes: vec!["profile".to_owned(), "email".to_owned()],
                sign_in_page: "/signin".to_owned(),
                timeout: Duration::from_secs(10),
                idle: Duration::from_secs(60 * 60),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn from(vars: &[(&str, &str)]) -> Result<Settings, Missing> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        Settings::from(|name| vars.get(name).cloned())
    }

    const REQUIRED: [(&str, &str); 3] = [
        ("SUNGMA_AUTHOR_ISSUER", "http://localhost:8080/auth/v1"),
        ("SUNGMA_AUTHOR_CLIENT_ID", "author"),
        ("SUNGMA_AUTHOR_CLIENT_SECRET", "secret"),
    ];

    #[test]
    fn the_required_settings_alone_take_the_defaults() {
        let settings = from(&REQUIRED).unwrap();
        assert_eq!(settings.addr, "127.0.0.1:8090");
        assert_eq!(settings.provider, "your organization");
        assert_eq!(settings.bff.issuer, "http://localhost:8080/auth/v1");
        assert_eq!(settings.bff.client_id, "author");
        assert_eq!(settings.bff.client_secret, "secret");
        assert_eq!(settings.bff.origin, "http://localhost:8090");
        assert_eq!(settings.bff.scopes, ["profile", "email"]);
        assert_eq!(settings.bff.sign_in_page, "/signin");
        assert_eq!(settings.bff.timeout, Duration::from_secs(10));
        assert_eq!(settings.bff.idle, Duration::from_secs(3600));
    }

    #[test]
    fn set_settings_replace_the_defaults() {
        let mut vars = REQUIRED.to_vec();
        vars.extend([
            ("SUNGMA_AUTHOR_ADDR", "0.0.0.0:80"),
            ("SUNGMA_AUTHOR_ORIGIN", "https://author.example.com"),
            ("SUNGMA_AUTHOR_PROVIDER", "Okta"),
        ]);
        let settings = from(&vars).unwrap();
        assert_eq!(settings.addr, "0.0.0.0:80");
        assert_eq!(settings.bff.origin, "https://author.example.com");
        assert_eq!(settings.provider, "Okta");
    }

    #[test]
    fn each_required_setting_must_be_set() {
        for missing in REQUIRED.map(|(name, _)| name) {
            let vars: Vec<_> = REQUIRED
                .into_iter()
                .filter(|(name, _)| *name != missing)
                .collect();
            assert_eq!(from(&vars).err(), Some(Missing(missing)));
        }
    }
}
