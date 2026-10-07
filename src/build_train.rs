//! Architecture integration over Fleetix's construction engine and the native
//! Nix specialist backend. Cache publication and activation remain caller-owned.
use fleetix::build_train::{
    runtime::{self, Backend, Client, Config},
    *,
};
use nix_manager_core::build::frontier::{Native, Node as NativeNode, Output};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::AtomicBool};

/// Deployment-generated client contract; no credentials are placed in this file.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    /// Canonical builder identity supplied by topology/deployment.
    pub builder: String,
    /// Private Unix socket on the builder.
    pub socket: PathBuf,
    /// Immutable native/store/admission policy digest.
    pub policy: String,
    /// Operator-owned exact preparation receipt directory.
    pub preparation_dir: PathBuf,
    /// Direct source/derivation roots shared with the coordinator.
    pub gc_roots: PathBuf,
}

impl Connection {
    /// Discover the selected builder's deployed service contract. An absent default
    /// permits standalone operation; explicit, malformed or incompatible contracts
    /// fail closed rather than silently bypassing shared construction.
    pub fn discover(
        builder: &str,
        explicit: Option<&Path>,
        default: &Path,
    ) -> Result<Option<Self>, String> {
        let path = explicit.unwrap_or(default);
        match std::fs::symlink_metadata(path) {
            Err(e) if explicit.is_none() && e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(e) => return Err(format!("inspect train connection {}: {e}", path.display())),
            Ok(_) => {}
        }
        let connection = Self::load(path)?;
        if connection.builder != builder {
            return Err("train connection does not match the selected build host".into());
        }
        Ok(Some(connection))
    }

    /// Load an explicitly selected connection; no Canix paths are assumed.
    pub fn load(path: &Path) -> Result<Self, String> {
        let config: Self =
            serde_json::from_reader(std::fs::File::open(path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if config.builder.is_empty()
            || config.policy.is_empty()
            || !config.socket.is_absolute()
            || !config.preparation_dir.is_absolute()
            || !config.gc_roots.starts_with("/nix/var/nix/gcroots/")
        {
            return Err("invalid build train connection".into());
        }
        Ok(config)
    }

    /// Call the shared library directly, rather than another frontend executable.
    pub fn client(&self) -> Client {
        Client {
            socket: self.socket.clone(),
            policy: self.policy.clone(),
        }
    }
}

/// Typed service configuration. All execution policy is supplied by deployment.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    /// Builder identity, independent of any consumer fleet.
    pub builder: String,
    /// Qualified resource-admission contract identity.
    pub admission_contract: String,
    /// Fleetix coordinator capacity and persistence policy.
    pub coordinator: Config,
    /// Native Nix backend policy.
    pub native: Native,
    /// Deployment-enforced coordinator memory ceiling, bound into the policy.
    pub memory_max: String,
}

/// Versioned positional JSON contract mirrored by the NixOS module. Arrays are
/// invariant under Cargo feature unification (`serde_json/preserve_order`).
pub fn policy_identity(service: &Service) -> Result<String, String> {
    let native = &service.native;
    let config = &service.coordinator;
    let value = (
        "fleetix-train-policy",
        2,
        &service.builder,
        &service.admission_contract,
        (
            &native.nix,
            &native.timeout,
            native.timeout_seconds,
            native.query_timeout_seconds,
            &native.system,
            &native.gc_roots,
            native.substitutes,
        ),
        (
            &config.socket,
            &config.state_dir,
            config.workers,
            config.planning_workers,
            config.queue_limit,
            config.aging_seconds,
            config.planning_timeout_seconds,
        ),
        &service.memory_max,
    );
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&value).map_err(|e| e.to_string())?)
    ))
}

struct NixBackend(Native);

