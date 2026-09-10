//! Self-update and uninstall workflows behind an injectable system gateway.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::{Command, Stdio};

use anyhow::Context;
use crossterm::style::Color;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::output;
use crate::{ServiceClient, StokerPaths, is_service_unavailable};

use super::{print_info, print_success, print_warning, runtime};

pub(crate) fn update(yes: bool) -> anyhow::Result<()> {
    update_with_gateway(yes, &SystemUpdateGateway)
}

pub(crate) trait UpdateGateway {
    fn latest_release(&self) -> anyhow::Result<GithubRelease>;
    fn download_bytes(&self, url: &str) -> anyhow::Result<Vec<u8>>;
    fn ensure_scheduler_stopped(&self) -> anyhow::Result<()>;
    fn current_executable(&self) -> anyhow::Result<PathBuf>;
    fn install_updated_binary(&self, current_exe: &Path, binary: &[u8]) -> anyhow::Result<()>;
    fn request_confirmation(&self, action: &str) -> anyhow::Result<bool>;
}

pub(crate) struct SystemUpdateGateway;

impl UpdateGateway for SystemUpdateGateway {
    fn latest_release(&self) -> anyhow::Result<GithubRelease> {
        latest_release()
    }

    fn download_bytes(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        download_bytes(url)
    }

    fn ensure_scheduler_stopped(&self) -> anyhow::Result<()> {
        ensure_scheduler_stopped()
    }

    fn current_executable(&self) -> anyhow::Result<PathBuf> {
        std::env::current_exe().context("locate current Stoker executable")
    }

    fn install_updated_binary(&self, current_exe: &Path, binary: &[u8]) -> anyhow::Result<()> {
        install_updated_binary(current_exe, binary)
    }

    fn request_confirmation(&self, action: &str) -> anyhow::Result<bool> {
        request_confirmation(action)
    }
}

pub(crate) fn update_with_gateway<G: UpdateGateway>(yes: bool, gateway: &G) -> anyhow::Result<()> {
    let current =
        Version::parse(env!("CARGO_PKG_VERSION")).context("parse current Stoker version")?;
    let release = gateway.latest_release()?;
    let latest = release.version()?;
    match latest.cmp(&current) {
        std::cmp::Ordering::Less => {
            anyhow::bail!(
                "GitHub reports Stoker {latest}, which is older than the installed {current}; refusing to downgrade"
            );
        }
        std::cmp::Ordering::Equal => {
            print_info(format!("Stoker is already up to date ({current})."));
            return Ok(());
        }
        std::cmp::Ordering::Greater => {}
    }

    print_warning(format!("Stoker will update from {current} to {latest}."));
    if !yes && !gateway.request_confirmation("Continue with update")? {
        print_warning("Update cancelled.");
        return Ok(());
    }

    gateway.ensure_scheduler_stopped()?;
    let current_exe = gateway.current_executable()?;
    let binary = download_release_binary_with_gateway(&release, gateway)?;
    gateway.install_updated_binary(&current_exe, &binary)?;
    #[cfg(unix)]
    print_success(format!("Stoker was updated to {latest}."));
    #[cfg(windows)]
    print_success(
        "Stoker update is being finalized after this command exits. The update helper will report success or failure.",
    );
    Ok(())
}

pub(crate) fn uninstall(yes: bool) -> anyhow::Result<()> {
    let paths = StokerPaths::from_env()?;
    match runtime()?.block_on(ServiceClient::new(paths.clone()).status()) {
        Ok(_) => {
            anyhow::bail!("Scheduler is running. Stop it with 'stoker stop' before uninstalling.")
        }
        Err(error) if is_service_unavailable(&error) => {}
        Err(error) => return Err(error),
    }

    print_warning("Stoker will be uninstalled.");
    print_warning(format!(
        "Job data and logs will be kept at {}.",
        paths.root.display()
    ));
    if !yes && !request_confirmation("Continue with uninstall")? {
        print_warning("Uninstall cancelled.");
        return Ok(());
    }

    let current_exe = std::env::current_exe().context("locate current Stoker executable")?;

    #[cfg(unix)]
    return remove_unix_binary(&current_exe);

    #[cfg(windows)]
    return schedule_windows_uninstall(std::process::id(), &current_exe);
}

