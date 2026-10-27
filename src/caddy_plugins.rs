//! Caddy plugin version and `pluginsHash` refresh for Nix flakes.
//!
//! The consumer flake declares `caddy.plugins = [ "github.com/OWNER/REPO@VERSION" ]`
//! and `caddy.pluginsHash = "sha256-..."` in one Nix file and exposes the
//! `caddy-helpers` flake module's `legacyPackages.<system>.caddyPluginsVendorFor`.
//! A refresh forces the vendor fixed-output derivation with a fake hash, reads
//! the `got:` hash, realizes the vendor output with it and edits the source.
//!
//! Realization is supplied through [`Nix`], so consumers keep their own
//! evaluation leases, build bounds and GC roots. Publication stays with the
//! caller: [`Resolved::store_path`] is the realized vendor output to upload.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Boxed error, convertible into `anyhow`/`miette` reports.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Nixpkgs' `lib.fakeHash`, forcing the vendor derivation to report its hash.
pub const FAKE_HASH: &str = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const PLUGINS_MARKER: &str = "caddy.plugins";
const HASH_MARKER: &str = "caddy.pluginsHash";

/// One `module@version` occurrence in a Nix file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plugin {
    /// File containing the occurrence.
    pub file: PathBuf,
    /// Go module path, for example `github.com/caddy-dns/cloudflare`.
    pub module: String,
    /// Go module version, for example `v0.2.4`.
    pub version: String,
}

/// A resolved `pluginsHash`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// SRI hash of the vendor fixed-output derivation.
    pub hash: String,
    /// Realized vendor output for the caller to publish.
    pub store_path: String,
}

/// Realization backend for the vendor derivation.
pub trait Nix {
    /// Nix system of the build platform, for example `x86_64-linux`.
    fn system(&mut self) -> Result<String>;
    /// Build an impure expression expected to fail its hash check; return stderr.
    fn probe(&mut self, expr: &str) -> Result<String>;
    /// Build an impure expression and return its `out` store path.
    fn realize(&mut self, expr: &str) -> Result<String>;
}

/// [`Nix`] backend running the `nix` CLI with `--impure` and `--no-link`.
#[derive(Debug, Default)]
pub struct NixCommand;

impl NixCommand {
    fn build(expr: &str, extra: &[&str]) -> Result<std::process::Output> {
        Ok(Command::new("nix")
            .args(["build", "--impure", "--no-link"])
            .args(extra)
            .args(["--expr", expr])
            .output()?)
    }
}

impl Nix for NixCommand {
    fn system(&mut self) -> Result<String> {
        let output = Command::new("nix")
            .args([
                "eval",
                "--impure",
                "--raw",
                "--expr",
                "builtins.currentSystem",
            ])
            .output()?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
        }
        Ok(String::from_utf8(output.stdout)?)
    }

    fn probe(&mut self, expr: &str) -> Result<String> {
        Ok(String::from_utf8_lossy(&Self::build(expr, &[])?.stderr).into_owned())
    }

    fn realize(&mut self, expr: &str) -> Result<String> {
        let output = Self::build(expr, &["--print-out-paths"])?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
        }
        let stdout = String::from_utf8(output.stdout)?;
        Ok(stdout
            .lines()
            .last()
            .ok_or("nix build printed no output path")?
            .to_string())
    }
}

/// `source` with Nix comments blanked to spaces at the same byte offsets;
/// string literals are kept.
fn mask_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let blank = |range: Range<usize>, out: &mut Vec<u8>| {
        for byte in &mut out[range] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    };
    let mut i = 0;
    while i < bytes.len() {
        let rest = &bytes[i..];
        if rest[0] == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
        } else if rest.starts_with(b"''") {
            i += 2;
            while i < bytes.len() {
                if bytes[i..].starts_with(b"''") {
                    if matches!(bytes.get(i + 2), Some(b'$' | b'\'' | b'\\')) {
                        i += 3;
                        continue;
                    }
                    i += 2;
                    break;
                }
                i += 1;
            }
        } else if rest[0] == b'#' {
            let end = rest
                .iter()
                .position(|b| *b == b'\n')
                .map_or(bytes.len(), |n| i + n);
            blank(i..end, &mut out);
            i = end;
        } else if rest.starts_with(b"/*") {
            let end = rest[2..]
                .windows(2)
                .position(|w| w == b"*/")
                .map_or(bytes.len(), |n| i + 2 + n + 2);
            blank(i..end, &mut out);
            i = end;
        } else {
            i += 1;
        }
    }
    // Whole characters are blanked, so the result stays valid UTF-8.
    String::from_utf8(out).expect("comment masking preserves UTF-8")
}

