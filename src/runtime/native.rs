use super::traits::RuntimeAdapter;
use std::path::{Path, PathBuf};

/// Native runtime — full access, runs on Mac/Linux/Docker/Raspberry Pi
pub struct NativeRuntime;

impl NativeRuntime {
    pub fn new() -> Self {
        Self
    }
}

impl RuntimeAdapter for NativeRuntime {
    fn name(&self) -> &str {
        "native"
    }

    fn has_shell_access(&self) -> bool {
        true
    }

    fn has_filesystem_access(&self) -> bool {
        true
    }

    fn storage_path(&self) -> PathBuf {
        directories::UserDirs::new().map_or_else(
            || PathBuf::from(".zeroclaw"),
            |u| u.home_dir().join(".zeroclaw"),
        )
    }

    fn supports_long_running(&self) -> bool {
        true
    }

    #[cfg(target_os = "windows")]
    fn build_shell_command(
        &self,
        command: &str,
        workspace_dir: &Path,
    ) -> anyhow::Result<tokio::process::Command> {
        let shell = std::env::var("ComSpec")
            .or_else(|_| std::env::var("COMSPEC"))
            .unwrap_or_else(|_| "cmd.exe".to_string());

        let mut process = tokio::process::Command::new(shell);
        process.arg("/C").arg(command).current_dir(workspace_dir);
        Ok(process)
    }

    #[cfg(not(target_os = "windows"))]
    fn build_shell_command(
        &self,
        command: &str,
        workspace_dir: &Path,
    ) -> anyhow::Result<tokio::process::Command> {
        let mut process = tokio::process::Command::new("sh");
        process.arg("-c").arg(command).current_dir(workspace_dir);
        Ok(process)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_name() {
        assert_eq!(NativeRuntime::new().name(), "native");
    }

    #[test]
    fn native_has_shell_access() {
        assert!(NativeRuntime::new().has_shell_access());
    }

    #[test]
    fn native_has_filesystem_access() {
        assert!(NativeRuntime::new().has_filesystem_access());
    }

    #[test]
    fn native_supports_long_running() {
        assert!(NativeRuntime::new().supports_long_running());
    }

    #[test]
    fn native_memory_budget_unlimited() {
        assert_eq!(NativeRuntime::new().memory_budget(), 0);
    }

    #[test]
    fn native_storage_path_contains_zeroclaw() {
        let path = NativeRuntime::new().storage_path();
        assert!(path.to_string_lossy().contains("zeroclaw"));
    }

    #[test]
    fn native_builds_shell_command() {
        let cwd = std::env::temp_dir();
        let command = NativeRuntime::new()
            .build_shell_command("echo hello", &cwd)
            .unwrap();
        let debug = format!("{command:?}");
        assert!(debug.contains("echo hello"));
    }

    #[test]
    fn native_builds_platform_shell_wrapper() {
        let cwd = std::env::temp_dir();
        let command = NativeRuntime::new()
            .build_shell_command("echo hello", &cwd)
            .unwrap();
        let debug = format!("{command:?}").to_lowercase();
        #[cfg(target_os = "windows")]
        assert!(debug.contains("/c") && debug.contains("cmd"));
        #[cfg(not(target_os = "windows"))]
        assert!(debug.contains("sh") && debug.contains("-c"));
    }
}
