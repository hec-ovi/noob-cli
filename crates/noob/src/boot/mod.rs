//! Session bootstrap shared by every surface that assembles an Agent: the
//! REPL, exec, child and serve. Argv value parsing lives here too, because
//! every one of those surfaces reads the same flag shapes.

use noob_provider::http::{Client, Timeouts};
use noob_provider::types::Overrides;

use crate::agent::{Agent, prompt};
use crate::session::{ReplayReport, Session};
use crate::tools::ToolCtx;
use crate::ui::Ui;
use crate::{config, emit, mcp, skills, subagent, tools};

/// A flag's value must exist and must not look like another flag; consuming
/// blindly turns one forgotten value into a silent misconfig.
pub(crate) fn value_for(flag: &str, next: Option<&String>, usage: &str) -> Result<String, String> {
    match next {
        Some(v) if !v.starts_with('-') => Ok(v.clone()),
        _ => Err(format!("noob: {flag} needs a value; {usage}")),
    }
}

// ---------------------------------------------------------------------------
// Session bootstrap shared by the REPL and exec
// ---------------------------------------------------------------------------

pub(crate) fn model_label(config_dir: &std::path::Path, ov: &Overrides) -> String {
    ov.model
        .clone()
        .or_else(|| config::setting(config_dir, "NOOB_MODEL"))
        .unwrap_or_else(|| "default".to_string())
}

pub(crate) struct BootArgs {
    pub(crate) ov: Overrides,
    pub(crate) yolo: bool,
    /// Start in plan mode (read-only tools until /go).
    pub(crate) plan: bool,
    /// Register only the read-only set (read-only children).
    pub(crate) read_only: bool,
    /// Nonmutating research child: local reads plus the websearch CLI tool.
    pub(crate) web_only: bool,
    /// Relay sub-agent stderr as `[subagent] ...` diagnostics.
    pub(crate) verbose: bool,
    /// Skill names already loaded by ancestor agents. A child filters these
    /// from discovery so orchestration skills cannot recursively invoke
    /// themselves through nested delegation.
    pub(crate) excluded_skills: Vec<String>,
    /// None = no persistence; Some(None) = fresh id; Some(Some(id)) = resume.
    pub(crate) session: Option<Option<String>>,
    /// Where protocol frames go. None means whatever `NOOB_EMIT` says, which
    /// is off for every ordinary surface. `serve` overrides it with stdout,
    /// because there the stream is not a side-channel, it is the output.
    pub(crate) emitter: Option<emit::Emitter>,
}

impl BootArgs {
    pub(crate) fn new(ov: Overrides, yolo: bool, plan: bool, session: Option<Option<String>>) -> BootArgs {
        BootArgs {
            ov,
            yolo,
            plan,
            read_only: false,
            web_only: false,
            verbose: false,
            excluded_skills: Vec::new(),
            session,
            emitter: None,
        }
    }
}

