//! Composer attachments: what the server accepts and the byte upload that follows
//! `attachments.createUploadUrl` (`packages/contracts/src/{chatAttachment,assets}.ts`).
use anyhow::{Context, Result, bail};

/// `PROVIDER_SEND_TURN_SUPPORTED_IMAGE_MIME_TYPES`; other images go up as plain files.
pub const IMAGE_TYPES: &[&str] = &["image/gif", "image/jpeg", "image/png", "image/webp"];
/// `PROVIDER_SEND_TURN_MAX_IMAGE_BYTES`.
pub const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;

/// An image type the providers take, by the file's own bytes.
pub fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// A plain file's type by its extension, for the agent's benefit; the server stores any.
pub fn file_type(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map_or("", |(_, e)| e)
        .to_ascii_lowercase();
    match ext.as_str() {
        "txt" | "log" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "pdf" => "application/pdf",
        "svg" => "image/svg+xml",
        "zip" => "application/zip",
        "rs" | "ts" | "tsx" | "js" | "py" | "go" | "sh" | "toml" | "yaml" | "yml" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// POSTs the bytes to the upload URL the server minted; the token in its path is the only
/// credential. Blocking; call it off the UI thread.
pub fn upload_blocking(http_base: &str, relative_url: &str, bytes: Vec<u8>) -> Result<()> {
    let url = url::Url::parse(http_base)?.join(relative_url.trim_start_matches('/'))?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let res = crate::pairing::http()
            .post(url)
            .header("content-type", "application/octet-stream")
            .body(bytes)
            .send()
            .await
            .context("Couldn't reach the server")?;
        if !res.status().is_success() {
            bail!(
                "Upload failed ({}): {}",
                res.status(),
                res.text().await.unwrap_or_default()
            );
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_are_known_by_their_bytes() {
        assert_eq!(image_type(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(image_type(b"GIF89a.."), Some("image/gif"));
        assert_eq!(image_type(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(image_type(b"%PDF-1.7"), None);
    }
}