/// Byte ranges of the values delimited by `open`/`close` in `name = <open>...<close>`
/// assignments outside comments, tolerating any whitespace around `=`.
fn assignment_values(source: &str, name: &str, open: char, close: char) -> Vec<Range<usize>> {
    let source = &mask_comments(source);
    source
        .match_indices(name)
        .filter_map(|(start, _)| {
            let after = start + name.len();
            let rest = source[after..].trim_start().strip_prefix('=')?;
            let value = rest.trim_start().strip_prefix(open)?;
            let begin = source.len() - value.len();
            Some(begin..begin + value.find(close)?)
        })
        .collect()
}

/// Extract `"github.com/...@version"` literals from `caddy.plugins = [ ... ]` lists.
pub fn extract_plugins(source: &str, file: &Path) -> Vec<Plugin> {
    let source = &mask_comments(source);
    assignment_values(source, PLUGINS_MARKER, '[', ']')
        .into_iter()
        .flat_map(|range| {
            let list = &source[range];
            list.match_indices("\"github.com/")
                .filter_map(move |(start, _)| {
                    let literal = &list[start + 1..];
                    let (module, version) = literal[..literal.find('"')?].split_once('@')?;
                    (!version.is_empty()).then(|| Plugin {
                        file: file.to_path_buf(),
                        module: module.to_string(),
                        version: version.to_string(),
                    })
                })
        })
        .collect()
}

/// Replace complete `"module@old"` literals with `"module@new"`.
pub fn bump_version(source: &str, module: &str, old: &str, new: &str) -> String {
    source.replace(
        &format!("\"{module}@{old}\""),
        &format!("\"{module}@{new}\""),
    )
}

/// Replace every `caddy.pluginsHash = "..."` value with `hash`; also returns
/// how many assignments were replaced.
pub fn set_plugins_hash(source: &str, hash: &str) -> (String, usize) {
    let ranges = assignment_values(source, HASH_MARKER, '"', '"');
    let mut out = String::with_capacity(source.len());
    let mut copied = 0;
    for range in &ranges {
        out.push_str(&source[copied..range.start]);
        out.push_str(hash);
        copied = range.end;
    }
    out.push_str(&source[copied..]);
    (out, ranges.len())
}

/// Every `got: sha256-...` hash reported by a failed fixed-output build.
pub fn extract_got_hashes(stderr: &str) -> Vec<String> {
    stderr
        .split("got:")
        .skip(1)
        .filter_map(|tail| {
            let hash = tail.trim_start().strip_prefix("sha256-")?;
            let len = hash
                .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=')))
                .unwrap_or(hash.len());
            (len > 0).then(|| format!("sha256-{}", &hash[..len]))
        })
        .collect()
}

/// Expression for the flake's vendor derivation; `None` forces [`FAKE_HASH`].
pub fn vendor_expr(flake: &Path, system: &str, plugins: &[Plugin], hash: Option<&str>) -> String {
    let plugins = plugins
        .iter()
        .map(|p| format!("\"{}@{}\"", p.module, p.version))
        .collect::<Vec<_>>()
        .join(" ");
    let hash = hash.unwrap_or(FAKE_HASH);
    format!(
        "((builtins.getFlake {flake}).legacyPackages.{system}.caddyPluginsVendorFor [ {plugins} ] \"{hash}\")",
        flake = nix_string(&flake.display().to_string()),
    )
}

fn nix_string(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace("${", "\\${");
    format!("\"{escaped}\"")
}

/// Sorted, deduplicated `module@version` identity of a plugin set.
pub fn fingerprint(plugins: &[Plugin]) -> String {
    plugins
        .iter()
        .map(|p| format!("{}@{}", p.module, p.version))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join("\n")
}

