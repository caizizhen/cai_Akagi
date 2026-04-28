//! Install a Mjai bot from a GitHub release.
//!
//! Flow:
//!
//! 1. Fetch `https://api.github.com/repos/<repo>/releases/latest` (anonymous).
//! 2. Pick one asset — by glob if `asset_glob` is set, else the first `.zip`.
//! 3. Stream the asset into a tempfile under `<dest_root>/.downloads/`.
//! 4. Open the tempfile as a zip, validate every entry's path is enclosed
//!    inside the destination (no `..`, no absolute paths).
//! 5. Extract to a sibling tempdir.
//! 6. If the archive has a single top-level directory, strip it.
//! 7. Validate `bot.py` is present in the extracted layout.
//! 8. Atomic-move the tempdir into `<dest_root>/<name>/`.
//!
//! Existing `<dest_root>/<name>/` is treated as an error — frontend can
//! offer an explicit "remove and reinstall" toggle later. Authenticated
//! installs (private repos / token), tarballs, and source-tree clones are
//! out of scope for v1.

use crate::bot::manifest::Manifest;
use crate::bot::registry::BotEntry;
use crate::event_bus::NotifyBus;
use crate::schema::Notification;
use anyhow::{Context, Result, bail};
use globset::Glob;
use serde::Deserialize;
use std::path::{Component, Path, PathBuf};
use tokio::io::AsyncWriteExt;

const GITHUB_API: &str = "https://api.github.com";
const USER_AGENT: &str = concat!("akagi/", env!("CARGO_PKG_VERSION"));
const DOWNLOADS_DIR: &str = ".downloads";

/// Pointer to a GitHub release-zip install.
#[derive(Debug, Clone)]
pub struct GithubInstallSpec {
    /// `owner/name`.
    pub repo: String,
    /// Glob to pick one asset out of the release. `None` → first `.zip`.
    pub asset_glob: Option<String>,
    /// Override target subdir name. `None` → second segment of `repo`.
    pub name: Option<String>,
}

impl GithubInstallSpec {
    /// Resolve the target subdir name. Repo must already be validated.
    fn target_name(&self) -> String {
        if let Some(n) = self.name.as_deref() {
            return n.to_owned();
        }
        // `repo` is already validated to be `owner/name`; second segment
        // is the default install name.
        self.repo
            .split('/')
            .nth(1)
            .unwrap_or(&self.repo)
            .to_owned()
    }
}

