use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::{io::AsyncReadExt, sync::watch};

const IMPORT_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MAX_WORKER_OUTPUT: usize = 16 * 1024 * 1024;
const PHASES: [&str; 4] = ["preparing", "enumerating", "analysing", "finishing"];

#[derive(Clone)]
pub struct AccessHelper {
    pub script_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub struct WorkerProgress {
    pub phase: String,
    pub completed: u64,
    pub total: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct AccessFailure {
    pub code: &'static str,
    pub message: &'static str,
    pub status: u16,
}

impl AccessFailure {
    fn new(code: &'static str, message: &'static str, status: u16) -> Self {
        Self {
            code,
            message,
            status,
        }
    }
    fn invalid() -> Self {
        Self::new(
            "INVALID_ACCESS_FILE",
            "Accessファイルを読み取れません。形式や保護の状態を確認してください。",
            400,
        )
    }
    fn cancelled() -> Self {
        Self::new(
            "ACCESS_CANCELLED",
            "Accessの解析を中止しました。ファイルを選び直して解析できます。",
            499,
        )
    }
    fn timeout() -> Self {
        Self::new(
            "ACCESS_TIMEOUT",
            "Accessの解析が15分の制限時間内に完了しませんでした。解析結果は作成されていません。",
            504,
        )
    }
    fn unavailable() -> Self {
        Self::new(
            "ACCESS_UNAVAILABLE",
            "このPCでAccessの解析環境を利用できません。JSON解析資料または合成サンプルを使用してください。",
            503,
        )
    }
    fn extraction() -> Self {
        Self::new(
            "ACCESS_EXTRACTION_FAILED",
            "Accessのメタデータを取得できませんでした。保護・暗号化・破損の可能性があります。",
            500,
        )
    }
    fn cleanup() -> Self {
        Self::new(
            "ACCESS_CLEANUP_FAILED",
            "解析用の一時ファイルを削除できませんでした。一時ファイルが残っている可能性があります。",
            500,
        )
    }
}

fn finish_after_cleanup(
    result: Result<Value, AccessFailure>,
    cleanup: Result<(), std::io::Error>,
) -> Result<Value, AccessFailure> {
    match cleanup {
        Ok(()) => result,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => result,
        Err(_) => Err(AccessFailure::cleanup()),
    }
}

impl AccessHelper {
    fn script_path(&self) -> PathBuf {
        let path = self.script_dir.join("export-access.ps1");
        dunce::simplified(&path).to_path_buf()
    }

    pub async fn capabilities(&self) -> Value {
        let script = self.script_path();
        let work =
            std::env::temp_dir().join(format!("access2future-capability-{}", uuid::Uuid::new_v4()));
        if tokio::fs::create_dir(&work).await.is_err() {
            return serde_json::json!({"available": false, "reason": "Accessの解析環境を確認できません。"});
        }
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            hidden_powershell()
                .args(["-File"])
                .arg(script)
                .args(["-Capabilities", "-WorkDirectory"])
                .arg(&work)
                .output(),
        )
        .await;
        let available = match result {
            Ok(Ok(output)) if output.status.success() => {
                serde_json::from_slice::<Value>(&output.stdout)
                    .ok()
                    .and_then(|value| value.get("available").and_then(Value::as_bool))
                    .unwrap_or(false)
            }
            _ => false,
        };
        let _ = tokio::fs::remove_dir_all(work).await;
        if available {
            serde_json::json!({"available": true})
        } else {
            serde_json::json!({"available": false, "reason": "Microsoft Access COM を利用できません。"})
        }
    }

    pub async fn import(
        &self,
        bytes: &[u8],
        file_name: &str,
        progress: watch::Sender<WorkerProgress>,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<Value, AccessFailure> {
        if !supported_name(file_name) || !valid_signature(bytes) {
            return Err(AccessFailure::invalid());
        }
        let suffix = Path::new(file_name)
            .extension()
            .and_then(|part| part.to_str())
            .unwrap_or("accdb")
            .to_ascii_lowercase();
        let work = std::env::temp_dir().join(format!("access2future-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir(&work)
            .await
            .map_err(|_| AccessFailure::extraction())?;
        let staged = work.join(format!("readonly-source.{suffix}"));
        let result = self
            .run_worker(bytes, file_name, &work, &staged, progress, &mut cancel)
            .await;
        let cleanup = tokio::fs::remove_dir_all(&work).await;
        finish_after_cleanup(result, cleanup)
    }

    async fn run_worker(
        &self,
        bytes: &[u8],
        file_name: &str,
        work: &Path,
        staged: &Path,
        progress: watch::Sender<WorkerProgress>,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<Value, AccessFailure> {
        tokio::fs::write(staged, bytes)
            .await
            .map_err(|_| AccessFailure::extraction())?;
        let mut permissions = tokio::fs::metadata(staged)
            .await
            .map_err(|_| AccessFailure::extraction())?
            .permissions();
        permissions.set_readonly(true);
        tokio::fs::set_permissions(staged, permissions)
            .await
            .map_err(|_| AccessFailure::extraction())?;
        if *cancel.borrow() {
            return Err(AccessFailure::cancelled());
        }

        let script = self.script_path();
        let mut child = hidden_powershell()
            .args(["-File"])
            .arg(script)
            .args(["-SourcePath"])
            .arg(staged)
            .args(["-WorkDirectory"])
            .arg(work)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    AccessFailure::unavailable()
                } else {
                    AccessFailure::extraction()
                }
            })?;

        let stdout = child.stdout.take().ok_or_else(AccessFailure::extraction)?;
        let stdout_reader = tokio::spawn(read_limited(stdout, MAX_WORKER_OUTPUT));
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut reader = stderr;
                let mut sink = tokio::io::sink();
                let _ = tokio::io::copy(&mut reader, &mut sink).await;
            });
        }
        let started = Instant::now();
        let mut last: Option<WorkerProgress> = None;
        let mut terminal_error = None;
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|_| AccessFailure::extraction())? {
                break Some(status);
            }
            if *cancel.borrow() || cancel.has_changed().unwrap_or(false) {
                let _ = tokio::fs::write(work.join("cancel-requested"), b"").await;
                wait_for_owner_or_exit(work, &mut child, Duration::from_secs(10)).await;
                let _ = child.kill().await;
                let _ = child.wait().await;
                self.cleanup_owned(work).await;
                terminal_error = Some(AccessFailure::cancelled());
                break None;
            }
            if started.elapsed() >= IMPORT_TIMEOUT {
                let _ = tokio::fs::write(work.join("cancel-requested"), b"").await;
                wait_for_owner_or_exit(work, &mut child, Duration::from_secs(10)).await;
                let _ = child.kill().await;
                let _ = child.wait().await;
                self.cleanup_owned(work).await;
                terminal_error = Some(AccessFailure::timeout());
                break None;
            }
            if let Some(snapshot) = read_progress(&work.join("progress.json"), last.as_ref()).await
            {
                let _ = progress.send(snapshot.clone());
                last = Some(snapshot);
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(150)) => {},
                changed = cancel.changed() => { if changed.is_err() { terminal_error = Some(AccessFailure::cancelled()); } },
            }
        };
        if let Some(error) = terminal_error {
            return Err(error);
        }
        let output = stdout_reader
            .await
            .map_err(|_| AccessFailure::extraction())?
            .map_err(|_| AccessFailure::extraction())?;
        if !status.ok_or_else(AccessFailure::extraction)?.success() {
            return Err(AccessFailure::extraction());
        }
        let text = String::from_utf8_lossy(&output);
        let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}').trim())
            .map_err(|_| AccessFailure::extraction())?;
        if value.get("error").is_some() {
            return Err(worker_failure(&value));
        }
        if value.get("schemaVersion").and_then(Value::as_u64) != Some(1)
            || !value.get("assets").is_some_and(Value::is_array)
            || !value.get("relations").is_some_and(Value::is_array)
            || !value.get("limitations").is_some_and(Value::is_array)
        {
            return Err(AccessFailure::extraction());
        }
        let fingerprint = format!("{:x}", Sha256::digest(bytes));
        let mut result = value;
        let mut source = serde_json::json!({"name": file_name, "kind": "access", "analysedAt": chrono::Utc::now().to_rfc3339(), "fingerprint": fingerprint});
        if let Some(version) = result
            .get("source")
            .and_then(|source| source.get("accessVersion"))
            .filter(|version| !version.is_null())
        {
            source["accessVersion"] = version.clone();
        }
        result["source"] = source;
        let normalized = access2future_domain::normalize_inventory(&result)
            .map_err(|_| AccessFailure::extraction())?;
        let assets = normalized
            .get("assets")
            .and_then(Value::as_array)
            .map_or(0, Vec::len) as u64;
        let _ = progress.send(WorkerProgress {
            phase: "finishing".into(),
            completed: assets,
            total: Some(assets),
        });
        Ok(normalized)
    }

    async fn cleanup_owned(&self, work: &Path) {
        let _ = hidden_powershell()
            .args(["-File"])
            .arg(self.script_path())
            .args(["-CleanupOwned", "-WorkDirectory"])
            .arg(work)
            .output()
            .await;
    }
}

