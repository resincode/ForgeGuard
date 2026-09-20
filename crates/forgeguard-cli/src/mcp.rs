//! MCP surface: the read and verify commands an agent already runs through the
//! CLI, exposed as tools so a harness can call them without shelling out.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use forgeguard_core::{
    analyze_impact, architecture,
    config::{ForgeGuardConfig, ScanConfig, CONFIG_FILE},
    delete_project, find_symbols, index_repository, index_status, list_projects,
    memory::{ensure_current, run_query, search_symbols},
    run_changed_gate, run_doctor, run_gate, symbol_card, task_state, trace_path, AgentTarget,
    Detail, Direction, GateOptions, IndexOptions, LspOptions, MemoryStats, RetrievalOptions, Store,
};
use kurir::{Harness, RegistrationOptions, Scope, ServerSpec};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars::JsonSchema,
    tool, tool_handler, tool_router,
    transport::stdio,
    ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
struct ForgeGuardMcp {
    root: PathBuf,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct GateRequest {
    /// Scope findings to changed files instead of the whole repository.
    #[serde(default)]
    changed: bool,
    /// Skip the configured quality commands and run static rules only.
    #[serde(default)]
    no_run: bool,
    /// Git revision to compare against when `changed` is set.
    base: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TaskStatusRequest {
    session: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum DetailArg {
    Metadata,
    Structure,
    Snippet,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum DirectionArg {
    Inbound,
    Outbound,
    Both,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct DoctorRequest {}

#[derive(Debug, Deserialize, JsonSchema)]
struct MemoryFindRequest {
    /// Symbol name, `Type.method`, or a substring.
    query: String,
    /// Maximum hits to return; defaults to 20.
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct MemorySymbolRequest {
    /// Symbol name or `Type.method`.
    query: String,
    /// `metadata`, `structure` (default), `snippet`, or `full`.
    detail: Option<DetailArg>,
    /// Maximum source bytes to return; defaults to 8192.
    max_bytes: Option<usize>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct MemoryImpactRequest {
    /// Git revision to compare against; defaults to the working tree vs HEAD.
    base: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct MemoryArchitectureRequest {}

#[derive(Debug, Deserialize, JsonSchema)]
struct MemoryTraceRequest {
    /// Symbol name or `Type.method` to start from.
    query: String,
    /// `inbound` (callers), `outbound` (callees), or `both` (default).
    direction: Option<DirectionArg>,
    /// Hops to follow, 1 to 5; defaults to 2.
    depth: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct MemoryQueryRequest {
    /// A `MATCH ... RETURN` query. Mutations and raw SQL are rejected.
    query: String,
    /// Maximum rows; defaults to 100.
    limit: Option<usize>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct MemoryIndexRequest {
    /// Reparse every file instead of trusting cached hashes.
    force: Option<bool>,
    /// Ask installed language servers about call sites the static pass could not
    /// attribute to a type. Off by default: it needs external tools.
    lsp: Option<bool>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct MemoryDeleteRequest {
    /// Must be true: deleting the graph cannot be undone without re-indexing.
    #[serde(default)]
    confirm: bool,
}

#[tool_router]
impl ForgeGuardMcp {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "gate",
        description = "Run ForgeGuard static rules and configured quality commands; returns the gate report."
    )]
    async fn gate(&self, Parameters(request): Parameters<GateRequest>) -> Result<String, String> {
        let root = self.root.clone();
        blocking(move || {
            let config = ForgeGuardConfig::load(&root).context(
                "ForgeGuard is not initialized; run `forgeguard init` in the repository first",
            )?;
            if request.changed {
                run_changed_gate(&root, &config, request.no_run, request.base.as_deref())
            } else {
                run_gate(
                    &root,
                    &config,
                    &GateOptions {
                        skip_commands: request.no_run,
                        paths: None,
                    },
                )
            }
        })
        .await
    }

    #[tool(
        name = "doctor",
        description = "Check ForgeGuard configuration, hooks, and required local tools."
    )]
    async fn doctor(&self, Parameters(_): Parameters<DoctorRequest>) -> Result<String, String> {
        let root = self.root.clone();
        blocking(move || {
            let config = if root.join(CONFIG_FILE).exists() {
                Some(ForgeGuardConfig::load(&root)?)
            } else {
                None
            };
            run_doctor(&root, config.as_ref())
        })
        .await
    }

    #[tool(
        name = "task_status",
        description = "Read the ForgeGuard task state for one agent session."
    )]
    async fn task_status(
        &self,
        Parameters(request): Parameters<TaskStatusRequest>,
    ) -> Result<String, String> {
        let root = self.root.clone();
        blocking(move || {
            task_state(&root, &request.session)?.with_context(|| {
                format!("no ForgeGuard task found for session {}", request.session)
            })
        })
        .await
    }

    #[tool(
        name = "memory_find",
        description = "Find symbols in the persistent code graph by name, qualified name, or substring. Returns metadata only: no source is read."
    )]
    async fn memory_find(
        &self,
        Parameters(request): Parameters<MemoryFindRequest>,
    ) -> Result<String, String> {
        let root = self.root.clone();
        blocking(move || {
            let store = ensure_index(&root)?;
            find_symbols(&root, &store, &request.query, request.limit.unwrap_or(20))
        })
        .await
    }

    #[tool(
        name = "memory_search",
        description = "Rank symbols by BM25 over name, signature, and path. Use when the exact symbol name is unknown; use memory_find when it is."
    )]
    async fn memory_search(
        &self,
        Parameters(request): Parameters<MemoryFindRequest>,
    ) -> Result<String, String> {
        let root = self.root.clone();
        blocking(move || {
            let store = ensure_index(&root)?;
            search_symbols(&root, &store, &request.query, request.limit.unwrap_or(20))
        })
        .await
    }

    #[tool(
        name = "memory_trace",
        description = "Walk the call chain from a symbol: inbound (callers), outbound (callees), or both, up to 5 hops."
    )]
    async fn memory_trace(
        &self,
        Parameters(request): Parameters<MemoryTraceRequest>,
    ) -> Result<String, String> {
        let root = self.root.clone();
        blocking(move || {
            let store = ensure_index(&root)?;
            let direction = parse_direction(request.direction);
            trace_path(
                &root,
                &store,
                &request.query,
                direction,
                request.depth.unwrap_or(2),
            )?
            .with_context(|| format!("no indexed symbol matches {}", request.query))
        })
        .await
    }

    #[tool(
        name = "memory_query",
        description = "Run a read-only Cypher-like query over the graph, for example MATCH (f:Function)-[:CALLS]->(g:Function) WHERE f.name = \"main\" RETURN g.qualified. Mutations and raw SQL are rejected."
    )]
    async fn memory_query(
        &self,
        Parameters(request): Parameters<MemoryQueryRequest>,
    ) -> Result<String, String> {
        let root = self.root.clone();
        blocking(move || {
            let store = ensure_index(&root)?;
            run_query(&root, &store, &request.query, request.limit.unwrap_or(100))
        })
        .await
    }

    #[tool(
        name = "memory_symbol",
        description = "Retrieve one symbol at the requested detail: metadata, structure (callers, callees, dependents, tests), snippet, or full file. Prefer the smallest level that answers the question."
    )]
    async fn memory_symbol(
        &self,
        Parameters(request): Parameters<MemorySymbolRequest>,
    ) -> Result<String, String> {
        let root = self.root.clone();
        blocking(move || {
            let store = ensure_index(&root)?;
            let options = RetrievalOptions {
                detail: parse_detail(request.detail),
                max_bytes: request.max_bytes.unwrap_or(8192),
            };
            symbol_card(&root, &store, &request.query, &options)?
                .with_context(|| format!("no indexed symbol matches {}", request.query))
        })
        .await
    }

    #[tool(
        name = "memory_impact",
        description = "Map the current Git diff to changed symbols, their callers, dependent files, and related tests."
    )]
    async fn memory_impact(
        &self,
        Parameters(request): Parameters<MemoryImpactRequest>,
    ) -> Result<String, String> {
        let root = self.root.clone();
        blocking(move || {
            let store = ensure_index(&root)?;
            analyze_impact(&root, &store, request.base.as_deref())
        })
        .await
    }

    #[tool(
        name = "memory_index",
        description = "Build or refresh the code graph. Other memory tools index on first use, so call this only to force a rebuild or to warm the graph up front."
    )]
    async fn memory_index(
        &self,
        Parameters(request): Parameters<MemoryIndexRequest>,
    ) -> Result<String, String> {
        let root = self.root.clone();
        blocking(move || {
            let config = scan_config(&root)?;
            index_repository(
                &root,
                &config,
                &IndexOptions {
                    force: request.force.unwrap_or(false),
                    paths: None,
                    lsp: request.lsp.unwrap_or(false).then(LspOptions::default),
                },
            )
        })
        .await
    }

    #[tool(
        name = "memory_status",
        description = "Report whether this repository has an index, how large it is, and whether it was built against the checked-out commit."
    )]
    async fn memory_status(
        &self,
        Parameters(_): Parameters<MemoryArchitectureRequest>,
    ) -> Result<String, String> {
        let root = self.root.clone();
        blocking(move || index_status(&root)).await
    }

    #[tool(
        name = "memory_projects",
        description = "List repositories on this machine that have a ForgeGuard code graph."
    )]
    async fn memory_projects(
        &self,
        Parameters(_): Parameters<MemoryArchitectureRequest>,
    ) -> Result<String, String> {
        blocking(list_projects).await
    }

    #[tool(
        name = "memory_stats",
        description = "Report indexing and retrieval counters, including the source bytes the graph avoided reading."
    )]
    async fn memory_stats(
        &self,
        Parameters(_): Parameters<MemoryArchitectureRequest>,
    ) -> Result<String, String> {
        let root = self.root.clone();
        blocking(move || Ok(MemoryStats::load(&root))).await
    }

    #[tool(
        name = "memory_delete",
        description = "Delete this repository's code graph. Requires confirm=true. Source files are never touched; the graph can be rebuilt by indexing again."
    )]
    async fn memory_delete(
        &self,
        Parameters(request): Parameters<MemoryDeleteRequest>,
    ) -> Result<String, String> {
        let root = self.root.clone();
        blocking(move || {
            if !request.confirm {
                anyhow::bail!("refusing to delete the index without confirm=true");
            }
            Ok(serde_json::json!({ "deleted": delete_project(&root)? }))
        })
        .await
    }

    #[tool(
        name = "memory_architecture",
        description = "Summarise the repository in one request: languages, modules, layers, entry points, and external dependencies."
    )]
    async fn memory_architecture(
        &self,
        Parameters(_): Parameters<MemoryArchitectureRequest>,
    ) -> Result<String, String> {
        let root = self.root.clone();
        blocking(move || {
            let store = ensure_index(&root)?;
            architecture(&root, &store)
        })
        .await
    }
}