#[derive(Debug, Deserialize)]
struct ReleaseJson {
    #[serde(default)]
    tag_name: Option<String>,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

/// End-to-end install. Returns the registry entry for the freshly
/// extracted bot.
pub async fn install_from_github_release(
    spec: GithubInstallSpec,
    dest_root: &Path,
    notify: &NotifyBus,
) -> Result<BotEntry> {
    validate_repo(&spec.repo)?;
    let target_name = spec.target_name();
    validate_target_name(&target_name)?;
    let dest_dir = dest_root.join(&target_name);
    if dest_dir.exists() {
        bail!(
            "{} already exists — remove it before reinstalling",
            dest_dir.display()
        );
    }

    let notify_id = format!("bot-install-{target_name}");
    let _ = notify.send(
        Notification::info(format!("Installing {target_name}"))
            .body(format!("Fetching release metadata for {}", spec.repo))
            .sticky()
            .id(notify_id.clone()),
    );

    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .context("build http client")?;

    let release: ReleaseJson = client
        .get(format!(
            "{GITHUB_API}/repos/{}/releases/latest",
            spec.repo
        ))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("fetch release metadata")?
        .error_for_status()
        .context("github release endpoint returned an error")?
        .json()
        .await
        .context("parse release JSON")?;

    let asset = pick_asset(&release.assets, spec.asset_glob.as_deref())?;

    let _ = notify.send(
        Notification::info(format!("Installing {target_name}"))
            .body(format!(
                "Downloading {} ({})",
                asset.name,
                release.tag_name.as_deref().unwrap_or("latest"),
            ))
            .sticky()
            .id(notify_id.clone()),
    );

    let downloads_dir = dest_root.join(DOWNLOADS_DIR);
    tokio::fs::create_dir_all(&downloads_dir)
        .await
        .with_context(|| format!("mkdir {}", downloads_dir.display()))?;

    let tempfile_path = download_asset(&client, &asset.browser_download_url, &downloads_dir)
        .await
        .with_context(|| format!("download {}", asset.browser_download_url))?;

    let _ = notify.send(
        Notification::info(format!("Installing {target_name}"))
            .body("Extracting…")
            .sticky()
            .id(notify_id.clone()),
    );

    // Extract into a sibling dir of the eventual destination so the
    // final rename stays on the same filesystem. We use a manually-named
    // dir (not TempDir) because we may rename the dir itself away — Drop
    // semantics on a renamed TempDir are awkward.
    let staging = downloads_dir.join(format!(
        "akagi-extract-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let install_result = (|| -> Result<()> {
        std::fs::create_dir(&staging)
            .with_context(|| format!("mkdir {}", staging.display()))?;
        extract_zip_safe(&tempfile_path, &staging)
            .with_context(|| format!("extract {}", tempfile_path.display()))?;

        let resolved_root = strip_single_top_level(&staging)?;
        validate_layout(&resolved_root)?;

        std::fs::rename(&resolved_root, &dest_dir).with_context(|| {
            format!(
                "rename {} -> {}",
                resolved_root.display(),
                dest_dir.display()
            )
        })?;
        Ok(())
    })();

    // Best-effort cleanup of any leftover staging contents. If the
    // staging dir was renamed away wholesale, this is a no-op error.
    let _ = std::fs::remove_dir_all(&staging);
    let _ = tokio::fs::remove_file(&tempfile_path).await;

    install_result?;

    let _ = notify.send(
        Notification::success(format!("{target_name} installed"))
            .body(format!(
                "{} ({})",
                spec.repo,
                release.tag_name.as_deref().unwrap_or("latest"),
            ))
            .id(notify_id),
    );

    let pyproject = dest_dir.join("pyproject.toml");
    Ok(BotEntry {
        name: target_name.clone(),
        dir: dest_dir.clone(),
        pyproject: pyproject.is_file().then_some(pyproject),
        manifest: Manifest::load(&dest_dir).ok().flatten(),
    })
}

fn validate_repo(repo: &str) -> Result<()> {
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        bail!("repo {repo:?} is not in the form `owner/name`");
    }
    for part in parts {
        if !part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        {
            bail!(
                "repo {repo:?} has illegal characters; expected [A-Za-z0-9._-]+/[A-Za-z0-9._-]+"
            );
        }
    }
    Ok(())
}

fn validate_target_name(name: &str) -> Result<()> {
    if name.is_empty() || name.starts_with('.') {
        bail!("target name {name:?} is invalid");
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        bail!("target name {name:?} contains path separators");
    }
    if name == "base" {
        bail!("target name {name:?} is reserved");
    }
    Ok(())
}

/// Choose one asset out of the release.
///
/// Public for unit tests so we can verify the glob and fallback rules
/// without spinning up a network mock.
pub fn pick_asset<'a>(assets: &'a [Asset], glob: Option<&str>) -> Result<&'a Asset> {
    if assets.is_empty() {
        bail!("release has no assets");
    }

    if let Some(pat) = glob {
        let matcher = Glob::new(pat)
            .with_context(|| format!("compile glob {pat:?}"))?
            .compile_matcher();
        let matches: Vec<&Asset> = assets.iter().filter(|a| matcher.is_match(&a.name)).collect();
        match matches.len() {
            0 => bail!("no asset in release matches glob {pat:?}"),
            1 => Ok(matches[0]),
            n => bail!(
                "{n} assets matched glob {pat:?}; tighten the pattern (matched: {:?})",
                matches.iter().map(|a| &a.name).collect::<Vec<_>>()
            ),
        }
    } else {
        assets
            .iter()
            .find(|a| a.name.to_ascii_lowercase().ends_with(".zip"))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no .zip asset in release; pass asset_glob to pick a non-zip explicitly"
                )
            })
    }
}

