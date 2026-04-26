//! Python interpreter + `uv` locator and per-bot venv sync.
//!
//! Two modes:
//!
//! - **Bundled**: `<resource_dir>/runtime/python/<triple>/...` and
//!   `<resource_dir>/runtime/uv/<triple>/uv` are shipped inside the Tauri
//!   resource bundle. This is what end users get from a packaged build —
//!   zero Python install required.
//! - **System**: `python3` and `uv` are looked up on `PATH` via the `which`
//!   crate. Used during development (`cargo run` from a checkout) and as a
//!   graceful fallback if the bundled binaries are missing.
//!
//! Per-bot venvs live under `<bot_dir>/.akagi/venv` so they don't clash with
//! a developer's own `.venv` if they happen to keep one in the bot folder.
//! `uv sync` is run on demand and skipped via a stamp file when neither
//! `pyproject.toml` nor `uv.lock` have changed since the last successful
//! sync.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tokio::process::Command;

const STAMP_FILE: &str = "synced.stamp";
const VENV_DIR: &str = "venv";
const AKAGI_DIR: &str = ".akagi";

/// Origin of the python + uv binaries this runtime points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    /// Bundled python-build-standalone + uv from the Tauri resource dir.
    Bundled,
    /// `python3` + `uv` discovered on `PATH`. Dev-mode fallback.
    System,
}

#[derive(Debug, Clone)]
pub struct PythonRuntime {
    /// Interpreter that uv uses to seed venvs (`UV_PYTHON`).
    python: PathBuf,
    /// `uv` binary.
    uv: PathBuf,
    mode: RuntimeMode,
}

impl PythonRuntime {
    /// Direct construction. Tests use this; production code uses `locate`.
    pub fn from_paths(python: PathBuf, uv: PathBuf, mode: RuntimeMode) -> Self {
        Self { python, uv, mode }
    }

    /// Locate bundled binaries first, then fall back to system PATH.
    ///
    /// `resource_dir` is the Tauri-managed resource path (e.g.
    /// `app.path().resource_dir()`); pass `None` outside Tauri.
    pub fn locate(resource_dir: Option<&Path>) -> Result<Self> {
        if let Some(rd) = resource_dir {
            if let Some(rt) = try_bundled(rd) {
                return Ok(rt);
            }
        }
        try_system().context(
            "no bundled runtime found and neither `python3` nor `uv` is on PATH",
        )
    }

    pub fn python(&self) -> &Path {
        &self.python
    }

    pub fn uv(&self) -> &Path {
        &self.uv
    }

    pub fn mode(&self) -> RuntimeMode {
        self.mode
    }

    /// Run `uv sync` against the bot's `pyproject.toml` if the on-disk
    /// signature has changed since the last successful sync. Idempotent.
    pub async fn ensure_synced(&self, bot_dir: &Path) -> Result<()> {
        let pyproject = bot_dir.join("pyproject.toml");
        if !pyproject.is_file() {
            bail!(
                "pyproject.toml missing in {} — every Akagi bot must declare its deps",
                bot_dir.display()
            );
        }
        let lock = bot_dir.join("uv.lock");
        let venv = bot_dir.join(AKAGI_DIR).join(VENV_DIR);
        let stamp_path = bot_dir.join(AKAGI_DIR).join(STAMP_FILE);

        let current = current_signature(&pyproject, &lock)?;
        if venv.is_dir() && stamp_matches(&stamp_path, &current).await? {
            return Ok(());
        }

        if let Some(parent) = stamp_path.parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!("create {}", parent.display())
            })?;
        }

        let status = Command::new(&self.uv)
            .arg("sync")
            .arg("--project")
            .arg(bot_dir)
            .env("UV_PYTHON", &self.python)
            .env("UV_PROJECT_ENVIRONMENT", &venv)
            .status()
            .await
            .with_context(|| format!("spawn `uv` at {}", self.uv.display()))?;
        if !status.success() {
            bail!("uv sync failed in {} ({status})", bot_dir.display());
        }

        write_stamp(&stamp_path, &current).await?;
        Ok(())
    }

    /// Build a `tokio::process::Command` that runs the venv's python with
    /// the given args, with `current_dir` set to the bot directory so
    /// `bot.py` can resolve relative paths.
    pub fn command_for(&self, bot_dir: &Path, args: &[&str]) -> Command {
        let py = venv_python(&bot_dir.join(AKAGI_DIR).join(VENV_DIR));
        let mut cmd = Command::new(py);
        cmd.current_dir(bot_dir).args(args);
        cmd
    }
}

fn try_bundled(resource_dir: &Path) -> Option<PythonRuntime> {
    let triple = host_triple();
    let py = resource_dir
        .join("runtime")
        .join("python")
        .join(triple)
        .join(if cfg!(windows) { "python.exe" } else { "bin/python3" });
    let uv = resource_dir
        .join("runtime")
        .join("uv")
        .join(triple)
        .join(if cfg!(windows) { "uv.exe" } else { "uv" });
    if py.is_file() && uv.is_file() {
        Some(PythonRuntime::from_paths(py, uv, RuntimeMode::Bundled))
    } else {
        None
    }
}