fn parse_detail(value: Option<DetailArg>) -> Detail {
    match value {
        None | Some(DetailArg::Structure) => Detail::Structure,
        Some(DetailArg::Metadata) => Detail::Metadata,
        Some(DetailArg::Snippet) => Detail::Snippet,
        Some(DetailArg::Full) => Detail::Full,
    }
}

fn parse_direction(value: Option<DirectionArg>) -> Direction {
    match value {
        None | Some(DirectionArg::Both) => Direction::Both,
        Some(DirectionArg::Inbound) => Direction::Inbound,
        Some(DirectionArg::Outbound) => Direction::Outbound,
    }
}

fn scan_config(root: &Path) -> Result<ScanConfig> {
    ForgeGuardConfig::scan_settings(root)
}

/// Keep the graph current without making the agent call an index tool first: an
/// empty index is built, an existing one is refreshed from the Git diff. A
/// refresh failure (no Git, for example) leaves the existing index in use.
fn ensure_index(root: &Path) -> Result<Store> {
    ensure_current(root, &scan_config(root)?)
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ForgeGuardMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// Gate runs spawn quality commands and walk the tree, so they stay off the
/// async executor.
async fn blocking<T, F>(work: F) -> Result<String, String>
where
    T: Serialize + Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    let value = tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("{error:#}"))?;
    serde_json::to_string(&value).map_err(|error| error.to_string())
}

