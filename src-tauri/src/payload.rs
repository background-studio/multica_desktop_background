use std::{fs, path::Path};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::models::{DisplaySettings, MediaItem, MediaKind};

mod generated {
    include!(concat!(env!("OUT_DIR"), "/payload_assets.rs"));
}

const REVIEW_SHADOW_STYLE_ID: &str = "multica-background-review-shadow-style";
const MAX_INLINE_MEDIA_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevisionInput<'a> {
    sha256: &'a str,
    display: &'a DisplaySettings,
    kind: &'a MediaKind,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PayloadConfig<'a> {
    media_url: &'a str,
    media_kind: &'a MediaKind,
    display: &'a DisplaySettings,
    revision: &'a str,
}

pub struct ActivePayload {
    pub script: String,
    pub revision: String,
}

fn digest(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    format!("{:x}", hasher.finalize())
}

pub fn build_active_payload(
    media: &MediaItem,
    media_path: &Path,
    display: &DisplaySettings,
) -> Result<ActivePayload, String> {
    if media.byte_size > MAX_INLINE_MEDIA_BYTES {
        return Err("背景媒体超过 64 MB 内嵌上限，请选择更小的文件。".to_string());
    }
    let bytes = fs::read(media_path).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_INLINE_MEDIA_BYTES {
        return Err("背景媒体超过 64 MB 内嵌上限，请选择更小的文件。".to_string());
    }
    let file_digest = {
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        format!("{:x}", hasher.finalize())
    };
    let media_url = format!("data:{};base64,{}", media.mime_type, STANDARD.encode(&bytes));
    let revision_input = serde_json::to_vec(&RevisionInput {
        sha256: &file_digest,
        display,
        kind: &media.kind,
    })
    .map_err(|error| error.to_string())?;
    let revision = digest(&[&revision_input]);
    let payload_revision = digest(&[
        revision.as_bytes(),
        generated::BACKGROUND_CSS.as_bytes(),
        generated::REVIEW_SHADOW_CSS.as_bytes(),
    ]);
    let serialized = serde_json::to_string(&PayloadConfig {
        media_url: &media_url,
        media_kind: &media.kind,
        display,
        revision: &payload_revision,
    })
    .map_err(|error| error.to_string())?
    .replace('<', "\\u003c");
    let css =
        serde_json::to_string(generated::BACKGROUND_CSS).map_err(|error| error.to_string())?;
    let review_css =
        serde_json::to_string(generated::REVIEW_SHADOW_CSS).map_err(|error| error.to_string())?;
    let review_style_id =
        serde_json::to_string(REVIEW_SHADOW_STYLE_ID).map_err(|error| error.to_string())?;
    let script = generated::PAYLOAD_TEMPLATE
        .replace("${serialized}", &serialized)
        .replace("${css}", &css)
        .replace("${reviewShadowCss}", &review_css)
        .replace("${reviewShadowStyleId}", &review_style_id);
    Ok(ActivePayload { script, revision })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{MediaKind, MediaOrigin};
    use uuid::Uuid;

    #[test]
    fn builds_payload_from_canonical_typescript_resource() {
        let root = std::env::temp_dir().join(format!("codex-payload-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("background.png");
        fs::write(&path, b"payload bytes").unwrap();
        let item = MediaItem {
            id: Uuid::new_v4().to_string(),
            name: "background.png".to_string(),
            kind: MediaKind::Image,
            origin: MediaOrigin::Local,
            file_name: "background.png".to_string(),
            mime_type: "image/png".to_string(),
            byte_size: 13,
            sha256: "abc".to_string(),
            source_url: None,
            file_count: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            preview_url: None,
        };
        let payload = build_active_payload(&item, &path, &DisplaySettings::default()).unwrap();
        assert!(payload.script.contains("multica-background-layer"));
        assert!(payload.script.contains("diffs-container"));
        assert!(payload.script.contains(r#"[data-sidebar=\"menu-button\"]:hover"#));
        assert!(payload.script.contains("data:image/png;base64,"));
        assert_eq!(payload.revision.len(), 64);
        let _ = fs::remove_dir_all(root);
    }
}