fn try_system() -> Result<PythonRuntime> {
    let python = which::which("python3")
        .or_else(|_| which::which("python"))
        .context("locate python3/python on PATH")?;
    let uv = which::which("uv").context("locate uv on PATH")?;
    Ok(PythonRuntime::from_paths(python, uv, RuntimeMode::System))
}

fn venv_python(venv: &Path) -> PathBuf {
    if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    }
}

/// Build target triple — used to pick the right bundled runtime.
fn host_triple() -> &'static str {
    // `cargo` doesn't expose the runtime target triple, only the build-time
    // one — which is exactly what we want here, since the bundled binary
    // matches what we compiled for.
    env!("TARGET_TRIPLE", "TARGET_TRIPLE not set; build.rs should pass it")
}

/// `mtime:size` for `pyproject.toml` plus the same for `uv.lock` if it
/// exists. Cheap to compute (no file read) and stable across reboots
/// (mtime is filesystem-persistent). Granularity is 1 s, which is fine —
/// `uv sync` writes its lockfile with the current second.
fn current_signature(pyproject: &Path, lock: &Path) -> Result<String> {
    let proj = file_meta(pyproject)?;
    let lock_part = if lock.exists() {
        let l = file_meta(lock)?;
        format!("{}:{}", l.0, l.1)
    } else {
        "0:0".into()
    };
    Ok(format!("v1|{}:{}|{}", proj.0, proj.1, lock_part))
}

fn file_meta(p: &Path) -> Result<(u64, u64)> {
    let m = std::fs::metadata(p).with_context(|| format!("stat {}", p.display()))?;
    let mtime = m
        .modified()
        .with_context(|| format!("mtime {}", p.display()))?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    Ok((mtime, m.len()))
}

async fn stamp_matches(stamp_path: &Path, current: &str) -> Result<bool> {
    match tokio::fs::read_to_string(stamp_path).await {
        Ok(saved) => Ok(saved.trim() == current),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("read {}", stamp_path.display())),
    }
}

async fn write_stamp(stamp_path: &Path, sig: &str) -> Result<()> {
    tokio::fs::write(stamp_path, sig)
        .await
        .with_context(|| format!("write {}", stamp_path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(p: &Path, body: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn dummy_runtime() -> PythonRuntime {
        PythonRuntime::from_paths(
            PathBuf::from("/dev/null/python"),
            PathBuf::from("/dev/null/uv"),
            RuntimeMode::System,
        )
    }

    #[tokio::test]
    async fn ensure_synced_bails_when_pyproject_missing() {
        let tmp = TempDir::new().unwrap();
        let rt = dummy_runtime();
        let err = rt.ensure_synced(tmp.path()).await.unwrap_err();
        assert!(
            err.to_string().contains("pyproject.toml missing"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn signature_changes_when_pyproject_changes() {
        let tmp = TempDir::new().unwrap();
        let py = tmp.path().join("pyproject.toml");
        let lock = tmp.path().join("uv.lock");
        write(&py, "[project]\nname='a'\n");
        let s1 = current_signature(&py, &lock).unwrap();

        // Bump mtime by sleeping 1.1s + writing different content. mtime
        // granularity on most filesystems is 1s, so we need to clear at
        // least one whole second.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(&py, "[project]\nname='b'\n");
        let s2 = current_signature(&py, &lock).unwrap();
        assert_ne!(s1, s2, "signature must change after pyproject edit");
    }

    #[test]
    fn signature_includes_lock_when_present() {
        let tmp = TempDir::new().unwrap();
        let py = tmp.path().join("pyproject.toml");
        let lock = tmp.path().join("uv.lock");
        write(&py, "[project]\nname='a'\n");

        let no_lock = current_signature(&py, &lock).unwrap();
        write(&lock, "version = 1\n");
        let with_lock = current_signature(&py, &lock).unwrap();
        assert_ne!(no_lock, with_lock);
    }

    #[tokio::test]
    async fn stamp_round_trip() {
        let tmp = TempDir::new().unwrap();
        let stamp = tmp.path().join(AKAGI_DIR).join(STAMP_FILE);
        std::fs::create_dir_all(stamp.parent().unwrap()).unwrap();

        assert!(!stamp_matches(&stamp, "v1|abc").await.unwrap());
        write_stamp(&stamp, "v1|abc").await.unwrap();
        assert!(stamp_matches(&stamp, "v1|abc").await.unwrap());
        assert!(!stamp_matches(&stamp, "v1|xyz").await.unwrap());
    }

    #[test]
    fn venv_python_path_per_platform() {
        let venv = Path::new("/foo/.akagi/venv");
        let p = venv_python(venv);
        if cfg!(windows) {
            assert!(p.ends_with("Scripts/python.exe") || p.ends_with("Scripts\\python.exe"));
        } else {
            assert!(p.ends_with("bin/python"));
        }
    }

    #[test]
    fn try_bundled_returns_none_when_runtime_dir_empty() {
        let tmp = TempDir::new().unwrap();
        assert!(try_bundled(tmp.path()).is_none());
    }
}