fn worker_failure(value: &Value) -> AccessFailure {
    match value.pointer("/error/code").and_then(Value::as_str) {
        Some("ACCESS_UNAVAILABLE") => AccessFailure::unavailable(),
        Some("ACCESS_CANCELLED") => AccessFailure::cancelled(),
        Some("ACCESS_TIMEOUT") => AccessFailure::timeout(),
        _ => AccessFailure::extraction(),
    }
}

fn hidden_powershell() -> tokio::process::Command {
    let mut command = tokio::process::Command::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.as_std_mut().creation_flags(0x08000000);
    }
    command
}

fn supported_name(name: &str) -> bool {
    let extension = Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    !name.is_empty()
        && name.len() <= 180
        && !name
            .chars()
            .any(|ch| ch == '/' || ch == '\\' || ch.is_control())
        && matches!(extension.to_ascii_lowercase().as_str(), "mdb" | "accdb")
}

fn valid_signature(bytes: &[u8]) -> bool {
    if bytes.len() < 128 {
        return false;
    }
    let signature = String::from_utf8_lossy(&bytes[4..20]);
    signature.starts_with("Standard Jet DB\0") || signature.starts_with("Standard ACE DB\0")
}

async fn read_limited<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> Result<Vec<u8>, std::io::Error> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        if output.len() + read > limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "worker output too large",
            ));
        }
        output.extend_from_slice(&buffer[..read]);
    }
    Ok(output)
}