pub fn serve(root: PathBuf) -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to start the MCP runtime")?
        .block_on(async move {
            let service = ForgeGuardMcp::new(root)
                .serve(stdio())
                .await
                .context("failed to start the MCP server")?;
            service.waiting().await.context("MCP server stopped")?;
            Ok(())
        })
}

pub fn register(
    root: &Path,
    client: &str,
    project: bool,
    dry_run: bool,
    force: bool,
) -> Result<()> {
    let harness: Harness = client.parse()?;
    let spec = server_spec();
    if harness.is_snippet_only() {
        println!(
            "{}",
            serde_json::to_string_pretty(&kurir::registration::snippet_for(&spec))?
        );
        return Ok(());
    }
    let options = RegistrationOptions {
        scope: if project { Scope::Project } else { Scope::User },
        cwd: root.to_path_buf(),
        force,
        dry_run,
        print: dry_run,
        ..RegistrationOptions::default()
    };
    let result = kurir::register(harness, &spec, &options)?;
    let place = result
        .target
        .as_ref()
        .map_or_else(|| result.harness.clone(), |path| path.display().to_string());
    match result.action.as_str() {
        "registered" => println!("registered ForgeGuard MCP server with {place}"),
        "already-configured" => println!("ForgeGuard MCP server already registered in {place}"),
        _ => {}
    }
    Ok(())
}

