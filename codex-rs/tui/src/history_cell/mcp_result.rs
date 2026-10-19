//! Width-independent display content retained after an MCP call completes.

use crate::exec_cell::TOOL_CALL_MAX_LINES;
use crate::text_formatting::format_and_truncate_tool_result;
use base64::Engine;
use codex_protocol::mcp::CallToolResult;
use image::DynamicImage;
use image::ImageReader;
use rmcp::model::ContentBlock;
use rmcp::model::ResourceContents;
use serde::Deserialize;
use std::borrow::Cow;
use std::io::Cursor;
use tracing::error;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum McpResultKind {
    Standard,
    NodeRepl,
}

#[derive(Debug)]
pub(super) struct McpToolResult {
    pub(super) content: Vec<McpContentBlock>,
    pub(super) is_error: bool,
    pub(super) has_image: bool,
}

#[derive(Debug)]
pub(super) struct McpContentBlock {
    display: McpContentDisplay,
    original_text: Option<String>,
}

#[derive(Debug)]
enum McpContentDisplay {
    Text(String),
    Summary(Cow<'static, str>),
    Json(String),
}

impl McpToolResult {
    pub(super) fn new(result: CallToolResult, kind: McpResultKind) -> Self {
        let mut has_image = false;
        let content = result
            .content
            .into_iter()
            .map(|block| {
                let parsed = ContentBlock::deserialize(&block);
                let original_text = match (&parsed, kind) {
                    (Ok(ContentBlock::Text(_)), _) | (_, McpResultKind::Standard) => None,
                    (_, McpResultKind::NodeRepl) => block
                        .get("text")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                };
                let display = match parsed {
                    Ok(ContentBlock::Text(text)) => McpContentDisplay::Text(text.text),
                    Ok(ContentBlock::Image(image)) => {
                        if !has_image {
                            has_image = decode_mcp_image(&image.data).is_some();
                        }
                        McpContentDisplay::Summary("<image content>".into())
                    }
                    Ok(ContentBlock::Audio(_)) => {
                        McpContentDisplay::Summary("<audio content>".into())
                    }
                    Ok(ContentBlock::Resource(resource)) => {
                        let summary = match resource.resource {
                            ResourceContents::TextResourceContents { uri, .. }
                            | ResourceContents::BlobResourceContents { uri, .. } => {
                                format!("embedded resource: {uri}").into()
                            }
                            _ => "<unknown embedded resource>".into(),
                        };
                        McpContentDisplay::Summary(summary)
                    }
                    Ok(ContentBlock::ResourceLink(link)) => {
                        McpContentDisplay::Summary(format!("link: {}", link.uri).into())
                    }
                    Ok(_) | Err(_) => McpContentDisplay::Json(block.to_string()),
                };
                McpContentBlock {
                    display,
                    original_text,
                }
            })
            .collect();

        Self {
            content,
            is_error: result.is_error.unwrap_or(false),
            has_image,
        }
    }
}

impl McpContentBlock {
    pub(super) fn text(&self) -> Option<&str> {
        match &self.display {
            McpContentDisplay::Text(text) => Some(text),
            McpContentDisplay::Summary(_) | McpContentDisplay::Json(_) => {
                self.original_text.as_deref()
            }
        }
    }

    pub(super) fn render(&self, width: usize) -> String {
        match &self.display {
            McpContentDisplay::Text(text) | McpContentDisplay::Json(text) => {
                format_and_truncate_tool_result(text, TOOL_CALL_MAX_LINES, width)
            }
            McpContentDisplay::Summary(summary) => summary.to_string(),
        }
    }
}

fn decode_mcp_image(data: &str) -> Option<DynamicImage> {
    let base64_data = if let Some(data_url) = data.strip_prefix("data:") {
        data_url.split_once(',')?.1
    } else {
        data
    };
    let raw_data = base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .map_err(|e| {
            error!("Failed to decode image data: {e}");
            e
        })
        .ok()?;
    let reader = ImageReader::new(Cursor::new(raw_data))
        .with_guessed_format()
        .map_err(|e| {
            error!("Failed to guess image format: {e}");
            e
        })
        .ok()?;

    reader
        .decode()
        .map_err(|e| {
            error!("Image decoding failed: {e}");
            e
        })
        .ok()
}
