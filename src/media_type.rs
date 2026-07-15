use anyhow::{Context, Result};
use std::path::Path;

const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";

pub fn detect(path_hint: &Path, content: &Path) -> Result<String> {
    let by_extension = mime_guess::from_path(path_hint)
        .first_raw()
        .map(str::to_string);
    let by_content = infer::get_from_path(content)
        .with_context(|| format!("failed to inspect content type of {}", content.display()))?
        .map(|kind| kind.mime_type().to_string());

    Ok(match (by_content, by_extension) {
        (Some(detected), Some(extension)) if is_generic_container(&detected) => extension,
        (Some(detected), _) => detected,
        (None, Some(extension)) => extension,
        (None, None) => DEFAULT_CONTENT_TYPE.to_string(),
    })
}

fn is_generic_container(content_type: &str) -> bool {
    matches!(
        content_type,
        "application/octet-stream"
            | "application/zip"
            | "application/xml"
            | "text/plain"
            | "text/xml"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn content_signature_overrides_wrong_extension() {
        let temp = TempDir::new().unwrap();
        let content = temp.path().join("cached");
        fs::write(
            &content,
            [
                0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H',
                b'D', b'R',
            ],
        )
        .unwrap();
        assert_eq!(
            detect(Path::new("image.jpg"), &content).unwrap(),
            "image/png"
        );
    }

    #[test]
    fn extension_handles_semantic_text_types() {
        let temp = TempDir::new().unwrap();
        let content = temp.path().join("cached");
        fs::write(&content, b"body { color: red; }").unwrap();
        assert_eq!(detect(Path::new("site.css"), &content).unwrap(), "text/css");
    }

    #[test]
    fn unknown_content_uses_binary_default() {
        let temp = TempDir::new().unwrap();
        let content = temp.path().join("cached");
        fs::write(&content, b"custom data").unwrap();
        assert_eq!(
            detect(Path::new("README"), &content).unwrap(),
            DEFAULT_CONTENT_TYPE
        );
    }
}
