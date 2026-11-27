//! Caddy plugin version and `pluginsHash` refresh for Nix flakes.
//!
//! Dotted declarations use the consumer's `caddyPluginsVendorFor` helper.
//! Nested Caddy declarations can instead key `pluginsHash` by Caddy version;
//! each hash is resolved against that version from the consumer's nixpkgs pins.
//! A refresh forces the vendor fixed-output derivation with a fake hash, reads
//! the `got:` hash, realizes the vendor output with it and edits the source.
//!
//! Realization is supplied through [`Nix`], so consumers keep their own
//! evaluation leases, build bounds and GC roots. Publication stays with the
//! caller: [`Resolved::store_path`] is the realized vendor output to upload.

use rnix::ast::{AttrSet, AttrpathValue, Expr, HasEntry, InterpolPart, Str};
use rowan::ast::AstNode;
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
            .args(["build", "--no-update-lock-file", "--impure", "--no-link"])
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
                "--no-update-lock-file",
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

fn nested_caddy(source: &str) -> Vec<AttrSet> {
    let root = rnix::Root::parse(source);
    if !root.errors().is_empty() {
        return Vec::new();
    }
    root.syntax()
        .descendants()
        .filter_map(AttrpathValue::cast)
        .filter_map(|entry| {
            let attr = entry.attrpath()?.attrs().last()?;
            match (attr.syntax().text().to_string().as_str(), entry.value()?) {
                ("caddy", Expr::AttrSet(set)) => Some(set),
                _ => None,
            }
        })
        .collect()
}

fn nested_fields<'a>(sets: &'a [AttrSet], key: &'a str) -> impl Iterator<Item = Expr> + 'a {
    sets.iter()
        .flat_map(HasEntry::attrpath_values)
        .filter_map(move |entry| {
            (entry.attrpath()?.syntax().text() == key).then(|| entry.value())?
        })
}

fn plain_string(string: &Str) -> Option<String> {
    match string.normalized_parts().as_slice() {
        [InterpolPart::Literal(value)] => Some(value.clone()),
        [] => Some(String::new()),
        _ => None,
    }
}

fn nested_plugin_strings(sets: &[AttrSet]) -> impl Iterator<Item = Str> + '_ {
    nested_fields(sets, "plugins")
        .filter_map(|expr| match expr {
            Expr::List(list) => Some(list.items()),
            _ => None,
        })
        .flatten()
        .filter_map(|expr| match expr {
            Expr::Str(string) => Some(string),
            _ => None,
        })
}

fn string_range(string: &Str) -> Range<usize> {
    let span = string.syntax().text_range();
    usize::from(span.start())..usize::from(span.end())
}

fn nested_version_hashes(expr: Expr) -> Option<BTreeMap<String, Range<usize>>> {
    let set = match expr {
        Expr::AttrSet(set) => set,
        Expr::Select(select) => match select.expr()? {
            Expr::AttrSet(set) => set,
            _ => return None,
        },
        _ => return None,
    };
    let mut ranges = BTreeMap::new();
    for entry in set.entries() {
        let rnix::ast::Entry::AttrpathValue(entry) = entry else {
            return None;
        };
        let attrpath = entry.attrpath()?;
        let mut attrs = attrpath.attrs();
        let (Some(rnix::ast::Attr::Str(key)), None, Some(Expr::Str(value))) =
            (attrs.next(), attrs.next(), entry.value())
        else {
            return None;
        };
        let version = plain_string(&key)?;
        if version.split('.').count() != 3
            || !version
                .split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            || !plain_string(&value)?.starts_with("sha256-")
            || ranges.insert(version, string_range(&value)).is_some()
        {
            return None;
        }
    }
    (!ranges.is_empty()).then_some(ranges)
}

/// Extract `"github.com/...@version"` literals from dotted and nested Caddy lists.
pub fn extract_plugins(source: &str, file: &Path) -> Vec<Plugin> {
    let nested = nested_caddy(source);
    let nested_plugins = nested_plugin_strings(&nested)
        .filter_map(|string| plain_string(&string))
        .filter_map(|literal| {
            let (module, version) = literal.split_once('@')?;
            (module.starts_with("github.com/") && !version.is_empty()).then(|| Plugin {
                file: file.to_path_buf(),
                module: module.to_string(),
                version: version.to_string(),
            })
        });
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
        .chain(nested_plugins)
        .collect()
}

