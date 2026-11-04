//! The MCP server (rmcp, behind this thin adapter): the six tools and the
//! instructions line. It speaks what rmcp speaks: the 2025-06-18 `initialize`
//! handshake and Claude Code's 2026-07-28 `server/discover`. Everything rmcp
//! sees is here; the tools themselves know nothing of rmcp.

use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, DiscoverResult,
    Implementation, InitializeRequestParams, InitializeResult, ListToolsResult,
    PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig, SubscriptionFilter,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::Value;

use super::Tools;
use super::args::{parse, refused};
use super::tools::{self, Name, text::INSTRUCTIONS};

/// The server's name.
pub const SERVER: &str = "herdr-inbox";

/// The server: the tools' bodies behind it.
pub struct Server {
    pub tools: Arc<dyn Tools>,
}

/// A tool result as the Python server returns a dict: its JSON as indented
/// text (pydantic's `to_json(indent=2)`: non-ASCII kept) and as structured
/// content.
pub fn result(v: Value) -> CallToolResult {
    let text = serde_json::to_string_pretty(&v).unwrap_or_default();
    let mut r = CallToolResult::success(vec![ContentBlock::text(text)]);
    r.structured_content = Some(v);
    r
}

/// A tool error: its text, `isError: true`.
fn failed(text: String) -> CallToolResponse {
    CallToolResult::error(vec![ContentBlock::text(text)]).into()
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        // what the Python server (mcp 2.x) advertises under 2026-07-28: tools, prompts and
        // resources, each with list-changed notifications, resources with subscriptions
        let caps = ServerCapabilities::builder()
            .enable_prompts()
            .enable_prompts_list_changed()
            .enable_resources()
            .enable_resources_list_changed()
            .enable_resources_subscribe()
            .enable_tools()
            .enable_tool_list_changed()
            .build();
        ServerConfig::new(caps)
            .with_server_info(Implementation::new(SERVER, ""))
            .with_instructions(INSTRUCTIONS)
    }

    /// The 2025-06-18 handshake, as the Python server answers it: the same
    /// capabilities with no list-change notifications and no subscriptions.
    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        context.peer.set_peer_info(request.clone());
        let mut info = self.negotiate_initialize(&request)?;
        let caps = &mut info.capabilities;
        if let Some(t) = caps.tools.as_mut() {
            t.list_changed = Some(false);
        }
        if let Some(p) = caps.prompts.as_mut() {
            p.list_changed = Some(false);
        }
        if let Some(r) = caps.resources.as_mut() {
            r.list_changed = Some(false);
            r.subscribe = Some(false);
        }
        Ok(info)
    }

    /// Discovery (2026-07-28), as the Python server answers it: only that version.
    async fn discover(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> Result<DiscoverResult, ErrorData> {
        Ok(DiscoverResult::from_server_info(
            vec![ProtocolVersion::V_2026_07_28],
            self.get_info(),
        ))
    }

    /// Claude Code listens for list changes (`subscriptions/listen`); nothing
    /// ever changes, but the subscription is acknowledged as Python does.
    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(requested.clone())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools::list()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let Some(name) = Name::parse(&request.name) else {
            return Ok(failed(format!("Unknown tool: {}", request.name)));
        };
        let call = match parse(name, &request.arguments.unwrap_or_default()) {
            Ok(call) => call,
            Err(why) => return Ok(failed(refused(name, &why))),
        };
        // the tools block (SQLite, herdr's socket, the retry waits): off the runtime's thread
        let tools = Arc::clone(&self.tools);
        let done = tokio::task::spawn_blocking(move || tools.call(call)).await;
        Ok(match done {
            Ok(Ok(v)) => result(v).into(),
            Ok(Err(e)) => failed(format!("Error executing tool {}: {e}", name.as_str())),
            Err(e) => failed(format!("Error executing tool {}: {e}", name.as_str())),
        })
    }
}