/// Group occurrences by module, preserving first-seen module order.
pub fn group_by_module(plugins: &[Plugin]) -> Vec<(&str, Vec<&Plugin>)> {
    let mut groups: Vec<(&str, Vec<&Plugin>)> = Vec::new();
    for plugin in plugins {
        match groups
            .iter_mut()
            .find(|(module, _)| *module == plugin.module)
        {
            Some((_, sites)) => sites.push(plugin),
            None => groups.push((&plugin.module, vec![plugin])),
        }
    }
    groups
}

/// One plugin per module at its target version; rejects unresolved drift.
pub fn desired_plugins(
    plugins: &[Plugin],
    latest: &BTreeMap<String, String>,
) -> Result<Vec<Plugin>> {
    let mut desired = BTreeMap::<&str, Plugin>::new();
    for plugin in plugins {
        let version = latest.get(&plugin.module).unwrap_or(&plugin.version);
        if let Some(existing) = desired.get(plugin.module.as_str()) {
            if existing.version != *version {
                return Err(format!(
                    "Caddy plugin {} has ambiguous versions: {}, {version}",
                    plugin.module, existing.version
                )
                .into());
            }
        }
        desired.insert(
            &plugin.module,
            Plugin {
                version: version.clone(),
                ..plugin.clone()
            },
        );
    }
    Ok(desired.into_values().collect())
}

