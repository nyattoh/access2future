use std::{fs::File, io::Write, path::Path};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

pub type SaveError = std::io::Error;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Markdown,
    Json,
}

impl ExportFormat {
    fn file_name(self) -> &'static str {
        match self {
            Self::Markdown => "access-migration-plan.md",
            Self::Json => "access-migration-plan.json",
        }
    }

    fn filter(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Markdown => ("Markdown", &["md"]),
            Self::Json => ("JSON", &["json"]),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SaveRequest {
    pub format: ExportFormat,
    pub content: String,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct SaveResponse {
    pub path: String,
    pub bytes: u64,
}

/// Shows the native Save As dialog and writes the file. `Ok(None)` means the user cancelled.
pub async fn save_via_native_dialog(
    app: &AppHandle,
    request: SaveRequest,
) -> Result<Option<SaveResponse>, SaveError> {
    let (name, extensions) = request.format.filter();
    let mut dialog = app
        .dialog()
        .file()
        .set_file_name(request.format.file_name())
        .add_filter(name, extensions);
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    dialog.save_file(move |chosen| {
        let _ = sender.send(chosen);
    });
    let Some(chosen) = receiver
        .await
        .map_err(|_| SaveError::other("save dialog did not return"))?
    else {
        return Ok(None);
    };
    let path = chosen
        .into_path()
        .map_err(|_| SaveError::other("save dialog returned a non-file path"))?;
    tokio::task::spawn_blocking(move || write_export_file(&path, &request.content))
        .await
        .map_err(|_| SaveError::other("save task stopped"))?
        .map(Some)
}

/// Reports success only after the bytes are flushed and the file size is verified.
pub fn write_export_file(path: &Path, content: &str) -> Result<SaveResponse, SaveError> {
    let mut file = File::create(path)?;
    file.write_all(content.as_bytes())?;
    file.sync_all()?;
    let bytes = file.metadata()?.len();
    if bytes != content.len() as u64 {
        return Err(SaveError::other("written size differs from content size"));
    }
    Ok(SaveResponse {
        path: path.display().to_string(),
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("a2f-save-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_exact_utf8_bytes_and_reports_the_path_and_length() {
        let dir = temp_dir("ok");
        let path = dir.join("plan.md");
        let content = "\u{FEFF}# 移行計画\n未承認\n";

        let saved = write_export_file(&path, content).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), content.as_bytes());
        assert_eq!(saved.path, path.display().to_string());
        assert_eq!(saved.bytes, content.len() as u64);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn overwrites_an_existing_longer_file_without_leftover_bytes() {
        let dir = temp_dir("overwrite");
        let path = dir.join("plan.json");
        std::fs::write(&path, "x".repeat(100)).unwrap();

        write_export_file(&path, "{}").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn write_errors_do_not_report_success() {
        let dir = temp_dir("err");
        let missing_parent = dir.join("no-such-folder").join("plan.md");

        assert!(write_export_file(&missing_parent, "x").is_err());
        assert!(
            write_export_file(&dir, "x").is_err(),
            "a directory is not a file"
        );
        assert!(!missing_parent.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn request_accepts_only_the_closed_format_set() {
        let ok: SaveRequest =
            serde_json::from_str(r#"{"format":"markdown","content":"a"}"#).unwrap();
        assert!(matches!(ok.format, ExportFormat::Markdown));
        assert!(serde_json::from_str::<SaveRequest>(r#"{"format":"exe","content":"a"}"#).is_err());
        assert!(
            serde_json::from_str::<SaveRequest>(r#"{"format":"json","content":"a","path":"C:/x"}"#)
                .is_ok(),
            "unknown fields are ignored, never used as a path"
        );
    }
}