/// NOOB_DEPTH: 0 for the user's agent; children run at parent+1.
pub(crate) fn current_depth() -> u32 {
    std::env::var("NOOB_DEPTH")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// Returns the assembled agent and whether an explicit `--resume <id>` missed
/// (the id was given but no session file existed), so the REPL can tell the
/// human it started fresh. The flag is display-only; `exec`/`child` ignore it.
pub(crate) fn bootstrap(boot: BootArgs, ui: &mut Ui) -> Result<(Agent, bool), String> {
    let config_dir = config::config_dir();
    let mut ov = boot.ov;
    if ov.base_url.is_none()
        && config::setting(&config_dir, "NOOB_BASE_URL").is_none()
        && let Some(found) = config::autodetect_base_url(&config_dir)
    {
        ui.note(&format!("using {found} (autodetected)"));
        ov.base_url = Some(found);
    }
    let workspace = std::env::current_dir()
        .and_then(|d| d.canonicalize())
        .map_err(|e| format!("cannot resolve the working directory: {e}"))?;
    let (sandbox_mode, sandbox_label) = config::detect_sandbox(&config_dir, boot.yolo);
    let sandbox = tools::guard::Sandbox::from(sandbox_mode);

    // The env-block model label follows the same precedence as the real
    // request (flag > env > .env) but is independent of base-url resolution,
    // so `debug prompt` and a live session print the identical head.
    let model = model_label(&config_dir, &ov);
    let model_name = model.clone();
    let skill_paths = config::skill_paths(&config_dir, &workspace);
    let mut discovered = skills::discover(&workspace, &config_dir, &skill_paths);
    if !boot.excluded_skills.is_empty() {
        discovered.retain(|skill| !boot.excluded_skills.contains(&skill.name));
    }
    let (mut mcp_servers, mcp_warnings) = mcp::config::load(&workspace, &config_dir);
    for warning in &mcp_warnings {
        ui.note(&format!("mcp: {warning}"));
    }
    if boot.web_only && !tools::websearch::available() {
        return Err(format!(
            "a web research child needs the {:?} CLI on PATH; install it with \
             `uv tool install websearch-skill`, or delegate with tools \"all\" instead",
            tools::websearch::PROGRAM
        ));
    }
    // A web child talks to the web through its own tool, never through a
    // configured MCP server: the set of servers is the user's, and a research
    // child restricted to reading must not inherit whatever they registered.
    if boot.web_only {
        mcp_servers.clear();
    }
    let inputs = prompt::PromptInputs {
        cwd: workspace.display().to_string(),
        model,
        sandbox: sandbox_label,
        agents: prompt::load_prompt_md(&config_dir, "AGENTS.md"),
        tools: prompt::load_prompt_md(&config_dir, "TOOLS.md"),
        project_agents: prompt::load_prompt_md(&workspace, "AGENTS.md"),
        skills_index: skills::index(&discovered),
        // A read-only child has no mcp_call; naming servers would only
        // tempt it into calls it cannot make.
        mcp_line: if boot.read_only && !boot.web_only {
            None
        } else {
            prompt::mcp_line(&mcp_servers)
        },
    };
    let system = prompt::assemble(&inputs);
    // Registered set is decided here and stays byte-stable for the session:
    // the skill tool exists only when discovery found at least one skill,
    // the MCP pair only when mcp.json configured at least one server, and
    // the task tool only below the recursion ceiling with the full set.
    let depth = current_depth();
    let mut tool_specs = tools::specs();
    if !discovered.is_empty() {
        tool_specs.push(tools::skill::spec());
    }
    if !mcp_servers.is_empty() {
        tool_specs.push(tools::mcp::connect_spec());
        tool_specs.push(tools::mcp::call_spec());
    }
    let websearch = tools::websearch::available();
    if websearch {
        tool_specs.push(tools::websearch::spec());
    }
    let with_task = depth < subagent::MAX_DEPTH && !boot.read_only;
    if with_task {
        tool_specs.push(subagent::spec());
    }
    if boot.web_only {
        tool_specs.retain(|t| tools::WEB_RESEARCH_SET.contains(&t.name.as_str()));
    } else if boot.read_only {
        tool_specs.retain(|t| tools::READ_ONLY_SET.contains(&t.name.as_str()));
    }
    // Taken before `workspace` is moved into the context: a watcher needs to
    // know which tree the paths in every later frame are relative to.
    let workspace_label = workspace.to_string_lossy().into_owned();
    let mut tool_ctx = ToolCtx::new(workspace, sandbox);
    tool_ctx.core.caps = if config::tool_caps_lifted(&config_dir) {
        tools::truncate::Caps::uncapped()
    } else {
        tools::truncate::Caps::default()
    };
    tool_ctx.fs.read_dedup = config::read_dedup(&config_dir);
    tool_ctx.skills.list = discovered;
    tool_ctx.websearch = websearch;
    if !mcp_servers.is_empty() && (!boot.read_only || boot.web_only) {
        tool_ctx.mcp = Some(mcp::Mcp::new(mcp_servers));
    }
    if with_task {
        tool_ctx.task = Some(subagent::TaskCfg {
            depth,
            concurrency: config::task_concurrency(&config_dir, subagent::DEFAULT_CONCURRENCY),
            max_turns: config::task_max_turns(&config_dir, subagent::DEFAULT_MAX_TURNS),
            tools_default: config::task_tools(&config_dir, subagent::DEFAULT_TOOLS),
            wall_clock: config::task_wall_clock(&config_dir, subagent::DEFAULT_WALL_CLOCK_S),
            verbose: boot.verbose,
            overrides: ov.clone(),
            yolo: boot.yolo,
            ancestor_skills: boot.excluded_skills.clone(),
            background: None,
        });
    }

    let (session, replayed, resume_missed, replay_report) = match boot.session {
        None => (None, Vec::new(), false, ReplayReport::default()),
        Some(id) => {
            let requested = id.is_some();
            let resolved = match id.as_deref() {
                Some("latest") => Session::latest_id(&config_dir)?,
                _ => id,
            };
            let (s, items, existed, report) = Session::open(&config_dir, resolved.as_deref())?;
            (Some(s), items, requested && !existed, report)
        }
    };
    if let Some(warning) = replay_report.warning() {
        ui.error(&warning);
    }
    // A resumed session keeps counting where it left off: the readout is about
    // the session, not about this process.
    if let Some(session) = session.as_ref() {
        ui.seed_tokens(session.tokens());
    }
    // The side-channel opens here, once every surface has been decided and
    // before the first frame anything could emit. Off unless NOOB_EMIT names
    // a file, in which case no byte on any existing surface moves.
    let emitter = boot.emitter.unwrap_or_else(emit::Emitter::from_env);
    emitter.send(noob_proto::Event::SessionStart {
        id: session
            .as_ref()
            .map(|s| s.id().to_string())
            .unwrap_or_default(),
        workspace: workspace_label,
        model: model_name,
        resumed: !replayed.is_empty(),
    });
    tool_ctx.core.emitter = emitter;
    let mut agent = Agent::new(
        Client::new(Timeouts::default()),
        config_dir.clone(),
        ov,
        system,
        tool_specs,
        replayed,
        tool_ctx,
        session,
        config::ctx_tokens(&config_dir),
    );
    // 0 (the default) is unbounded; NOOB_MAX_ROUNDS puts a ceiling back.
    agent.max_rounds = config::max_rounds(&config_dir, 0);
    let orphaned = agent.repair_orphaned_background_results();
    if orphaned > 0 {
        agent.show_session_warning(ui);
        ui.note(&format!(
            "recovered {orphaned} unfinished background sub-agent(s) as canceled"
        ));
    }
    if boot.plan {
        agent.enter_plan(ui);
    }
    // Read-only children: the schemas are already filtered above; this arms
    // the dispatcher's defense in depth against hallucinated mutations.
    agent.read_only = boot.read_only;
    Ok((agent, resume_missed))
}