impl NixBackend {
    fn request_native(&self, request: &Request, create: bool) -> Result<Native, String> {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        let base = std::fs::symlink_metadata(&self.0.gc_roots).map_err(|e| e.to_string())?;
        if !base.is_dir() || base.mode() & 0o077 != 0 {
            return Err("train GC root base must be a private directory".into());
        }
        let mut native = self.0.clone();
        let namespace = self.0.gc_roots.join("requests");
        let digest = format!("{:x}", Sha256::digest(request.attempt.as_bytes()));
        native.gc_roots = namespace.join(digest);
        for path in [&namespace, &native.gc_roots] {
            if create {
                match std::fs::DirBuilder::new().mode(0o700).create(path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
            match std::fs::symlink_metadata(path) {
                Ok(meta)
                    if meta.is_dir() && meta.uid() == base.uid() && meta.mode() & 0o077 == 0 => {}
                Err(e) if !create && e.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Err("unsafe request GC root namespace".into()),
            }
        }
        Ok(native)
    }
}

fn output(goal: &Goal) -> Output {
    Output {
        derivation: goal.derivation.clone(),
        name: goal.output.clone(),
    }
}
fn native_node(goal: &Goal, definition: &Definition) -> NativeNode {
    NativeNode {
        output: output(goal),
        path: definition.output_path.clone(),
        dependencies: definition.dependencies.iter().map(output).collect(),
        restore_only: definition.operation == Operation::Restore,
    }
}

/// Match Fleetix's archived evidence closure rather than every sibling output
/// present in native derivation JSON. The same ownership set governs release.
fn request_paths(request: &Request, graph: &Graph) -> std::collections::BTreeSet<String> {
    let source = request
        .source
        .split_once('#')
        .map_or(request.source.as_str(), |(path, _)| path);
    let mut paths = std::collections::BTreeSet::from([source.to_owned()]);
    paths.extend(request.roots.iter().map(|goal| goal.derivation.clone()));
    let mut visited = std::collections::BTreeSet::new();
    let mut pending: Vec<_> = request.roots.iter().collect();
    while let Some(goal) = pending.pop() {
        if !visited.insert(goal) {
            continue;
        }
        if let Some(definition) = graph.get(goal) {
            paths.insert(goal.derivation.clone());
            paths.insert(definition.output_path.clone());
            // Retain build-only evidence even when restoration prunes dispatch.
            pending.extend(&definition.dependencies);
        }
    }
    paths
}

impl Backend for NixBackend {
    fn plan(&self, request: &Request) -> Result<Graph, String> {
        self.0
            .plan(&request.roots.iter().map(output).collect::<Vec<_>>())
            .map(|nodes| {
                nodes
                    .into_iter()
                    .map(|node| {
                        (
                            Goal {
                                derivation: node.output.derivation,
                                output: node.output.name,
                            },
                            Definition {
                                output_path: node.path,
                                dependencies: node
                                    .dependencies
                                    .into_iter()
                                    .map(|o| Goal {
                                        derivation: o.derivation,
                                        output: o.name,
                                    })
                                    .collect(),
                                operation: if node.restore_only {
                                    Operation::Restore
                                } else {
                                    Operation::Build
                                },
                            },
                        )
                    })
                    .collect()
            })
            .map_err(|e| e.to_string())
    }
    fn retain(&self, request: &Request, graph: &Graph) -> Result<(), String> {
        let native = self.request_native(request, true)?;
        nix_manager_core::build::frontier::retain_paths(
            &native.gc_roots,
            &request_paths(request, graph),
        )
        .map_err(|e| e.to_string())
    }
    fn release(&self, request: &Request, graph: &Graph) -> Result<(), String> {
        let native = self.request_native(request, false)?;
        let owned = request_paths(request, graph);
        // Only the archived request's private namespace is removed. Shared outputs
        // remain pinned by every other request and by caller-owned attempt roots.
        let entries = match std::fs::read_dir(&native.gc_roots) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
        let mut paths = Vec::new();
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            let target = std::fs::read_link(&path).map_err(|e| e.to_string())?;
            if !target.starts_with("/nix/store")
                || target.parent() != Some(Path::new("/nix/store"))
                || path.file_name() != target.file_name()
                || !target.to_str().is_some_and(|p| owned.contains(p))
            {
                return Err("unexpected request GC root entry; retained for inspection".into());
            }
            paths.push(path);
        }
        for path in paths {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        std::fs::remove_dir(&native.gc_roots).map_err(|e| e.to_string())?;
        std::fs::File::open(native.gc_roots.parent().ok_or("missing root namespace")?)
            .and_then(|file| file.sync_all())
            .map_err(|e| e.to_string())
    }
    fn valid(&self, _: &Goal, definition: &Definition) -> Result<bool, String> {
        self.0
            .valid(&definition.output_path)
            .map_err(|e| e.to_string())
    }
    fn realise(&self, dispatch: &Dispatch) -> Result<(), String> {
        self.0
            .realise(&native_node(&dispatch.goal, &dispatch.definition))
            .map_err(|e| e.to_string())
    }
}

/// Start the shared engine using the immutable service/backend contract.
pub fn serve(service: Service, stop: Arc<AtomicBool>) -> Result<(), String> {
    validate_service(&service)?;
    runtime::serve(
        service.coordinator,
        Arc::new(NixBackend(service.native)),
        stop,
    )
}

fn validate_service(service: &Service) -> Result<(), String> {
    if service.coordinator.policy != policy_identity(service)? {
        return Err("build train policy digest does not match its backend contract".into());
    }
    if service.native.query_timeout_seconds == 0
        || service.native.query_timeout_seconds > 300
        || service.coordinator.planning_timeout_seconds
            < service
                .native
                .query_timeout_seconds
                .saturating_mul(2)
                .saturating_add(20)
    {
        return Err(
            "planning deadline must cover two bounded native queries and kill grace periods".into(),
        );
    }
    Ok(())
}

/// Complete an explicit offline policy rollover without migrating old requests.
/// The caller must hold the host activation lease and stop the old service first.
pub fn rollover(previous: Service, next: Service, token: &str) -> Result<PathBuf, String> {
    validate_service(&previous)?;
    validate_service(&next)?;
    if previous.builder != next.builder || previous.native.gc_roots != next.native.gc_roots {
        return Err("policy rollover cannot move builder or request-root ownership".into());
    }
    runtime::rollover(
        &previous.coordinator,
        &next.coordinator,
        &NixBackend(previous.native),
        token,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::os::unix::fs::symlink;

    fn service() -> Service {
        serde_json::from_str(include_str!("../tests/fixtures/build-train-service.json")).unwrap()
    }

    #[test]
    fn policy_matches_nix_fixture_under_both_json_map_feature_modes() {
        let service = service();
        assert_eq!(
            policy_identity(&service).unwrap(),
            service.coordinator.policy
        );
    }

    #[test]
    fn rollover_rejects_changed_ownership_and_invalid_contracts_before_touching_state() {
        let temp = tempfile::tempdir().unwrap();
        let mut previous = service();
        previous.coordinator.state_dir = temp.path().join("state");
        previous.coordinator.socket = temp.path().join("coordinator.sock");
        previous.coordinator.policy = policy_identity(&previous).unwrap();
        std::fs::create_dir(&previous.coordinator.state_dir).unwrap();
        let journal = previous.coordinator.state_dir.join("train.json");
        std::fs::write(&journal, "original recovery evidence").unwrap();
        for field in ["builder", "roots", "policy", "deadline"] {
            let mut next = previous.clone();
            match field {
                "builder" => next.builder = "replacement-builder".into(),
                "roots" => next.native.gc_roots = temp.path().join("foreign-roots"),
                "deadline" => next.coordinator.planning_timeout_seconds = 1,
                _ => {}
            }
            next.coordinator.policy = policy_identity(&next).unwrap();
            if field == "policy" {
                next.coordinator.policy = "unverified-policy".into();
            }
            let error = rollover(previous.clone(), next, "token").unwrap_err();
            assert!(
                error.contains("ownership")
                    || error.contains("backend contract")
                    || error.contains("deadline"),
                "{field}: {error}"
            );
            assert_eq!(
                std::fs::read_to_string(&journal).unwrap(),
                "original recovery evidence"
            );
            assert!(
                !previous
                    .coordinator
                    .state_dir
                    .join("rollover.json")
                    .exists()
            );
        }
    }

    #[test]
    fn automatic_discovery_requires_the_selected_builder_and_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let default = temp.path().join("connection.json");
        assert!(
            Connection::discover("atlas", None, &default)
                .unwrap()
                .is_none()
        );
        assert!(Connection::discover("atlas", Some(&default), &default).is_err());
        std::fs::write(&default, "malformed").unwrap();
        assert!(Connection::discover("atlas", None, &default).is_err());
        let service = service();
        let connection = Connection {
            builder: service.builder,
            policy: service.coordinator.policy,
            socket: service.coordinator.socket,
            gc_roots: service.native.gc_roots,
            preparation_dir: "/var/lib/fleetix-train/preparation".into(),
        };
        std::fs::write(&default, serde_json::to_vec(&connection).unwrap()).unwrap();
        assert_eq!(
            Connection::discover("atlas", None, &default)
                .unwrap()
                .unwrap()
                .policy,
            connection.policy
        );
        assert!(
            Connection::discover("murph", None, &default)
                .unwrap_err()
                .contains("selected build host")
        );
    }

    #[test]
    fn every_scheduling_and_execution_limit_changes_policy_identity() {
        let service = service();
        let expected = policy_identity(&service).unwrap();
        for mutate in [
            |s: &mut Service| s.coordinator.workers += 1,
            |s: &mut Service| s.coordinator.planning_workers += 1,
            |s: &mut Service| s.coordinator.queue_limit += 1,
            |s: &mut Service| s.coordinator.aging_seconds += 1,
            |s: &mut Service| s.coordinator.planning_timeout_seconds += 1,
            |s: &mut Service| s.native.timeout_seconds += 1,
            |s: &mut Service| s.native.query_timeout_seconds += 1,
            |s: &mut Service| s.memory_max.push('0'),
            |s: &mut Service| s.admission_contract.push('x'),
            |s: &mut Service| s.native.substitutes = !s.native.substitutes,
            |s: &mut Service| s.coordinator.state_dir.push("changed"),
        ] {
            let mut changed = service.clone();
            mutate(&mut changed);
            assert_ne!(policy_identity(&changed).unwrap(), expected);
        }
        let mut changed = service;
        changed.coordinator.policy = "digest-is-not-part-of-itself".into();
        assert_eq!(policy_identity(&changed).unwrap(), expected);
    }

    #[test]
    fn retained_native_siblings_match_the_archived_request_ownership() {
        let root = Goal {
            derivation: "root.drv".into(),
            output: "out".into(),
        };
        let headers = Goal {
            derivation: "shared.drv".into(),
            output: "dev".into(),
        };
        let sibling = Goal {
            output: "out".into(),
            ..headers.clone()
        };
        let request = Request {
            attempt: "murph".into(),
            target: "murph".into(),
            source: "source#murph".into(),
            roots: BTreeSet::from([root.clone()]),
            activates: true,
        };
        let definition = |path: &str, dependencies| Definition {
            output_path: path.into(),
            dependencies,
            operation: Operation::Restore,
        };
        let graph = Graph::from([
            (
                root,
                definition("root-out", BTreeSet::from([headers.clone()])),
            ),
            (headers, definition("shared-dev", BTreeSet::new())),
            (sibling.clone(), definition("unused-out", BTreeSet::new())),
        ]);
        let mut archived = graph.clone();
        archived.remove(&sibling);
        assert_eq!(
            request_paths(&request, &graph),
            request_paths(&request, &archived)
        );
        assert_eq!(
            request_paths(&request, &graph),
            BTreeSet::from([
                "source".into(),
                "root.drv".into(),
                "root-out".into(),
                "shared.drv".into(),
                "shared-dev".into(),
            ])
        );
    }

    #[test]
    fn dangling_default_connection_is_not_an_absent_contract() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("connection.json");
        symlink(temp.path().join("missing.json"), &path).unwrap();
        assert!(Connection::discover("atlas", None, &path).is_err());
    }

    #[test]
    fn retirement_preserves_unrelated_store_shaped_roots() {
        use std::os::unix::fs::DirBuilderExt;
        let temp = tempfile::tempdir().unwrap();
        let mut native = service().native;
        native.gc_roots = temp.path().join("roots");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&native.gc_roots)
            .unwrap();
        let backend = NixBackend(native);
        let request = Request {
            attempt: "a".into(),
            target: "builder".into(),
            source: "source".into(),
            roots: BTreeSet::new(),
            activates: false,
        };
        let namespace = backend.request_native(&request, true).unwrap().gc_roots;
        let foreign = "/nix/store/11111111111111111111111111111111-foreign";
        let root = namespace.join(Path::new(foreign).file_name().unwrap());
        symlink(foreign, &root).unwrap();
        assert!(backend.release(&request, &Graph::new()).is_err());
        assert_eq!(std::fs::read_link(root).unwrap(), Path::new(foreign));
    }

