//! Attic credentials and signing-key diagnostics without a live cache connection.

/// Unverified JWT claims and clock-based diagnostics, never authentication.
pub mod jwt {
    pub use harbor_cache::jwt::{
        JwtPayload, JwtStatus, decode_payload, format_duration, format_timestamp, status,
    };
}

/// Comparison of the live signing key with the declared keys.
#[derive(Debug, PartialEq, Eq)]
pub struct PublicKeyDrift<'a> {
    /// Whether the exact live key is present.
    pub live_declared: bool,
    /// Nonmatching keys with the same cache prefix.
    pub stale_same_cache: Vec<&'a str>,
    /// Nonmatching keys for other caches.
    pub unrelated: Vec<&'a str>,
}

impl<'a> PublicKeyDrift<'a> {
    /// Compare keys without querying Attic or interpreting unrelated keys as drift.
    pub fn compare(live_key: &str, declared: impl IntoIterator<Item = &'a str>) -> Self {
        let cache = live_key.split_once(':').map(|(cache, _)| cache);
        let mut drift = Self {
            live_declared: false,
            stale_same_cache: Vec::new(),
            unrelated: Vec::new(),
        };
        for key in declared {
            if key == live_key {
                drift.live_declared = true;
            } else if cache.is_none_or(|cache| {
                key.split_once(':')
                    .is_some_and(|(prefix, _)| prefix == cache)
            }) {
                drift.stale_same_cache.push(key);
            } else {
                drift.unrelated.push(key);
            }
        }
        drift
    }

    /// A missing live key takes precedence over stale-key warnings.
    pub fn status(&self) -> PublicKeyStatus {
        if !self.live_declared {
            PublicKeyStatus::Missing
        } else if !self.stale_same_cache.is_empty() {
            PublicKeyStatus::Stale
        } else {
            PublicKeyStatus::Clean
        }
    }
}

/// Result of comparing the declared keys to the live cache key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicKeyStatus {
    /// The live key is declared with no stale keys for this cache.
    Clean,
    /// The live key is declared, but stale keys for this cache remain.
    Stale,
    /// The live key is not declared.
    Missing,
}

/// Write an Attic token atomically with mode `0600`.
///
/// Preserve comments, other servers, the current default, and the existing
/// endpoint. Use `endpoint` when the server has no endpoint yet.
#[cfg(unix)]
pub fn attic_login_at(
    path: &std::path::Path,
    server: &str,
    endpoint: &str,
    token: &str,
) -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use std::{fs, io};
    use toml_edit::{DocumentMut, InlineTable, Item, Table, Value, value};

    ensure!(!server.trim().is_empty(), "Attic server name is empty");
    ensure!(
        !token.trim().is_empty(),
        "refusing to write an empty Attic token"
    );
    let raw = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    // A TOML parse error can quote the input, including credentials.
    let mut doc = raw
        .parse::<DocumentMut>()
        .map_err(|_| anyhow::anyhow!("invalid Attic TOML in {}", path.display()))?;
    if doc.get("default-server").is_none() {
        doc["default-server"] = value(server);
    }
    let servers = doc.entry("servers").or_insert_with(|| {
        let mut table = Table::new();
        table.set_implicit(true);
        Item::Table(table)
    });
    let inline = servers.is_inline_table();
    let servers = servers
        .as_table_like_mut()
        .context("Attic servers entry is not a table")?;
    let target = servers
        .entry(server)
        .or_insert_with(|| {
            if inline {
                Item::Value(Value::InlineTable(InlineTable::new()))
            } else {
                Item::Table(Table::new())
            }
        })
        .as_table_like_mut()
        .context("Attic server entry is not a table")?;
    if target.get("endpoint").is_none() {
        target.insert("endpoint", value(endpoint));
    }
    if let Some(item) = target.get_mut("token") {
        let old = item
            .as_value()
            .context("Attic token entry is not a value")?;
        let mut new = Value::from(token);
        *new.decor_mut() = old.decor().clone();
        *item = Item::Value(new);
    } else {
        target.insert("token", value(token));
    }
    nix_manager_core::fs::atomic_write_0600(path, doc.to_string().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::{jwt::*, *};
    use std::time::Duration;

    #[test]
    fn jwt_claims_are_only_clock_diagnostics() {
        let jwt = "header.eyJleHAiOjEyMCwibmJmIjo2MCwic3ViIjoiY2FjaGUiLCJodHRwczovL2p3dC5hdHRpYy5ycy92MSI6e319.not-a-signature";
        let claims = decode_payload(jwt).unwrap();
        let minute = Duration::from_secs(60);
        assert_eq!(claims.sub, "cache");
        assert_eq!(status(&claims, 0), JwtStatus::NotYetValid { in_: minute });
        assert_eq!(status(&claims, 60), JwtStatus::Valid { expires_in: minute });
        assert_eq!(status(&claims, 180), JwtStatus::Expired { ago: minute });
        assert_eq!(format_duration(minute), "1 min");
        assert_eq!(format_timestamp(claims.exp), "1970-01-01 00:02:00 UTC");
        assert!(decode_payload("malformed").is_err());
    }

    #[test]
    fn reports_missing_stale_and_unrelated_keys_separately() {
        use PublicKeyStatus::{Clean, Missing, Stale};
        assert_eq!(PublicKeyDrift::compare("x:new", ["x:new"]).status(), Clean);
        let drift = PublicKeyDrift::compare("x:new", ["x:old", "xy:other"]);
        assert_eq!(drift.status(), Missing);
        assert_eq!(drift.stale_same_cache, ["x:old"]);
        assert_eq!(drift.unrelated, ["xy:other"]);
        assert_eq!(
            PublicKeyDrift::compare("x:new", ["x:new", "x:old"]).status(),
            Stale
        );
        assert_eq!(
            PublicKeyDrift::compare("x:new", ["x:new", "xy:other"]).status(),
            Clean
        );
    }

    #[cfg(unix)]
    #[test]
    fn login_creates_a_private_file_and_edits_only_selected_credentials() {
        use std::{fs, os::unix::fs::PermissionsExt};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attic/config.toml");
        attic_login_at(&path, "first.cache", "https://first.example", "first").unwrap();
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains(r#"[servers."first.cache"]"#)
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let before = "# file comment\n\
            default-server = 'other'\n\
            [servers.other]\n\
            endpoint = 'https://other.example'\n\
            token = 'keep'\n\
            # selected server\n\
            [servers.local]\n\
            endpoint = 'https://local.example' # keep endpoint\n\
            token  =  'old' # keep token comment\n";
        fs::write(&path, before).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        attic_login_at(&path, "local", "https://unused.example", "new").unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert_eq!(after, before.replace("'old'", "\"new\""));
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(attic_login_at(&path, "local", "", " ").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
    }

    #[cfg(unix)]
    #[test]
    fn login_inserts_into_inline_servers_and_never_exposes_parse_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "servers = { existing = { token = 'keep' } } # note\n",
        )
        .unwrap();
        attic_login_at(&path, "local", "https://local.example", "new").unwrap();
        let edited = std::fs::read_to_string(&path).unwrap();
        let doc: toml_edit::DocumentMut = edited.parse().unwrap();
        assert_eq!(doc["servers"]["existing"]["token"].as_str(), Some("keep"));
        assert_eq!(doc["servers"]["local"]["token"].as_str(), Some("new"));
        assert!(edited.contains("# note"));
        let invalid = "token = 'private token\n";
        std::fs::write(&path, invalid).unwrap();
        let error = attic_login_at(&path, "local", "", "new").unwrap_err();
        assert!(!format!("{error:#}").contains("private token"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), invalid);
    }
}