/// Harness ids that keep their MCP configuration inside the repository. A
/// harness registered through its own CLI lands in a user-wide file, which is
/// the wrong shape here: every checkout has its own code graph, so the entry
/// has to sit next to the code it describes.
const fn project_harness(agent: AgentTarget) -> Option<&'static str> {
    match agent {
        AgentTarget::Claude => Some("claude-code"),
        AgentTarget::Cursor => Some("cursor"),
        AgentTarget::OpenCode => Some("opencode"),
        AgentTarget::OpenClaw => Some("openclaw"),
        AgentTarget::Antigravity => Some("antigravity-cli"),
        _ => None,
    }
}

/// Harnesses whose MCP entry only exists user-wide. They are named in a hint
/// rather than registered, because one entry would serve every repository.
const fn user_wide_harness(agent: AgentTarget) -> Option<&'static str> {
    match agent {
        AgentTarget::Codex => Some("codex"),
        AgentTarget::Hermes => Some("hermes"),
        AgentTarget::Windsurf => Some("windsurf"),
        AgentTarget::Copilot => Some("copilot-cli"),
        _ => None,
    }
}

/// Register `forgeguard mcp serve` for the agents `init` installed, in this
/// repository only. Best effort: a harness that cannot be written is reported
/// and skipped, because a failed registration must not fail the install.
/// Returns the repository-relative config paths that were touched.
pub fn register_agents(root: &Path, agents: &[AgentTarget], quiet: bool) -> Vec<String> {
    let spec = server_spec();
    let mut written = Vec::new();
    for agent in agents {
        let Some(id) = project_harness(*agent) else {
            // Nothing to write here, but silence would look like success: the
            // agent is installed and its MCP entry is not.
            if let (false, Some(client)) = (quiet, user_wide_harness(*agent)) {
                println!(
                    "  MCP for {client} is user-wide, not per repository: run `forgeguard mcp register --client {client}` to add it"
                );
            }
            continue;
        };
        let Ok(harness) = id.parse::<Harness>() else {
            continue;
        };
        let options = RegistrationOptions {
            scope: Scope::Project,
            cwd: root.to_path_buf(),
            ..RegistrationOptions::default()
        };
        match kurir::register(harness, &spec, &options) {
            Ok(result) => {
                // Git reads `\` in a pattern as an escape, so a Windows path has
                // to reach `.gitignore` with forward slashes.
                let relative = result.target.as_ref().and_then(|path| {
                    path.strip_prefix(root)
                        .ok()
                        .map(|path| path.to_string_lossy().replace('\\', "/"))
                });
                if !quiet {
                    match result.action.as_str() {
                        "registered" => println!(
                            "  MCP registered for {id} in {}",
                            relative.clone().unwrap_or_else(|| id.to_owned())
                        ),
                        "already-configured" => println!("  MCP already registered for {id}"),
                        _ => {}
                    }
                }
                if let Some(relative) = relative {
                    written.push(relative);
                }
            }
            Err(error) => {
                eprintln!("  MCP registration skipped for {id}: {error}");
            }
        }
    }
    written
}

fn server_spec() -> ServerSpec {
    ServerSpec::stdio(
        "forgeguard",
        "forgeguard",
        vec!["mcp".into(), "serve".into()],
    )
}
