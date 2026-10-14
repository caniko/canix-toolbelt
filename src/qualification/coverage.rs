//! Coverage applicability over sealed receipts. Provider custody is separate:
//! callers obtain the expected report/receipt hashes through their trusted forge
//! observer, never from the bundle being verified.
use super::*;
use std::collections::BTreeSet;

/// Hosted evidence cannot describe live production acceptance.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CoverageLevel {
    /// Reusable implementation's own qualification.
    Producer,
    /// Exact consumer artifact and contract exercised by that implementation.
    ConsumerConformance,
}

/// Independently chosen obligation and original method inventory.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoverageRequirement {
    /// Producer evidence is not implicitly promoted to consumer evidence.
    pub level: CoverageLevel,
    /// Required sealed execution label.
    pub gate: String,
    /// Stable original test selectors; empty inventories are invalid.
    pub methods: BTreeSet<String>,
}

/// Consumer-selected requirements, not discovered from successful jobs.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoverageRequirements {
    /// Wire version, currently 1.
    pub schema_version: u32,
    /// Full source/input cohort, including `consumer-contract` for conformance.
    pub composition: Composition,
    /// Stable obligation IDs with explicit levels, gates and method selectors.
    pub obligations: BTreeMap<String, CoverageRequirement>,
}

/// Per-method execution disposition. None except Passed establishes coverage.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MethodStatus {
    /// Executed and passed.
    Passed,
    /// Executed and failed.
    Failed,
    /// Did not execute.
    Skipped,
    /// No terminal result.
    Pending,
    /// Execution cancelled.
    Cancelled,
}

/// A producer's coverage claim, bound to a passing execution by the seal.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObligationCoverage {
    /// Explicit evidence level.
    pub level: CoverageLevel,
    /// Sealed gate that executed these methods.
    pub gate: String,
    /// Terminal execution result for every independently required method.
    #[serde(deserialize_with = "unique_map")]
    pub methods: BTreeMap<String, MethodStatus>,
}

/// Versioned `artifacts/coverage.json` inside a portable seal.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoverageReport {
    /// Wire version, currently 1.
    pub schema_version: u32,
    /// Tested source/input cohort.
    pub composition: Composition,
    /// Explicit coverage claims; the verifier requires the exact obligation set.
    #[serde(deserialize_with = "unique_map")]
    pub obligations: BTreeMap<String, ObligationCoverage>,
}

fn unique_map<'de, D, V>(deserializer: D) -> Result<BTreeMap<String, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    V: Deserialize<'de>,
{
    struct Unique<V>(std::marker::PhantomData<V>);
    impl<'de, V: Deserialize<'de>> serde::de::Visitor<'de> for Unique<V> {
        type Value = BTreeMap<String, V>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a map with unique coverage keys")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut access: A,
        ) -> Result<Self::Value, A::Error> {
            let mut values = BTreeMap::new();
            while let Some((key, value)) = access.next_entry::<String, V>()? {
                if values.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate coverage key"));
                }
            }
            Ok(values)
        }
    }
    deserializer.deserialize_map(Unique(std::marker::PhantomData))
}

/// Integrity/applicability result. Does not authenticate a provider or authorize deployment.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageVerification {
    /// Number of complete obligations verified.
    pub obligations_verified: usize,
    /// Number of passing original methods verified across those obligations.
    pub methods_verified: usize,
    /// Explicitly distinguishes this check from provider/live acceptance.
    pub scope: &'static str,
}

/// Verify coverage against independent requirements, exact receipt hashes and a
/// report hash obtained through provider custody. A self-consistent bundle alone
/// cannot establish provider authenticity. No hosted level covers live acceptance.
pub fn verify_coverage(
    directory: &Path,
    expected: &CoverageRequirements,
    required_receipts: &ArtifactIndex,
    report_sha256: &str,
) -> io::Result<CoverageVerification> {
    if expected.schema_version != 1 || expected.obligations.is_empty() {
        return Err(invalid(
            "coverage requires version 1 and nonempty obligations",
        ));
    }
    let seal = verify_seal(directory, &expected.composition, required_receipts)?;
    const REPORT: &str = "artifacts/coverage.json";
    if seal.artifacts.get(REPORT).map(String::as_str) != Some(report_sha256)
        || sha256_file(&directory.join(REPORT))? != report_sha256
    {
        return Err(invalid(
            "coverage report differs from independently bound digest",
        ));
    }
    let report: CoverageReport = read_json(&directory.join(REPORT))?;
    if report.schema_version != 1
        || report.composition != expected.composition
        || report.obligations.keys().ne(expected.obligations.keys())
    {
        return Err(invalid(
            "coverage source/input cohort or obligation set differs",
        ));
    }
    let mut methods_verified = 0;
    for (name, required) in &expected.obligations {
        let observed = &report.obligations[name];
        if name.is_empty()
            || required.methods.is_empty()
            || required.methods.iter().any(String::is_empty)
            || !required_receipts.contains_key(&required.gate)
            || observed.gate != required.gate
            || observed.level != required.level
        {
            return Err(invalid(format!(
                "coverage level, gate or inventory differs for {name}"
            )));
        }
        if required.level == CoverageLevel::ConsumerConformance {
            let contract = expected.composition.inputs.get("consumer-contract");
            let consumer = expected.composition.sources.get("consumer");
            let digest = |value: Option<&String>| {
                value.is_some_and(|hash| {
                    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
                })
            };
            if !digest(contract) || !digest(consumer) {
                return Err(invalid(
                    "consumer conformance requires bound consumer and consumer-contract digests",
                ));
            }
        }
        if observed.methods.keys().ne(required.methods.iter())
            || observed
                .methods
                .values()
                .any(|status| *status != MethodStatus::Passed)
        {
            return Err(invalid(format!(
                "method coverage is missing or not passed for {name}"
            )));
        }
        methods_verified += required.methods.len();
    }
    Ok(CoverageVerification {
        obligations_verified: expected.obligations.len(),
        methods_verified,
        scope: "coverage-integrity-and-applicability",
    })
}
