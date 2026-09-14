//! MCP surface: the read and verify commands an agent already runs through the
//! CLI, exposed as tools so a harness can call them without shelling out.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use forgeguard_core::{
    config::{ForgeGuardConfig, CONFIG_FILE},
    run_changed_gate, run_doctor, run_gate, task_state, GateOptions,
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

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct DoctorRequest {}

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

fn server_spec() -> ServerSpec {
    ServerSpec::stdio(
        "forgeguard",
        "forgeguard",
        vec!["mcp".into(), "serve".into()],
    )
}