/// Replace complete `"module@old"` literals with `"module@new"`, only inside
/// dotted and nested Caddy plugin lists.
pub fn bump_version(source: &str, module: &str, old: &str, new: &str) -> String {
    let old = format!("{module}@{old}");
    let from = nix_string(&old);
    let to = nix_string(&format!("{module}@{new}"));
    let mut out = String::with_capacity(source.len());
    let mut copied = 0;
    for range in assignment_values(source, PLUGINS_MARKER, '[', ']') {
        out.push_str(&source[copied..range.start]);
        out.push_str(&source[range.clone()].replace(&from, &to));
        copied = range.end;
    }
    out.push_str(&source[copied..]);
    let nested = nested_caddy(&out);
    let mut changes: Vec<_> = nested_plugin_strings(&nested)
        .filter(|literal| plain_string(literal).as_deref() == Some(old.as_str()))
        .map(|literal| string_range(&literal))
        .collect();
    changes.sort_unstable_by_key(|range| std::cmp::Reverse(range.start));
    for range in changes {
        out.replace_range(range, &to);
    }
    out
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
    let plugins = vendor_plugins(plugins);
    let hash = hash.unwrap_or(FAKE_HASH);
    format!(
        "((builtins.getFlake {flake}).legacyPackages.{system}.caddyPluginsVendorFor [ {plugins} ] \"{hash}\")",
        flake = nix_string(&flake.display().to_string()),
    )
}

