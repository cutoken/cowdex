use crate::history_cell::HistoryCell;
use crate::legacy_core::config::Config;
use crate::terminal_hyperlinks::HyperlinkLine;
use crate::terminal_hyperlinks::adaptive_wrap_hyperlink_lines;
use crate::terminal_hyperlinks::visible_lines;
use crate::wrapping::RtOptions;
use codex_config::types::McpServerTransportConfig;
use ratatui::style::Stylize;
use ratatui::text::Line;
use url::Url;

#[derive(Debug)]
pub(super) struct AgentCronLink(HyperlinkLine);

impl HistoryCell for AgentCronLink {
    fn raw_lines(&self) -> Vec<Line<'static>> {
        crate::history_cell::plain_lines(vec![self.0.line.clone()])
    }

    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        visible_lines(self.display_hyperlink_lines(width))
    }

    fn display_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        if width == 0 {
            return Vec::new();
        }
        adaptive_wrap_hyperlink_lines(std::slice::from_ref(&self.0), RtOptions::new(width.into()))
    }
}

pub(super) fn new_link(config: &Config) -> Option<AgentCronLink> {
    let configured = config.mcp_servers.get().iter().any(|(name, server)| {
        name.to_ascii_lowercase().contains("agentcron")
            || match &server.transport {
                McpServerTransportConfig::StreamableHttp { url, .. } => Url::parse(url)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
                    .is_some_and(|host| host == "agentcron.ai" || host.ends_with(".agentcron.ai")),
                McpServerTransportConfig::Stdio { command, args, .. } => {
                    command.to_ascii_lowercase().contains("agentcron")
                        || args
                            .iter()
                            .any(|arg| arg.to_ascii_lowercase().contains("agentcron"))
                }
            }
    });
    if configured {
        return None;
    }
    let mut line = HyperlinkLine::default();
    line.push_span("Project management: ".dim(), /*destination*/ None);
    line.push_span(
        "agentcron.ai".cyan().underlined(),
        Some("https://agentcron.ai"),
    );
    Some(AgentCronLink(line))
}