async fn download_asset(
    client: &reqwest::Client,
    url: &str,
    downloads_dir: &Path,
) -> Result<PathBuf> {
    let path = downloads_dir.join(format!(
        "akagi-bot-{}.zip",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    let mut response = client
        .get(url)
        .send()
        .await
        .context("send download request")?
        .error_for_status()
        .context("download endpoint returned error")?;

    let mut file = tokio::fs::File::create(&path)
        .await
        .with_context(|| format!("create {}", path.display()))?;
    while let Some(chunk) = response
        .chunk()
        .await
        .context("read body chunk")?
    {
        file.write_all(&chunk)
            .await
            .with_context(|| format!("write {}", path.display()))?;
    }
    file.flush().await.ok();
    Ok(path)
}

/// Extract `zip_path` into `dest_dir`. Rejects entries whose normalised
/// path escapes the destination root (`..`, absolute paths) — standard
/// "zip slip" defence.
pub fn extract_zip_safe(zip_path: &Path, dest_dir: &Path) -> Result<()> {
    let file = std::fs::File::open(zip_path)
        .with_context(|| format!("open {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("not a valid zip: {}", zip_path.display()))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .with_context(|| format!("read zip entry {i}"))?;
        let raw_name = entry.name().to_owned();

        // zip 8 exposes `enclosed_name()` which already validates that the
        // path is relative and contains no `..` segments. We additionally
        // double-check by walking components, because the `enclosed_name`
        // check is only as strong as the underlying `Path` parser.
        let Some(rel) = entry.enclosed_name() else {
            bail!("zip entry {raw_name:?} has an unsafe path");
        };
        if rel.components().any(|c| matches!(c, Component::ParentDir)) {
            bail!("zip entry {raw_name:?} contains `..`");
        }
        if rel.is_absolute() {
            bail!("zip entry {raw_name:?} is absolute");
        }

        let out_path = dest_dir.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)
                .with_context(|| format!("mkdir {}", out_path.display()))?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("mkdir {}", parent.display()))?;
        }
        let mut out = std::fs::File::create(&out_path)
            .with_context(|| format!("create {}", out_path.display()))?;
        std::io::copy(&mut entry, &mut out)
            .with_context(|| format!("write {}", out_path.display()))?;

        // Preserve unix mode bits (executable scripts).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = entry.unix_mode() {
                let _ = std::fs::set_permissions(
                    &out_path,
                    std::fs::Permissions::from_mode(mode),
                );
            }
        }
    }
    Ok(())
}

/// If `dir` contains exactly one entry and that entry is a directory,
/// return that nested directory. Otherwise return `dir` unchanged.
///
/// Real-world release zips usually wrap everything in a single top-level
/// dir like `mortal-v0.5.0/…`. Strip it so the bot's `bot.py` ends up
/// directly under `<bot>/bot.py` rather than `<bot>/mortal-v0.5.0/bot.py`.
pub fn strip_single_top_level(dir: &Path) -> Result<PathBuf> {
    let mut entries = std::fs::read_dir(dir)
        .with_context(|| format!("read_dir {}", dir.display()))?
        .filter_map(|e| e.ok())
        .collect::<Vec<_>>();
    if entries.len() != 1 {
        return Ok(dir.to_path_buf());
    }
    let only = entries.remove(0);
    if only.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
        Ok(only.path())
    } else {
        Ok(dir.to_path_buf())
    }
}