    #[test]
    fn request_retirement_releases_only_its_own_namespace_and_is_retryable() {
        let temp = tempfile::tempdir().unwrap();
        let mut native = service().native;
        native.gc_roots = temp.path().join("roots");
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&native.gc_roots)
                .unwrap();
        }
        let backend = NixBackend(native);
        let shared = "/nix/store/00000000000000000000000000000000-shared";
        let request = |attempt: &str| Request {
            attempt: attempt.into(),
            target: "builder".into(),
            source: shared.into(),
            roots: BTreeSet::new(),
            activates: false,
        };
        let a = request("a");
        let b = request("b");
        let root_a = backend.request_native(&a, true).unwrap().gc_roots;
        let root_b = backend.request_native(&b, true).unwrap().gc_roots;
        for directory in [&root_a, &root_b] {
            symlink(
                shared,
                directory.join(Path::new(shared).file_name().unwrap()),
            )
            .unwrap();
        }
        backend.release(&a, &Graph::new()).unwrap();
        backend.release(&a, &Graph::new()).unwrap();
        assert!(!root_a.exists());
        assert_eq!(
            std::fs::read_link(root_b.join(Path::new(shared).file_name().unwrap())).unwrap(),
            Path::new(shared)
        );
        std::fs::write(root_b.join("unexpected"), "foreign data").unwrap();
        assert!(backend.release(&b, &Graph::new()).is_err());
        assert!(root_b.join("unexpected").exists());
    }
}