#[derive(Debug, Deserialize)]
pub(crate) struct GithubRelease {
    pub(crate) tag_name: String,
    pub(crate) assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GithubAsset {
    pub(crate) name: String,
    pub(crate) browser_download_url: String,
}

impl GithubRelease {
    pub(crate) fn version(&self) -> anyhow::Result<Version> {
        let version = self.tag_name.strip_prefix('v').unwrap_or(&self.tag_name);
        Version::parse(version).context("parse latest GitHub release version")
    }
}

pub(crate) fn latest_release() -> anyhow::Result<GithubRelease> {
    let repository = env!("CARGO_PKG_REPOSITORY")
        .strip_prefix("https://github.com/")
        .and_then(|repository| repository.strip_suffix(".git").or(Some(repository)))
        .filter(|repository| !repository.is_empty())
        .ok_or_else(|| anyhow::anyhow!("package repository is not a GitHub repository"))?;
    let url = format!("https://api.github.com/repos/{repository}/releases/latest");
    let mut response = ureq::get(&url)
        .header("Accept", "application/vnd.github+json")
        .header(
            "User-Agent",
            concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .context("request latest Stoker GitHub release")?;
    let body = response
        .body_mut()
        .read_to_vec()
        .context("read latest Stoker GitHub release")?;
    serde_json::from_slice(&body).context("parse latest Stoker GitHub release")
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub(crate) fn platform_binary_name() -> anyhow::Result<&'static str> {
    Ok("stoker-windows-x86_64.exe")
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) fn platform_binary_name() -> anyhow::Result<&'static str> {
    Ok("stoker-linux-x86_64")
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub(crate) fn platform_binary_name() -> anyhow::Result<&'static str> {
    Ok("stoker-macos-arm64")
}

#[cfg(not(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64")
)))]
pub(crate) fn platform_binary_name() -> anyhow::Result<&'static str> {
    anyhow::bail!("automatic updates are not supported on this platform")
}

pub(crate) fn release_asset<'a>(
    release: &'a GithubRelease,
    name: &str,
) -> anyhow::Result<&'a GithubAsset> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .ok_or_else(|| anyhow::anyhow!("GitHub release does not contain asset {name}"))
}

pub(crate) fn download_bytes(url: &str) -> anyhow::Result<Vec<u8>> {
    let mut response = ureq::get(url)
        .header(
            "User-Agent",
            concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .with_context(|| format!("download {url}"))?;
    response
        .body_mut()
        .read_to_vec()
        .with_context(|| format!("read downloaded content from {url}"))
}

pub(crate) fn checksum_for(checksums: &str, asset_name: &str) -> Option<String> {
    checksums.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let checksum = fields.next()?.trim_start_matches('*');
        let name = fields.next()?.rsplit('/').next()?;
        (name == asset_name).then(|| checksum.to_ascii_lowercase())
    })
}

pub(crate) fn download_release_binary_with_gateway<G: UpdateGateway>(
    release: &GithubRelease,
    gateway: &G,
) -> anyhow::Result<Vec<u8>> {
    let binary_name = platform_binary_name()?;
    let binary_asset = release_asset(release, binary_name)?;
    let checksum_asset = release_asset(release, "SHA256SUMS")?;
    let binary = gateway.download_bytes(&binary_asset.browser_download_url)?;
    let checksums =
        String::from_utf8(gateway.download_bytes(&checksum_asset.browser_download_url)?)
            .context("decode SHA256SUMS")?;
    let expected = checksum_for(&checksums, binary_name)
        .ok_or_else(|| anyhow::anyhow!("SHA256SUMS does not contain {binary_name}"))?;
    let actual = Sha256::digest(&binary)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual != expected {
        anyhow::bail!("SHA-256 mismatch for {binary_name}");
    }
    Ok(binary)
}