fn vendor_plugins(plugins: &[Plugin]) -> String {
    plugins
        .iter()
        .map(|p| nix_string(&format!("{}@{}", p.module, p.version)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn versioned_vendor_expr(
    flake: &Path,
    system: &str,
    plugins: &[Plugin],
    version: &str,
    hash: Option<&str>,
) -> String {
    // Use the flake's pinned nixpkgs, never an arbitrary Caddy source override:
    // its withPlugins helper carries the corresponding Go/xcaddy setup.
    format!(
        r#"let
            inputs = (builtins.getFlake {flake}).inputs;
            version = {version};
            caddyFor = name: inputs.${{name}}.legacyPackages.${{ {system} }}.caddy;
            matches = name:
              let result = builtins.tryEval ((caddyFor name).version == version);
              in result.success && result.value;
            pins = builtins.filter
              (name: builtins.match "nixpkgs(-.*)?" name != null && matches name)
              (builtins.attrNames inputs);
            caddy = if pins == [] then throw ("No pinned nixpkgs provides Caddy " + version)
              else caddyFor (builtins.head pins);
          in (caddy.withPlugins {{ plugins = [ {plugins} ]; hash = {hash}; }}).src"#,
        flake = nix_string(&flake.display().to_string()),
        version = nix_string(version),
        system = nix_string(system),
        plugins = vendor_plugins(plugins),
        hash = nix_string(hash.unwrap_or(FAKE_HASH)),
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
    for file in find_nix_files(root, "plugins")? {
        plugins.extend(extract_plugins(
            &fs::read_to_string(root.join(&file))?,
            &file,
        ));
    }
    Ok(plugins)
}

/// Files under `root` assigning dotted or nested Caddy hashes, relative to `root`.
pub fn hash_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for file in find_nix_files(root, "pluginsHash")? {
        let raw = fs::read_to_string(root.join(&file))?;
        if set_plugins_hash(&raw, "").1 > 0
            || nested_fields(&nested_caddy(&raw), "pluginsHash")
                .next()
                .is_some()
        {
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
    resolve_vendor(nix, |hash| vendor_expr(flake, &system, plugins, hash))
}

fn resolve_vendor(
    nix: &mut impl Nix,
    expression: impl Fn(Option<&str>) -> String,
) -> Result<Resolved> {
    let stderr = nix.probe(&expression(None))?;
    let hash = extract_got_hashes(&stderr)
        .pop()
        .ok_or_else(|| format!("nix build failed without a fixed-output 'got:' hash:\n{stderr}"))?;
    let store_path = nix.realize(&expression(Some(&hash)))?;
    Ok(Resolved { hash, store_path })
}

/// Result of [`refresh`].
#[derive(Debug, Clone, Default)]
pub struct Refresh {
    /// Desired plugin set; versions are latest unless `hash_only`.
    pub plugins: Vec<Plugin>,
    /// Updated `(module, old, new)` occurrences.
    pub updates: Vec<(String, String, String)>,
    /// Resolved hash for a dotted declaration; `None` for versioned configs or dry runs.
    pub resolved: Option<Resolved>,
    /// Resolved hashes for a versioned declaration, keyed by Caddy version.
    pub resolved_by_version: BTreeMap<String, Resolved>,
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
    // Both layouts require a single plugin set and a single owner.
    let lists = assignment_values(&raw, PLUGINS_MARKER, '[', ']').len();
    let hashes = set_plugins_hash(&raw, "").1;
    let nested = nested_caddy(&raw);
    let nested_lists: Vec<_> = nested_fields(&nested, "plugins").collect();
    let nested_hashes: Vec<_> = nested_fields(&nested, "pluginsHash").collect();
    let version_hashes = if lists == 1
        && hashes == 1
        && nested_lists.is_empty()
        && nested_hashes.is_empty()
    {
        None
    } else if lists == 0
        && hashes == 0
        && nested_hashes.len() == 1
        && matches!(nested_lists.as_slice(), [Expr::List(list)] if list.items().count() == occurrences.len())
        && nested.iter().any(|set| {
            let set = std::slice::from_ref(set);
            nested_fields(set, "plugins").next().is_some()
                && nested_fields(set, "pluginsHash").next().is_some()
        })
    {
        Some(
            nested_version_hashes(nested_hashes[0].clone()).ok_or_else(|| {
                format!(
                    "{} must contain unique literal Caddy versions and hashes",
                    file.display()
                )
            })?,
        )
    } else {
        return Err(format!(
            "{} must contain one Caddy plugin list and one matching literal or versioned hash declaration",
            file.display()
        )
        .into());
    };
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
            ..Refresh::default()
        });
    }
    let mut updated = raw.clone();
    let mut resolved_by_version = BTreeMap::new();
    let resolved = if let Some(versions) = version_hashes {
        let system = nix.system()?;
        let mut replacements = Vec::new();
        for (version, range) in versions {
            let resolved = resolve_vendor(nix, |hash| {
                versioned_vendor_expr(&root, &system, &plugins, &version, hash)
            })?;
            replacements.push((range, nix_string(&resolved.hash)));
            resolved_by_version.insert(version, resolved);
        }
        replacements.sort_unstable_by_key(|(range, _)| std::cmp::Reverse(range.start));
        for (range, hash) in replacements {
            updated.replace_range(range, &hash);
        }
        None
    } else {
        let resolved = resolve_hash(&root, &plugins, nix)?;
        updated = set_plugins_hash(&updated, &resolved.hash).0;
        Some(resolved)
    };
    for (module, old, new) in &updates {
        updated = bump_version(&updated, module, old, new);
    }
    if updated != raw {
        fs::write(&file, updated)?;
    }
    Ok(Refresh {
        plugins,
        updates,
        resolved,
        resolved_by_version,
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
            bump_version(
                r#"caddy.plugins = [ "m@v1" "m@v1+incompatible" ];"#,
                "m",
                "v1",
                "v2"
            ),
            r#"caddy.plugins = [ "m@v2" "m@v1+incompatible" ];"#
        );
        // The same literal outside the plugin list is unrelated configuration.
        let other = r#"deps = [ "m@v1" ]; caddy.plugins = [ "m@v1" ];"#;
        assert_eq!(
            bump_version(other, "m", "v1", "v2"),
            r#"deps = [ "m@v1" ]; caddy.plugins = [ "m@v2" ];"#
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
    fn refreshes_nested_hashes_with_their_own_caddy_versions() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success()
        );
        let source = r#"{ pkgs, ... }: {
          unrelated.plugins = ["github.com/example/ignored@v1"];
          services.caddy = {
            plugins = [
              "github.com/example/plugin@v1"
              # "github.com/example/plugin@v1"
            ];
            pluginsHash =
              {
                # "2.11.6" = "sha256-comment=";
                "2.11.7" = "sha256-old7=";
                "2.11.9" = "sha256-old9=";
              }
              .${pkgs.caddy.version};
          };
        }"#;
        let path = dir.path().join("ingress.nix");
        fs::write(&path, source).unwrap();
        assert_eq!(
            hash_files(dir.path()).unwrap(),
            [PathBuf::from("ingress.nix")]
        );
        assert_eq!(
            discover(dir.path()).unwrap(),
            [plugin("ingress.nix", "github.com/example/plugin", "v1")]
        );
        assert_eq!(
            bump_version(source, "github.com/example/plugin", "v1", "v2"),
            source.replacen(
                "github.com/example/plugin@v1",
                "github.com/example/plugin@v2",
                1
            )
        );

        struct Versioned;
        impl Nix for Versioned {
            fn system(&mut self) -> Result<String> {
                Ok("x86_64-linux".into())
            }
            fn probe(&mut self, expr: &str) -> Result<String> {
                assert!(expr.contains("github.com/example/plugin@v1"));
                let suffix = if expr.contains("\"2.11.7\"") {
                    "7"
                } else {
                    assert!(expr.contains("\"2.11.9\""));
                    "9"
                };
                Ok(format!("got: sha256-NEW{suffix}="))
            }
            fn realize(&mut self, expr: &str) -> Result<String> {
                let suffix = if expr.contains("\"2.11.7\"") {
                    "7"
                } else {
                    assert!(expr.contains("\"2.11.9\""));
                    "9"
                };
                assert!(expr.contains(&format!("sha256-NEW{suffix}=")));
                Ok(format!("/nix/store/vendor{suffix}"))
            }
        }
        let dry = refresh(dir.path(), &mut Versioned, true, true).unwrap();
        assert!(dry.resolved_by_version.is_empty());
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        let done = refresh(dir.path(), &mut Versioned, true, false).unwrap();
        assert!(done.resolved.is_none());
        assert_eq!(
            done.resolved_by_version["2.11.7"].store_path,
            "/nix/store/vendor7"
        );
        assert_eq!(
            done.resolved_by_version["2.11.9"].store_path,
            "/nix/store/vendor9"
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            source
                .replace("sha256-old7=", "sha256-NEW7=")
                .replace("sha256-old9=", "sha256-NEW9=")
        );
        let computed = source.replace("\"sha256-old9=\"", "pkgs.lib.fakeHash");
        fs::write(&path, &computed).unwrap();
        assert!(refresh(dir.path(), &mut Versioned, true, false).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), computed);
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
