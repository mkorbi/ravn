use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    /// Build a user turn with optional text plus zero or more images
    /// (Phase 5.6). Empty/blank text is dropped so an image-only turn is valid.
    pub fn user_multimodal(text: Option<String>, images: Vec<ImageContent>) -> Self {
        let mut content = Vec::new();
        if let Some(t) = text {
            if !t.trim().is_empty() {
                content.push(ContentBlock::Text { text: t });
            }
        }
        for image in images {
            content.push(ContentBlock::Image { image });
        }
        Self {
            role: Role::User,
            content,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },

    /// Assistant requests a tool invocation.
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },

    /// Result of a previous tool invocation, attached to a user-role message.
    /// `trustworthy = false` triggers prompt-injection-safe rendering in adapters.
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
        trustworthy: bool,
    },

    /// Anthropic Extended Thinking block — must be preserved across turns
    /// to keep the prompt cache valid. The `signature` is provider-opaque.
    Thinking {
        thinking: String,
        signature: Option<String>,
    },

    /// Image input (Phase 5.6) — read by the vision-capable model. Only valid
    /// on user-role turns.
    Image { image: ImageContent },
}

/// Source of an image attached to a user turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageContent {
    /// A remote image URL the provider fetches.
    Url { url: String },
    /// Base64-encoded image bytes plus its MIME type (e.g. `image/png`).
    Base64 { media_type: String, data: String },
}

impl ImageContent {
    /// Build from a CLI arg: an `http(s)://` URL, or a local file path which is
    /// read and base64-encoded (media type inferred from the extension).
    pub async fn from_path_or_url(arg: &str) -> Result<Self, String> {
        let arg = arg.trim();
        if arg.starts_with("http://") || arg.starts_with("https://") {
            return Ok(ImageContent::Url {
                url: arg.to_string(),
            });
        }
        let path = std::path::Path::new(arg);
        let media_type = media_type_for(path)
            .ok_or_else(|| format!("unsupported image type (use png/jpg/gif/webp): {arg}"))?;
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| format!("read {arg}: {e}"))?;
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Ok(ImageContent::Base64 {
            media_type: media_type.to_string(),
            data,
        })
    }
}

/// Map a file extension to a supported image MIME type.
pub fn media_type_for(path: &std::path::Path) -> Option<&'static str> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())?;
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn image_block_round_trips() {
        let b = ContentBlock::Image {
            image: ImageContent::Base64 {
                media_type: "image/png".into(),
                data: "abc".into(),
            },
        };
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(v["type"], "image");
        assert_eq!(v["image"]["base64"]["media_type"], "image/png");
        let back: ContentBlock = serde_json::from_value(v).unwrap();
        assert!(matches!(back, ContentBlock::Image { .. }));
    }

    #[test]
    fn url_image_serializes() {
        let v = serde_json::to_value(ImageContent::Url {
            url: "https://x/y.png".into(),
        })
        .unwrap();
        assert_eq!(v["url"]["url"], "https://x/y.png");
    }

    #[test]
    fn media_type_mapping() {
        assert_eq!(media_type_for(Path::new("a.PNG")), Some("image/png"));
        assert_eq!(media_type_for(Path::new("a.jpeg")), Some("image/jpeg"));
        assert_eq!(media_type_for(Path::new("a.txt")), None);
        assert_eq!(media_type_for(Path::new("noext")), None);
    }

    #[test]
    fn user_multimodal_drops_blank_text_keeps_image() {
        let m = Message::user_multimodal(
            Some("   ".into()),
            vec![ImageContent::Url { url: "u".into() }],
        );
        assert_eq!(m.content.len(), 1);
        assert!(matches!(m.content[0], ContentBlock::Image { .. }));
    }
}