pub(crate) fn ensure_scheduler_stopped() -> anyhow::Result<()> {
    let paths = StokerPaths::from_env()?;
    match runtime()?.block_on(ServiceClient::new(paths).status()) {
        Ok(_) => anyhow::bail!("Scheduler is running. Stop it with 'stoker stop' before updating."),
        Err(error) if is_service_unavailable(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

pub(crate) fn install_updated_binary(current_exe: &Path, binary: &[u8]) -> anyhow::Result<()> {
    let update_dir = std::env::temp_dir().join(format!("stoker-update-{}", Uuid::new_v4()));
    fs::create_dir(&update_dir).context("create Stoker update directory")?;
    let update_binary = update_dir.join(platform_binary_name()?);
    if let Err(error) = fs::write(&update_binary, binary) {
        let _ = fs::remove_dir_all(&update_dir);
        return Err(error).context("write downloaded Stoker binary");
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::metadata(current_exe)
            .context("read current Stoker executable permissions")?
            .permissions();
        fs::set_permissions(
            &update_binary,
            fs::Permissions::from_mode(permissions.mode()),
        )
        .context("set updated Stoker executable permissions")?;
        if let Err(error) = fs::rename(&update_binary, current_exe) {
            let _ = fs::remove_dir_all(&update_dir);
            return Err(error).context("replace current Stoker executable");
        }
        fs::remove_dir_all(&update_dir).context("remove Stoker update directory")?;
        return Ok(());
    }

    #[cfg(windows)]
    {
        schedule_windows_update(std::process::id(), current_exe, &update_binary, &update_dir)?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    {
        let _ = fs::remove_dir_all(&update_dir);
        anyhow::bail!("updating is not supported on this platform")
    }
}

#[cfg(unix)]
pub(crate) fn remove_unix_binary(executable: &Path) -> anyhow::Result<()> {
    fs::remove_file(executable)
        .with_context(|| format!("remove Stoker executable at {}", executable.display()))?;
    print_success("Stoker has been uninstalled. Job data and logs were kept.");
    Ok(())
}

pub(crate) fn request_confirmation(action: &str) -> anyhow::Result<bool> {
    print!(
        "{}",
        output::paint_bold(
            format!("{action}? [y/N]: "),
            Color::Yellow,
            output::stdout_color_enabled(),
        )
    );
    io::stdout().flush().context("write confirmation prompt")?;
    let mut response = String::new();
    io::stdin()
        .read_line(&mut response)
        .context("read confirmation")?;
    Ok(is_confirmation(&response))
}

pub(crate) fn is_confirmation(response: &str) -> bool {
    matches!(response.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

#[cfg(windows)]
pub(crate) fn schedule_windows_uninstall(process_id: u32, executable: &Path) -> anyhow::Result<()> {
    let script = std::env::temp_dir().join(format!("stoker-uninstall-{}.cmd", Uuid::new_v4()));
    fs::write(&script, windows_uninstall_script(process_id, executable))
        .context("create Windows uninstall helper")?;
    let command = std::env::var_os("ComSpec").unwrap_or_else(|| "cmd.exe".into());
    Command::new(command)
        .args(["/C", script.to_string_lossy().as_ref()])
        .stdin(Stdio::null())
        .spawn()
        .context("schedule Windows uninstall helper")?;
    print_success("Uninstall scheduled. It will run after Stoker exits.");
    Ok(())
}

#[cfg(windows)]
pub(crate) fn schedule_windows_update(
    process_id: u32,
    executable: &Path,
    update_binary: &Path,
    update_dir: &Path,
) -> anyhow::Result<()> {
    let script = std::env::temp_dir().join(format!("stoker-update-{}.cmd", Uuid::new_v4()));
    fs::write(
        &script,
        windows_update_script(process_id, executable, update_binary, update_dir),
    )
    .context("create Windows update helper")?;
    let command = std::env::var_os("ComSpec").unwrap_or_else(|| "cmd.exe".into());
    Command::new(command)
        .args(["/C", script.to_string_lossy().as_ref()])
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("schedule Windows update helper")?;
    Ok(())
}

#[cfg(any(windows, test))]
pub(crate) fn windows_uninstall_script(process_id: u32, executable: &Path) -> String {
    format!(
        "@echo off\r\n:wait_for_stoker\r\ntasklist /FI \"PID eq {process_id}\" /NH | findstr \"{process_id}\" >NUL\r\nif not errorlevel 1 (\r\n  timeout /t 1 /nobreak >NUL\r\n  goto wait_for_stoker\r\n)\r\ndel /F /Q \"{}\"\r\nstart \"\" /B \"%ComSpec%\" /C del /F /Q \"%~f0\" >NUL 2>&1\r\nexit /B 0\r\n",
        executable.display()
    )
}

#[cfg(any(windows, test))]
pub(crate) fn windows_update_script(
    process_id: u32,
    executable: &Path,
    update_binary: &Path,
    update_dir: &Path,
) -> String {
    format!(
        "@echo off\r\n:wait_for_stoker\r\ntasklist /FI \"PID eq {process_id}\" /NH | findstr \"{process_id}\" >NUL\r\nif not errorlevel 1 (\r\n  timeout /t 1 /nobreak >NUL\r\n  goto wait_for_stoker\r\n)\r\nmove /Y \"{}\" \"{}\" >NUL\r\nif errorlevel 1 (\r\n  echo Stoker update failed: could not replace the executable.\r\n  exit /b 1\r\n)\r\nrmdir /S /Q \"{}\"\r\nif errorlevel 1 (\r\n  echo Stoker update completed, but cleanup failed.\r\n) else (\r\n  echo Stoker update completed successfully.\r\n)\r\nstart \"\" /B \"%ComSpec%\" /C del /F /Q \"%~f0\" >NUL 2>&1\r\nexit /B 0\r\n",
        update_binary.display(),
        executable.display(),
        update_dir.display()
    )
}
