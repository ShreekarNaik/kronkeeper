use std::path::Path;

use reqwest::Method;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use crate::models::JobPayload;

const MAX_CAPTURE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub success: bool,
    pub output: String,
    pub status_code: Option<u16>,
}

pub struct Executor {
    http_client: reqwest::Client,
    script_safe_dir: std::path::PathBuf,
}

impl Executor {
    pub fn new(script_safe_dir: std::path::PathBuf) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            script_safe_dir,
        }
    }

    pub async fn execute(&self, payload: &JobPayload) -> ExecutionResult {
        let timeout_dur = Duration::from_secs(payload.timeout_sec());
        match timeout(timeout_dur, self.execute_inner(payload)).await {
            Ok(result) => result,
            Err(_) => ExecutionResult {
                success: false,
                output: format!("execution timed out after {}s", payload.timeout_sec()),
                status_code: None,
            },
        }
    }

    async fn execute_inner(&self, payload: &JobPayload) -> ExecutionResult {
        match payload {
            JobPayload::Http {
                method,
                url,
                headers,
                body,
                ..
            } => self.execute_http(method, url, headers, body.as_deref()).await,
            JobPayload::Script { path, args, .. } => self.execute_script(path, args).await,
        }
    }

    async fn execute_http(
        &self,
        method: &str,
        url: &str,
        headers: &std::collections::HashMap<String, String>,
        body: Option<&str>,
    ) -> ExecutionResult {
        let method = match method.to_uppercase().parse::<Method>() {
            Ok(m) => m,
            Err(e) => {
                return ExecutionResult {
                    success: false,
                    output: format!("invalid HTTP method: {e}"),
                    status_code: None,
                };
            }
        };

        let mut req = self.http_client.request(method, url);
        for (k, v) in headers {
            req = req.header(k, v);
        }
        if let Some(body) = body {
            req = req.body(body.to_string());
        }

        match req.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let text = resp.text().await.unwrap_or_default();
                let truncated = truncate(&text);
                ExecutionResult {
                    success: (200..300).contains(&status),
                    output: truncated,
                    status_code: Some(status),
                }
            }
            Err(e) => ExecutionResult {
                success: false,
                output: e.to_string(),
                status_code: None,
            },
        }
    }

    async fn execute_script(&self, path: &str, args: &[String]) -> ExecutionResult {
        match validate_script_path(&self.script_safe_dir, path) {
            Ok(valid_path) => {
                let mut cmd = Command::new(&valid_path);
                cmd.args(args);
                match cmd.output().await {
                    Ok(output) => {
                        let stdout = truncate(&String::from_utf8_lossy(&output.stdout));
                        let stderr = truncate(&String::from_utf8_lossy(&output.stderr));
                        let combined = if stderr.is_empty() {
                            stdout
                        } else {
                            format!("{stdout}\nstderr: {stderr}")
                        };
                        ExecutionResult {
                            success: output.status.success(),
                            output: combined,
                            status_code: output.status.code().map(|c| c as u16),
                        }
                    }
                    Err(e) => ExecutionResult {
                        success: false,
                        output: e.to_string(),
                        status_code: None,
                    },
                }
            }
            Err(e) => ExecutionResult {
                success: false,
                output: e,
                status_code: None,
            },
        }
    }
}

pub fn validate_script_path(safe_dir: &Path, path: &str) -> Result<std::path::PathBuf, String> {
    let candidate = safe_dir.join(path);
    let canonical_safe = safe_dir
        .canonicalize()
        .map_err(|e| format!("script safe dir unavailable: {e}"))?;
    let canonical_script = candidate
        .canonicalize()
        .map_err(|e| format!("script path invalid: {e}"))?;

    if !canonical_script.starts_with(&canonical_safe) {
        return Err("script path escapes safe directory".to_string());
    }

    Ok(canonical_script)
}

fn truncate(s: &str) -> String {
    if s.len() <= MAX_CAPTURE_BYTES {
        s.to_string()
    } else {
        format!("{}...(truncated)", &s[..MAX_CAPTURE_BYTES])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn rejects_path_outside_safe_dir() {
        let dir = tempdir().unwrap();
        let safe = dir.path().join("safe");
        fs::create_dir_all(&safe).unwrap();
        let outside = dir.path().join("outside.sh");
        fs::write(&outside, "#!/bin/sh").unwrap();

        let err = validate_script_path(&safe, "../outside.sh").unwrap_err();
        assert!(err.contains("escapes") || err.contains("invalid"));
    }

    #[test]
    fn accepts_path_inside_safe_dir() {
        let dir = tempdir().unwrap();
        let safe = dir.path().join("safe");
        fs::create_dir_all(&safe).unwrap();
        let script = safe.join("run.sh");
        fs::write(&script, "#!/bin/sh").unwrap();

        let result = validate_script_path(&safe, "run.sh");
        assert!(result.is_ok());
    }
}
