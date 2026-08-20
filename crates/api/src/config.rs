//! Configuration read from the environment.
//!
//! Everything the process needs to start is gathered here, once, at boot: no
//! handler reads an environment variable of its own. [`Config::from_source`]
//! takes the lookup function as an argument so the whole parsing can be tested
//! without mutating the environment of the test process, which is shared by
//! every test running in parallel.

use std::fmt;
use std::net::SocketAddr;

/// Connection string of the database. Shared with the `db` crate, which reads
/// the same variable for its migration tool.
pub const DATABASE_URL_ENV: &str = db::DATABASE_URL_ENV;

/// Address the HTTP server binds to.
pub const ADDR_ENV: &str = "ALCOLOCO_API_ADDR";

/// Log level, in the `tracing-subscriber` filter syntax (`info`, `api=debug`…).
pub const LOG_LEVEL_ENV: &str = "ALCOLOCO_LOG_LEVEL";

/// Deployment environment, which decides whether the OpenAPI document is served.
pub const ENVIRONMENT_ENV: &str = "ALCOLOCO_ENV";

/// Address used when [`ADDR_ENV`] is not set.
pub const DEFAULT_ADDR: &str = "0.0.0.0:8080";

/// Log level used when [`LOG_LEVEL_ENV`] is not set.
pub const DEFAULT_LOG_LEVEL: &str = "info";

/// Where the process believes it is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    /// Local development: the OpenAPI document is served.
    Development,
    /// Anything user facing: the OpenAPI document is not served.
    Production,
}

impl Environment {
    /// Whether the OpenAPI document is exposed over HTTP.
    ///
    /// The issue asks for a specification "exposed in dev": the document
    /// describes routes that do not all exist publicly, so it stays off outside
    /// development rather than being merely undocumented.
    #[must_use]
    pub const fn exposes_openapi(self) -> bool {
        matches!(self, Self::Development)
    }
}

impl fmt::Display for Environment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Development => "development",
            Self::Production => "production",
        })
    }
}

/// Everything the process needs to start.
#[derive(Debug, Clone)]
pub struct Config {
    /// PostgreSQL connection string.
    pub database_url: String,
    /// Socket the HTTP server binds to.
    pub addr: SocketAddr,
    /// Filter handed to `tracing-subscriber`.
    pub log_level: String,
    /// Deployment environment.
    pub environment: Environment,
}

impl Config {
    /// Reads the configuration from the process environment.
    ///
    /// # Errors
    ///
    /// Fails when a variable is set to a value that cannot be parsed. A missing
    /// variable is never an error: every setting has a documented default.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_source(|key| std::env::var(key).ok())
    }

    /// Reads the configuration from an arbitrary lookup function.
    ///
    /// An empty value is treated as an absent one: a shell exporting
    /// `DATABASE_URL=` should fall back to the default rather than try to
    /// connect to the empty string.
    ///
    /// # Errors
    ///
    /// Fails on an unparsable address or an unknown environment name.
    pub fn from_source<F>(get: F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let read = |key: &str| get(key).filter(|value| !value.trim().is_empty());

        let addr_text = read(ADDR_ENV).unwrap_or_else(|| DEFAULT_ADDR.to_owned());
        let addr = addr_text
            .parse()
            .map_err(|_| ConfigError::Invalid(ADDR_ENV, addr_text))?;

        let environment = match read(ENVIRONMENT_ENV) {
            None => Environment::Development,
            Some(value) => match value.trim() {
                "development" | "dev" => Environment::Development,
                "production" | "prod" => Environment::Production,
                _ => return Err(ConfigError::Invalid(ENVIRONMENT_ENV, value)),
            },
        };

        Ok(Self {
            database_url: read(DATABASE_URL_ENV)
                .unwrap_or_else(|| db::DEFAULT_DATABASE_URL.to_owned()),
            addr,
            log_level: read(LOG_LEVEL_ENV).unwrap_or_else(|| DEFAULT_LOG_LEVEL.to_owned()),
            environment,
        })
    }
}

/// A variable is set to a value that cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// `$0` holds the unusable value `$1`.
    Invalid(&'static str, String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(key, value) => write!(formatter, "{key}: invalid value `{value}`"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn from(pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        Config::from_source(move |key| {
            owned
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        })
    }

    #[test]
    fn an_empty_environment_yields_the_documented_defaults() {
        let config = from(&[]).expect("defaults must parse");
        assert_eq!(config.database_url, db::DEFAULT_DATABASE_URL);
        assert_eq!(config.addr.to_string(), DEFAULT_ADDR);
        assert_eq!(config.log_level, DEFAULT_LOG_LEVEL);
        assert_eq!(config.environment, Environment::Development);
    }

    #[test]
    fn every_setting_is_read_from_its_own_variable() {
        // Each variable is given a value distinguishable from the default, so a
        // field left wired to its default cannot pass this test.
        let config = from(&[
            (DATABASE_URL_ENV, "postgres://u:p@db.internal:6543/alcoloco"),
            (ADDR_ENV, "127.0.0.1:9111"),
            (LOG_LEVEL_ENV, "api=debug"),
            (ENVIRONMENT_ENV, "production"),
        ])
        .expect("a fully specified environment must parse");

        assert_eq!(
            config.database_url,
            "postgres://u:p@db.internal:6543/alcoloco"
        );
        assert_eq!(config.addr.to_string(), "127.0.0.1:9111");
        assert_eq!(config.log_level, "api=debug");
        assert_eq!(config.environment, Environment::Production);
    }

    #[test]
    fn an_empty_value_falls_back_to_the_default() {
        let config = from(&[(DATABASE_URL_ENV, ""), (ADDR_ENV, "  ")]).expect("must parse");
        assert_eq!(config.database_url, db::DEFAULT_DATABASE_URL);
        assert_eq!(config.addr.to_string(), DEFAULT_ADDR);
    }

    #[test]
    fn an_unparsable_address_is_reported_rather_than_ignored() {
        // Silently falling back to 0.0.0.0:8080 would publish a service the
        // operator asked to keep on the loopback.
        let error = from(&[(ADDR_ENV, "not an address")]).expect_err("must fail");
        assert_eq!(
            error,
            ConfigError::Invalid(ADDR_ENV, "not an address".to_owned())
        );
    }

    #[test]
    fn an_unknown_environment_name_is_rejected() {
        // A typo must not silently downgrade production into development and
        // publish the OpenAPI document there.
        let error = from(&[(ENVIRONMENT_ENV, "staging")]).expect_err("must fail");
        assert_eq!(
            error,
            ConfigError::Invalid(ENVIRONMENT_ENV, "staging".to_owned())
        );
    }

    #[test]
    fn only_development_exposes_the_openapi_document() {
        assert!(Environment::Development.exposes_openapi());
        assert!(!Environment::Production.exposes_openapi());
    }
}