/// Require plugin and hash declarations to live in one file.
pub fn validate_single_owner(plugins: &[Plugin], hash_files: &[PathBuf]) -> Result<()> {
    let owners: BTreeSet<&PathBuf> = plugins
        .iter()
        .map(|plugin| &plugin.file)
        .chain(hash_files)
        .collect();
    if owners.len() > 1 {
        return Err(format!(
            "Caddy plugin/hash declarations span multiple files; effective plugin ownership is ambiguous: {}",
            owners
                .into_iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
        .into());
    }
    Ok(())
}

/// Tracked and untracked `*.nix` files under `root` containing `marker`,
/// relative to `root`.
pub fn find_nix_files(root: &Path, marker: &str) -> Result<Vec<PathBuf>> {
    let output = Command::new("git")
        .args([
            "grep",
            "-l",
            "-z",
            "--untracked",
            "-F",
            "-e",
            marker,
            "--",
            "*.nix",
        ])
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(format!(
            "git grep failed while discovering Nix files: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(String::from_utf8_lossy(path).into_owned()))
        .collect())
}

/// Plugin occurrences under `root`, with paths relative to `root`.
pub fn discover(root: &Path) -> Result<Vec<Plugin>> {
    let mut plugins = Vec::new();
    for file in find_nix_files(root, PLUGINS_MARKER)? {
        plugins.extend(extract_plugins(
            &fs::read_to_string(root.join(&file))?,
            &file,
        ));
    }
    Ok(plugins)
}

/// Files under `root` assigning a `caddy.pluginsHash = "..."` literal,
/// relative to `root`.
pub fn hash_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for file in find_nix_files(root, HASH_MARKER)? {
        if set_plugins_hash(&fs::read_to_string(root.join(&file))?, "").1 > 0 {
            files.push(file);
        }
    }
    Ok(files)
}

/// Go module proxy path encoding: uppercase letters become `!` + lowercase.
fn escape_module(module: &str) -> String {
    let mut escaped = String::with_capacity(module.len());
    for c in module.chars() {
        if c.is_ascii_uppercase() {
            escaped.push('!');
        }
        escaped.push(c.to_ascii_lowercase());
    }
    escaped
}

/// Latest version of a Go module from `proxy.golang.org`.
pub fn fetch_latest(module: &str) -> Result<String> {
    #[derive(serde::Deserialize)]
    struct Latest {
        #[serde(rename = "Version")]
        version: String,
    }
    let url = format!("https://proxy.golang.org/{}/@latest", escape_module(module));
    let latest: Latest = ureq::get(&url)
        .call()
        .map_err(|e| format!("GET {url}: {e}"))?
        .body_mut()
        .read_json()
        .map_err(|e| format!("parse JSON from {url}: {e}"))?;
    Ok(latest.version)
}

/// Resolve and realize the vendor hash for `plugins` from the flake at `flake`.
pub fn resolve_hash(flake: &Path, plugins: &[Plugin], nix: &mut impl Nix) -> Result<Resolved> {
    let system = nix.system()?;
    let stderr = nix.probe(&vendor_expr(flake, &system, plugins, None))?;
    let hash = extract_got_hashes(&stderr)
        .pop()
        .ok_or_else(|| format!("nix build failed without a fixed-output 'got:' hash:\n{stderr}"))?;
    let store_path = nix.realize(&vendor_expr(flake, &system, plugins, Some(&hash)))?;
    Ok(Resolved { hash, store_path })
}

/// Result of [`refresh`].
#[derive(Debug, Clone, Default)]
pub struct Refresh {
    /// Desired plugin set; versions are latest unless `hash_only`.
    pub plugins: Vec<Plugin>,
    /// Updated `(module, old, new)` occurrences.
    pub updates: Vec<(String, String, String)>,
    /// Resolved hash; `None` for a dry run or when no plugins are declared.
    pub resolved: Option<Resolved>,
}

/// Bump every plugin under `root` to its latest version (unless `hash_only`),
/// resolve the vendor hash and write both back. A dry run edits nothing and
/// builds nothing. Publishing `resolved.store_path` is the caller's step.
pub fn refresh(root: &Path, nix: &mut impl Nix, hash_only: bool, dry_run: bool) -> Result<Refresh> {
    let root = root.canonicalize()?;
    let occurrences = discover(&root)?;
    if occurrences.is_empty() {
        return Ok(Refresh::default());
    }
    validate_single_owner(&occurrences, &hash_files(&root)?)?;
    // Every occurrence lives in this one file (validate_single_owner).
    let file = root.join(&occurrences[0].file);
    let raw = fs::read_to_string(&file)?;
    // One hash covers exactly one plugin set; reject anything else up front.
    let lists = assignment_values(&raw, PLUGINS_MARKER, '[', ']').len();
    let hashes = set_plugins_hash(&raw, "").1;
    if lists != 1 || hashes != 1 {
        return Err(format!(
            "{} must contain exactly one `caddy.plugins = [ ... ]` list and one \
             `caddy.pluginsHash = \"...\"` string assignment (found {lists} and {hashes})",
            file.display()
        )
        .into());
    }
    let mut latest = BTreeMap::new();
    if !hash_only {
        // ponytail: sequential Go proxy lookups; parallelize if plugin sets grow.
        for (module, _) in group_by_module(&occurrences) {
            latest.insert(module.to_string(), fetch_latest(module)?);
        }
    }
    let plugins = desired_plugins(&occurrences, &latest)?;
    let updates: Vec<_> = occurrences
        .iter()
        .filter_map(|site| {
            let new = latest
                .get(&site.module)
                .filter(|new| **new != site.version)?;
            Some((site.module.clone(), site.version.clone(), new.clone()))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if dry_run {
        return Ok(Refresh {
            plugins,
            updates,
            resolved: None,
        });
    }
    let resolved = resolve_hash(&root, &plugins, nix)?;
    let mut updated = raw.clone();
    for (module, old, new) in &updates {
        updated = bump_version(&updated, module, old, new);
    }
    let (updated, _) = set_plugins_hash(&updated, &resolved.hash);
    if updated != raw {
        fs::write(&file, updated)?;
    }
    Ok(Refresh {
        plugins,
        updates,
        resolved: Some(resolved),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_TLS: &str = r#"      caddy.enable = true;
      caddy.plugins = ["github.com/caddy-dns/cloudflare@v0.2.4"];
      caddy.pluginsHash = "sha256-11RV+JD4rIx8efCGkcKcewu3QZN/tOiB7jusv5qZ+5I=";
"#;

    const SAMPLE_MULTI: &str = r#"caddy.plugins = [
  "github.com/caddy-dns/cloudflare@v0.2.4"
  "github.com/mholt/caddy-l4@v0.0.0-20250605"
];
"#;

    fn plugin(file: &str, module: &str, version: &str) -> Plugin {
        Plugin {
            file: PathBuf::from(file),
            module: module.to_string(),
            version: version.to_string(),
        }
    }

    #[test]
    fn extracts_plugins() {
        let plugins = extract_plugins(SAMPLE_TLS, Path::new("test.nix"));
        assert_eq!(
            plugins,
            [plugin(
                "test.nix",
                "github.com/caddy-dns/cloudflare",
                "v0.2.4"
            )]
        );
        let plugins = extract_plugins(SAMPLE_MULTI, Path::new("test.nix"));
        assert_eq!(plugins.len(), 2);
        assert_eq!(plugins[1].module, "github.com/mholt/caddy-l4");
        assert_eq!(plugins[1].version, "v0.0.0-20250605");
        let raw = r#"caddy.plugins = ["github.com/owner/parent/sub@v0.5.0"];"#;
        assert_eq!(
            extract_plugins(raw, Path::new("x.nix"))[0].module,
            "github.com/owner/parent/sub"
        );
        assert!(
            extract_plugins(
                r#"caddy.plugins = ["github.com/no-version" "x@y"];"#,
                Path::new("x.nix")
            )
            .is_empty()
        );
        // Only the plugin list counts; other module strings and quotes are ignored.
        let raw = "# \"github.com/c@v9\" \"quote\ncaddy.plugins\n  =\n  [ \"github.com/a@v1\" ];\nx = \"github.com/b@v2\";";
        assert_eq!(
            extract_plugins(raw, Path::new("x.nix")),
            [plugin("x.nix", "github.com/a", "v1")]
        );
        // Commented plugins and declarations do not count; `#` in strings is kept.
        let raw = "# caddy.plugins = [\"github.com/z@v0\"];\n/* caddy.plugins = [] */\ncaddy.plugins = [\n  # \"github.com/old@v0\"\n  \"github.com/a@v1#x\" /* \"github.com/b@v2\" */\n];";
        assert_eq!(
            extract_plugins(raw, Path::new("x.nix")),
            [plugin("x.nix", "github.com/a", "v1#x")]
        );
        assert_eq!(
            set_plugins_hash("# caddy.pluginsHash = \"old\"; é\n", "sha256-X=").1,
            0
        );
    }

    #[test]
    fn bumps_only_the_target_module() {
        let updated = bump_version(
            SAMPLE_MULTI,
            "github.com/caddy-dns/cloudflare",
            "v0.2.4",
            "v0.3.0",
        );
        assert!(updated.contains("github.com/caddy-dns/cloudflare@v0.3.0"));
        assert!(updated.contains("github.com/mholt/caddy-l4@v0.0.0-20250605"));
        assert!(!updated.contains("@v0.2.4"));
        assert_eq!(
            bump_version(SAMPLE_MULTI, "github.com/never/here", "v1", "v2"),
            SAMPLE_MULTI
        );
        assert_eq!(
            bump_version(r#""m@v1" "m@v1+incompatible""#, "m", "v1", "v2"),
            r#""m@v2" "m@v1+incompatible""#
        );
    }

    #[test]
    fn sets_plugins_hash() {
        let (out, replaced) = set_plugins_hash(SAMPLE_TLS, "sha256-NEWHASH=");
        assert_eq!(replaced, 1);
        assert_eq!(
            out,
            SAMPLE_TLS.replace(
                "sha256-11RV+JD4rIx8efCGkcKcewu3QZN/tOiB7jusv5qZ+5I=",
                "sha256-NEWHASH="
            )
        );
        assert_eq!(
            set_plugins_hash("no hash here", "sha256-X="),
            ("no hash here".to_string(), 0)
        );
        assert_eq!(
            set_plugins_hash("caddy.pluginsHash=\n  \"\";", "sha256-X="),
            ("caddy.pluginsHash=\n  \"sha256-X=\";".to_string(), 1)
        );
        assert_eq!(set_plugins_hash("caddy.pluginsHash = h;", "sha256-X=").1, 0);
    }

    #[test]
    fn extracts_got_hashes() {
        let stderr = r#"error: hash mismatch in fixed-output derivation '/nix/store/aaa-go-modules.drv':
         specified: sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=
            got:    sha256-BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=
            got:    sha256-aaa+bbb/ccc=
"#;
        assert_eq!(
            extract_got_hashes(stderr),
            [
                "sha256-BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=",
                "sha256-aaa+bbb/ccc="
            ]
        );
        assert!(extract_got_hashes("nothing matches here\n").is_empty());
    }

    #[test]
    fn vendor_expr_includes_path_system_modules_and_hash() {
        let plugins = [plugin("x.nix", "github.com/foo/bar", "v1")];
        let expr = vendor_expr(Path::new("/path/to/flake"), "x86_64-linux", &plugins, None);
        for part in [
            "/path/to/flake",
            "x86_64-linux",
            "\"github.com/foo/bar@v1\"",
            FAKE_HASH,
            "caddyPluginsVendorFor",
        ] {
            assert!(expr.contains(part), "{part} missing from {expr}");
        }
        assert!(
            vendor_expr(Path::new("/f"), "s", &plugins, Some("sha256-X="))
                .ends_with("\"sha256-X=\")")
        );
        let expr = vendor_expr(Path::new("/my dir/a\"${b}"), "s", &plugins, None);
        assert!(
            expr.contains(r#"(builtins.getFlake "/my dir/a\"\${b}")."#),
            "{expr}"
        );
        assert_eq!(escape_module("github.com/Azure/x"), "github.com/!azure/x");
    }

    #[test]
    fn groups_fingerprints_and_validates() {
        let plugins = [
            plugin("a.nix", "github.com/a", "v1"),
            plugin("b.nix", "github.com/b", "v2"),
            plugin("c.nix", "github.com/a", "v1"),
        ];
        let grouped = group_by_module(&plugins);
        assert_eq!(
            grouped
                .iter()
                .map(|(m, s)| (*m, s.len()))
                .collect::<Vec<_>>(),
            [("github.com/a", 2), ("github.com/b", 1)]
        );
        assert_eq!(fingerprint(&plugins), "github.com/a@v1\ngithub.com/b@v2");
        let drift = [
            plugin("caddy.nix", "github.com/m", "v1"),
            plugin("caddy.nix", "github.com/m", "v2"),
        ];
        assert!(desired_plugins(&drift, &BTreeMap::new()).is_err());
        let latest = BTreeMap::from([("github.com/m".to_string(), "v3".to_string())]);
        assert_eq!(
            desired_plugins(&drift, &latest).unwrap(),
            [plugin("caddy.nix", "github.com/m", "v3")]
        );
        let one = [plugin("one.nix", "github.com/x", "v1")];
        assert!(validate_single_owner(&one, &[PathBuf::from("one.nix")]).is_ok());
        assert!(validate_single_owner(&one, &[PathBuf::from("two.nix")]).is_err());
    }

    struct Fake;
    impl Nix for Fake {
        fn system(&mut self) -> Result<String> {
            Ok("x86_64-linux".into())
        }
        fn probe(&mut self, expr: &str) -> Result<String> {
            assert!(expr.contains(FAKE_HASH));
            Ok("got:    sha256-NEW=\n".into())
        }
        fn realize(&mut self, expr: &str) -> Result<String> {
            assert!(expr.contains("\"sha256-NEW=\""));
            Ok("/nix/store/x-vendor".into())
        }
    }

    #[test]
    fn hash_only_refresh_rewrites_the_owner_file() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(dir.path())
                    .status()
                    .unwrap()
                    .success()
            )
        };
        git(&["init", "-q"]);
        fs::write(dir.path().join("caddy.nix"), SAMPLE_TLS).unwrap();
        // Reading the option elsewhere does not make another hash owner.
        fs::write(dir.path().join("other.nix"), "x: x.caddy.pluginsHash\n").unwrap();
        let dry = refresh(dir.path(), &mut Fake, true, true).unwrap();
        assert!(dry.resolved.is_none());
        assert_eq!(
            fs::read_to_string(dir.path().join("caddy.nix")).unwrap(),
            SAMPLE_TLS
        );
        let done = refresh(dir.path(), &mut Fake, true, false).unwrap();
        assert_eq!(done.resolved.unwrap().store_path, "/nix/store/x-vendor");
        assert!(done.updates.is_empty());
        assert!(
            fs::read_to_string(dir.path().join("caddy.nix"))
                .unwrap()
                .contains(r#"caddy.pluginsHash = "sha256-NEW=""#)
        );
        // A computed hash cannot be rewritten; fail instead of leaving it stale.
        let computed = "caddy.plugins = [\"github.com/a@v1\"];\ncaddy.pluginsHash = h;\n";
        fs::write(dir.path().join("caddy.nix"), computed).unwrap();
        assert!(refresh(dir.path(), &mut Fake, true, false).is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join("caddy.nix")).unwrap(),
            computed
        );
        // Two independent plugin sets cannot share one resolved hash.
        let two = format!("{SAMPLE_TLS}{SAMPLE_TLS}");
        fs::write(dir.path().join("caddy.nix"), &two).unwrap();
        assert!(refresh(dir.path(), &mut Fake, true, true).is_err());
    }
}
