use canix_toolbelt::runtime::RuntimeManifest;
use std::error::Error;

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("runtime.pkl");
    std::fs::write(&path, include_str!("../examples/runtime.pkl")).unwrap();
    (directory, path)
}

#[test]
fn explicit_path_preserves_manifest_wire_format() {
    let (_directory, path) = fixture();
    let manifest = RuntimeManifest::load_from(&path).unwrap();
    assert_eq!(manifest.bins["fetch-tool"].path, "/opt/example/bin/fetch");
    assert_eq!(manifest.endpoints["api"].port, 8032);
    assert_eq!(manifest.bins["fetch-tool"].description, None);
    let json = serde_json::to_value(&manifest).unwrap();
    assert_eq!(json["secrets"]["token"]["agenixPath"], "/run/example/token");
    assert_eq!(json["secrets"]["token"]["mode"], "0440");
}

#[test]
fn consumer_schema_supplies_defaults_and_typed_entries() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("Schema.pkl"),
        r#"
class Binary {
    name: String
    path: String
    description: String? = null
}
class Endpoint {
    name: String
    scheme: String = "http"
    host: String
    port: UInt16
    description: String? = null
}
class Secret {
    name: String
    agenixPath: String
    mode: String = "0440"
}
bins: Mapping<String, Binary> = new {}
endpoints: Mapping<String, Endpoint> = new {}
secrets: Mapping<String, Secret> = new {}
"#,
    )
    .unwrap();
    let path = directory.path().join("runtime.pkl");
    std::fs::write(
        &path,
        r#"
amends "Schema.pkl"
import "Schema.pkl" as S
bins { ["fetch"] = new S.Binary { name = "fetch"; path = "/opt/example/bin/fetch"; description = "quotes: \" and \\ paths" } }
endpoints { ["api"] = new S.Endpoint { name = "api"; host = "127.0.0.1"; port = 8032 } }
secrets { ["token"] = new S.Secret { name = "token"; agenixPath = "/run/example/token" } }
"#,
    )
    .unwrap();

    let manifest = RuntimeManifest::load_from(&path).unwrap();
    assert_eq!(
        manifest.bins["fetch"].description.as_deref(),
        Some("quotes: \" and \\ paths")
    );
    assert_eq!(manifest.endpoints["api"].scheme, "http");
    assert_eq!(manifest.secrets["token"].mode, "0440");
}

#[test]
fn missing_file_retains_path_and_error_source() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing.pkl");
    let error = RuntimeManifest::load_from(&path).unwrap_err();
    assert_eq!(error.path(), path);
    assert!(error.to_string().contains("missing.pkl"));
    assert!(error.source().is_some());
}

#[test]
fn malformed_manifest_is_not_treated_as_empty() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("invalid.pkl");
    std::fs::write(&path, "bins = 7\n").unwrap();
    assert!(RuntimeManifest::load_from(&path).is_err());
}

#[tokio::test]
async fn async_loader_works_and_sync_loader_rejects_nested_runtime() {
    let (_directory, path) = fixture();
    let manifest = RuntimeManifest::load_from_async(&path).await.unwrap();
    assert_eq!(manifest.endpoints["api"].host, "127.0.0.1");
    let error = RuntimeManifest::load_from(&path).unwrap_err();
    assert!(error.to_string().contains("Tokio runtime"));
}