/// Independent toolbelt frontend over the same engine used by consumers.
#[cfg(feature = "cli")]
pub mod cli {
    use super::*;
    use clap::Subcommand;
    use runtime::Command;
    use std::io::Write;
    use std::process::ExitCode;
    /// Typed coordinator operations; service execution never accepts shell commands.
    #[derive(Subcommand)]
    pub enum TrainCommand {
        /// Start the builder-local coordinator (normally managed by systemd).
        Serve {
            /// Immutable service/backend configuration.
            #[arg(long)]
            config: PathBuf,
        },
        /// Inspect worker count and the durable activation fence.
        Status {
            /// Deployment-generated client connection.
            #[arg(long)]
            connection: PathBuf,
            /// Inspect one deployment request when supplied.
            #[arg(long)]
            attempt: Option<String>,
        },
        /// Cancel one request's interest; shared running work finishes normally.
        Cancel {
            /// Deployment-generated client connection.
            #[arg(long)]
            connection: PathBuf,
            /// Deployment attempt to cancel.
            #[arg(long)]
            attempt: String,
        },
        /// Retry a failed/cancelled request; the caller must rerun live safety stages.
        Retry {
            /// Deployment-generated client connection.
            #[arg(long)]
            connection: PathBuf,
            /// Deployment attempt to retry.
            #[arg(long)]
            attempt: String,
        },
        /// Archive a terminal request and release its train-owned roots after drain.
        Retire {
            /// Deployment-generated client connection.
            #[arg(long)]
            connection: PathBuf,
            /// Exact terminal deployment attempt.
            #[arg(long)]
            attempt: String,
        },
        /// Release a drained fence after verifying the activation owner's outcome.
        ReleaseFence {
            /// Deployment-generated client connection.
            #[arg(long)]
            connection: PathBuf,
            /// Exact retained fence token reported by status.
            #[arg(long)]
            token: String,
        },
        /// Archive a stopped, drained, fenced terminal train before a policy upgrade.
        Rollover {
            /// Immutable original service configuration retained before activation.
            #[arg(long)]
            previous_config: PathBuf,
            /// Newly deployed immutable service configuration.
            #[arg(long)]
            config: PathBuf,
            /// Exact retained activation fence token. Pending requests must be cancelled first.
            #[arg(long)]
            token: String,
        },
    }
    /// Execute the independent frontend.
    pub fn run(command: TrainCommand) -> Result<ExitCode, String> {
        let reply = match command {
            TrainCommand::Serve { config } => {
                let service = serde_json::from_reader(
                    std::fs::File::open(config).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                let stop = Arc::new(AtomicBool::new(false));
                for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
                    signal_hook::flag::register(signal, Arc::clone(&stop))
                        .map_err(|e| e.to_string())?;
                }
                serve(service, stop)?;
                return Ok(ExitCode::SUCCESS);
            }
            TrainCommand::Status {
                connection,
                attempt,
            } => Connection::load(&connection)?
                .client()
                .call(attempt.map_or(Command::Inspect, Command::Status))?,
            TrainCommand::Cancel {
                connection,
                attempt,
            } => Connection::load(&connection)?
                .client()
                .call(Command::Cancel(attempt))?,
            TrainCommand::Retry {
                connection,
                attempt,
            } => Connection::load(&connection)?
                .client()
                .call(Command::Retry(attempt))?,
            TrainCommand::Retire {
                connection,
                attempt,
            } => Connection::load(&connection)?
                .client()
                .call(Command::Retire(attempt))?,
            TrainCommand::ReleaseFence { connection, token } => Connection::load(&connection)?
                .client()
                .call(Command::ReleaseFence(token))?,
            TrainCommand::Rollover {
                previous_config,
                config,
                token,
            } => {
                let load = |path: &Path| -> Result<Service, String> {
                    serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())
                };
                let receipt = rollover(load(&previous_config)?, load(&config)?, &token)?;
                writeln!(
                    std::io::stdout().lock(),
                    "{}",
                    serde_json::json!({"rolloverReceipt": receipt})
                )
                .map_err(|e| e.to_string())?;
                return Ok(ExitCode::SUCCESS);
            }
        };
        let mut stdout = std::io::stdout().lock();
        serde_json::to_writer_pretty(&mut stdout, &reply).map_err(|e| e.to_string())?;
        writeln!(stdout).map_err(|e| e.to_string())?;
        Ok(ExitCode::SUCCESS)
    }
}