/// Reject installs that don't look like bots — `bot.py` is the registry
/// contract. Pyproject is recommended but not enforced (some bots may
/// run on system python without uv).
pub fn validate_layout(bot_root: &Path) -> Result<()> {
    let bot_py = bot_root.join("bot.py");
    if !bot_py.is_file() {
        bail!(
            "extracted archive does not contain bot.py at the top level — refusing install"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.to_owned(),
            browser_download_url: format!("https://example.com/{name}"),
        }
    }

    #[test]
    fn validate_repo_accepts_owner_slash_name() {
        validate_repo("Equim-chan/Mortal").unwrap();
        validate_repo("a.b/c-d_e.f").unwrap();
    }

    #[test]
    fn validate_repo_rejects_bad_input() {
        for bad in [
            "",
            "noslash",
            "/leading",
            "trailing/",
            "a/b/c",
            "owner with space/name",
            "owner/name?query",
        ] {
            let err = validate_repo(bad).unwrap_err();
            assert!(
                err.to_string().contains("repo"),
                "expected error for {bad:?}: {err:#}"
            );
        }
    }

    #[test]
    fn validate_target_name_rejects_separators_and_reserved() {
        validate_target_name("mortal").unwrap();
        for bad in ["", ".hidden", "a/b", "a\\b", "..", "with..parent", "base"] {
            assert!(
                validate_target_name(bad).is_err(),
                "{bad:?} should be invalid"
            );
        }
    }

    #[test]
    fn target_name_defaults_to_repo_second_segment() {
        let s = GithubInstallSpec {
            repo: "owner/name-with-dashes".into(),
            asset_glob: None,
            name: None,
        };
        assert_eq!(s.target_name(), "name-with-dashes");
    }

    #[test]
    fn target_name_override_takes_precedence() {
        let s = GithubInstallSpec {
            repo: "owner/upstream".into(),
            asset_glob: None,
            name: Some("local-alias".into()),
        };
        assert_eq!(s.target_name(), "local-alias");
    }

    #[test]
    fn pick_asset_first_zip_when_no_glob() {
        let assets = vec![
            asset("checksum.txt"),
            asset("mortal-v1.zip"),
            asset("mortal-v2.zip"),
        ];
        let chosen = pick_asset(&assets, None).unwrap();
        assert_eq!(chosen.name, "mortal-v1.zip");
    }

    #[test]
    fn pick_asset_no_zip_errors_without_glob() {
        let assets = vec![asset("checksum.txt"), asset("source.tar.gz")];
        let err = pick_asset(&assets, None).unwrap_err();
        assert!(err.to_string().contains("no .zip asset"));
    }

    #[test]
    fn pick_asset_glob_single_match() {
        let assets = vec![
            asset("mortal-v1.zip"),
            asset("mortal-v1-debug.zip"),
            asset("mortal-v1.sig"),
        ];
        let chosen = pick_asset(&assets, Some("mortal-v?.zip")).unwrap();
        assert_eq!(chosen.name, "mortal-v1.zip");
    }

    #[test]
    fn pick_asset_glob_no_match_errors() {
        let assets = vec![asset("mortal-v1.zip")];
        let err = pick_asset(&assets, Some("nope-*.zip")).unwrap_err();
        assert!(err.to_string().contains("no asset"));
    }

    #[test]
    fn pick_asset_glob_multiple_matches_errors() {
        let assets = vec![asset("mortal-v1.zip"), asset("mortal-v2.zip")];
        let err = pick_asset(&assets, Some("mortal-*.zip")).unwrap_err();
        assert!(err.to_string().contains("matched glob"));
    }

    #[test]
    fn pick_asset_empty_release_errors() {
        let err = pick_asset(&[], None).unwrap_err();
        assert!(err.to_string().contains("no assets"));
    }

    /// Build a tiny zip in memory at `path`, with `entries` like
    /// `("dir/file.txt", "body")`. Directories are inferred.
    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let f = File::create(path).unwrap();
        let mut z = ZipWriter::new(f);
        let opts = SimpleFileOptions::default();
        for (name, body) in entries {
            z.start_file(name.to_string(), opts).unwrap();
            z.write_all(body).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn extract_zip_safe_writes_files() {
        let tmp = TempDir::new().unwrap();
        let zip = tmp.path().join("a.zip");
        make_zip(&zip, &[("bot.py", b"print('hi')\n"), ("README.md", b"# hi\n")]);

        let out = TempDir::new().unwrap();
        extract_zip_safe(&zip, out.path()).unwrap();
        assert!(out.path().join("bot.py").is_file());
        assert!(out.path().join("README.md").is_file());
    }

    #[test]
    fn extract_zip_safe_preserves_directory_layout() {
        let tmp = TempDir::new().unwrap();
        let zip = tmp.path().join("a.zip");
        make_zip(
            &zip,
            &[
                ("mortal-v1/bot.py", b"print('hi')\n"),
                ("mortal-v1/sub/x.txt", b"x"),
            ],
        );

        let out = TempDir::new().unwrap();
        extract_zip_safe(&zip, out.path()).unwrap();
        assert!(out.path().join("mortal-v1/bot.py").is_file());
        assert!(out.path().join("mortal-v1/sub/x.txt").is_file());
    }

    #[test]
    fn extract_zip_safe_rejects_path_traversal() {
        let tmp = TempDir::new().unwrap();
        let zip = tmp.path().join("a.zip");
        make_zip(&zip, &[("../escape.txt", b"nope")]);

        let out = TempDir::new().unwrap();
        let err = extract_zip_safe(&zip, out.path()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unsafe path") || msg.contains("contains `..`"),
            "got: {msg}"
        );
    }

    #[test]
    fn strip_top_level_strips_single_dir() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("mortal-v1/sub")).unwrap();
        std::fs::write(tmp.path().join("mortal-v1/bot.py"), b"").unwrap();

        let resolved = strip_single_top_level(tmp.path()).unwrap();
        assert_eq!(resolved, tmp.path().join("mortal-v1"));
    }

    #[test]
    fn strip_top_level_keeps_root_when_multiple_entries() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("bot.py"), b"").unwrap();
        std::fs::write(tmp.path().join("README.md"), b"").unwrap();

        let resolved = strip_single_top_level(tmp.path()).unwrap();
        assert_eq!(resolved, tmp.path());
    }

    #[test]
    fn validate_layout_requires_bot_py() {
        let tmp = TempDir::new().unwrap();
        let err = validate_layout(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("bot.py"));

        std::fs::write(tmp.path().join("bot.py"), b"").unwrap();
        validate_layout(tmp.path()).unwrap();
    }
}