async fn read_progress(path: &Path, previous: Option<&WorkerProgress>) -> Option<WorkerProgress> {
    let bytes = tokio::fs::read(path).await.ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    let phase = value.get("phase")?.as_str()?;
    let completed = value.get("completed")?.as_u64()?;
    let total = match value.get("total")? {
        Value::Null => None,
        number => Some(number.as_u64()?),
    };
    if !PHASES.contains(&phase)
        || total.is_some_and(|total| completed > total)
        || (total.is_none() && completed != 0)
    {
        return None;
    }
    let current = WorkerProgress {
        phase: phase.to_owned(),
        completed,
        total,
    };
    if let Some(previous) = previous {
        let old_phase = PHASES.iter().position(|phase| *phase == previous.phase)?;
        let new_phase = PHASES.iter().position(|phase| *phase == current.phase)?;
        if current.completed < previous.completed
            || new_phase < old_phase
            || (previous.total.is_some() && current.total != previous.total)
        {
            return None;
        }
    }
    Some(current)
}

async fn wait_for_owner_or_exit(work: &Path, child: &mut tokio::process::Child, wait: Duration) {
    let until = Instant::now() + wait;
    while Instant::now() < until {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        if tokio::fs::try_exists(work.join("access-owner.json"))
            .await
            .unwrap_or(false)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn produces_powershell_compatible_helper_paths() {
        let script_path = |script_dir: &str| {
            AccessHelper {
                script_dir: PathBuf::from(script_dir),
            }
            .script_path()
        };

        assert_eq!(
            script_path(r"\\?\C:\Program Files\Access2Future\scripts"),
            PathBuf::from(r"C:\Program Files\Access2Future\scripts\export-access.ps1")
        );
        assert_eq!(
            script_path(r"\\?\D:\作業\scripts"),
            PathBuf::from(r"D:\作業\scripts\export-access.ps1")
        );
        assert_eq!(
            script_path(r"C:\Program Files\Access2Future\scripts"),
            PathBuf::from(r"C:\Program Files\Access2Future\scripts\export-access.ps1")
        );
        assert_eq!(
            script_path(r"\\?\UNC\server\share\scripts"),
            PathBuf::from(r"\\?\UNC\server\share\scripts\export-access.ps1")
        );
    }

    #[test]
    fn accepts_only_supported_access_names_and_signatures() {
        assert!(supported_name("synthetic-frontend.accdb"));
        assert!(supported_name("legacy.MDB"));
        assert!(!supported_name("../private.accdb"));
        assert!(!supported_name("notes.txt"));
        let mut bytes = vec![0; 128];
        bytes[4..20].copy_from_slice(b"Standard Jet DB\0");
        assert!(valid_signature(&bytes));
        bytes[4..20].copy_from_slice(b"not a database!!");
        assert!(!valid_signature(&bytes));
    }

    #[tokio::test]
    async fn rejects_an_invalid_access_signature_before_launching_powershell() {
        let helper = AccessHelper {
            script_dir: PathBuf::from("missing-helper"),
        };
        let (progress, _) = watch::channel(WorkerProgress {
            phase: "preparing".into(),
            completed: 0,
            total: None,
        });
        let (_, cancel) = watch::channel(false);
        let error = helper
            .import(&vec![0; 128], "sample.accdb", progress, cancel)
            .await
            .unwrap_err();
        assert_eq!(error.code, "INVALID_ACCESS_FILE");
    }

    #[test]
    fn maps_worker_error_codes_without_treating_extraction_as_unavailable() {
        assert_eq!(
            worker_failure(&serde_json::json!({"error":{"code":"ACCESS_UNAVAILABLE"}})).code,
            "ACCESS_UNAVAILABLE"
        );
        assert_eq!(
            worker_failure(&serde_json::json!({"error":{"code":"ACCESS_EXTRACTION_FAILED"}})).code,
            "ACCESS_EXTRACTION_FAILED"
        );
        assert_eq!(
            worker_failure(&serde_json::json!({"error":{"code":"ACCESS_CANCELLED"}})).code,
            "ACCESS_CANCELLED"
        );
        assert_eq!(
            worker_failure(&serde_json::json!({"error":{"code":"unexpected"}})).code,
            "ACCESS_EXTRACTION_FAILED"
        );
    }

    #[test]
    fn reports_failed_cleanup_after_a_cancelled_worker() {
        let result = finish_after_cleanup(
            Err(AccessFailure::cancelled()),
            Err(std::io::Error::other("locked")),
        );
        let error = result.unwrap_err();
        assert_eq!(error.code, "ACCESS_CLEANUP_FAILED");
        assert_eq!(error.status, 500);
    }

    #[tokio::test]
    async fn ignores_invalid_or_regressing_worker_progress() {
        let directory =
            std::env::temp_dir().join(format!("a2f-progress-test-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir(&directory).await.unwrap();
        let path = directory.join("progress.json");
        let first = WorkerProgress {
            phase: "enumerating".into(),
            completed: 3,
            total: Some(10),
        };
        tokio::fs::write(
            &path,
            br#"{"phase":"enumerating","completed":3,"total":10}"#,
        )
        .await
        .unwrap();
        assert_eq!(read_progress(&path, None).await.unwrap().completed, 3);
        tokio::fs::write(&path, br#"{"phase":"preparing","completed":2,"total":10}"#)
            .await
            .unwrap();
        assert!(read_progress(&path, Some(&first)).await.is_none());
        tokio::fs::write(&path, br#"{"phase":"analysing","completed":11,"total":10}"#)
            .await
            .unwrap();
        assert!(read_progress(&path, Some(&first)).await.is_none());
        tokio::fs::remove_dir_all(directory).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires a generated synthetic Access file and desktop Access COM"]
    async fn rust_access_native_smoke() {
        let sample = std::env::var_os("ACCESS2FUTURE_SYNTHETIC_SAMPLE")
            .expect("the native-smoke script supplies a generated synthetic database");
        let path = PathBuf::from(sample);
        let source = tokio::fs::read(&path).await.unwrap();
        let before = Sha256::digest(&source);
        let helper = AccessHelper {
            script_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts"),
        };
        let (progress, receiver) = watch::channel(WorkerProgress {
            phase: "preparing".into(),
            completed: 0,
            total: None,
        });
        let (_cancel_sender, cancel) = watch::channel(false);
        let inventory = helper
            .import(&source, "synthetic.accdb", progress, cancel)
            .await
            .unwrap();
        assert_eq!(inventory["source"]["kind"], "access");
        assert_eq!(inventory["assets"].as_array().unwrap().len(), 6);
        assert_eq!(inventory["relations"].as_array().unwrap().len(), 1);
        assert_eq!(receiver.borrow().phase, "finishing");
        assert_eq!(receiver.borrow().total, Some(6));
        assert_eq!(Sha256::digest(tokio::fs::read(path).await.unwrap()), before);
    }
}
