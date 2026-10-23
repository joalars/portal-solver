use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use reqwest::blocking::Client;
use semver::Version;
use serde::Deserialize;

const RELEASES_URL: &str = "https://api.github.com/repos/joalars/portal-solver/releases/latest";
const ASSET_URL_PREFIX: &str = "https://github.com/joalars/portal-solver/releases/download/";
const MAX_DOWNLOAD_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    download_url: String,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

pub fn check_for_update() -> Result<Option<Release>, String> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return Err("Automatic updates are currently supported on Linux x86_64 only.".to_owned());
    }
    let client = client()?;
    let release: GithubRelease = client
        .get(RELEASES_URL)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .and_then(|response| response.json())
        .map_err(|error| format!("Could not check GitHub releases: {error}"))?;
    let latest = parse_version(&release.tag_name).ok_or_else(|| {
        format!(
            "GitHub release tag '{}' is not a valid version.",
            release.tag_name
        )
    })?;
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|error| format!("The app's version is invalid: {error}"))?;
    if latest <= current {
        return Ok(None);
    }

    let asset_name = release_asset_name();
    let asset = release
        .assets
        .into_iter()
        .find(|asset| asset.name == asset_name)
        .ok_or_else(|| {
            format!(
                "Release {} does not include the required asset '{asset_name}'.",
                release.tag_name
            )
        })?;
    if !asset.browser_download_url.starts_with(ASSET_URL_PREFIX) {
        return Err("GitHub returned an unexpected release asset URL.".to_owned());
    }

    Ok(Some(Release {
        version: latest.to_string(),
        download_url: asset.browser_download_url,
    }))
}

pub fn install_and_restart(release: &Release) -> Result<(), String> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return Err("Automatic updates are currently supported on Linux x86_64 only.".to_owned());
    }
    let executable =
        std::env::current_exe().map_err(|error| format!("Could not locate the app: {error}"))?;
    let directory = executable
        .parent()
        .ok_or_else(|| "Could not locate the app directory.".to_owned())?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock error: {error}"))?
        .as_nanos();
    let temporary = directory.join(format!(
        ".portal-solver-update-{}-{nonce}.tmp",
        std::process::id()
    ));
    let backup = directory.join(format!(
        ".portal-solver-backup-{}-{nonce}",
        std::process::id()
    ));

    let result = download_update(&release.download_url, &temporary, &executable);
    if let Err(error) = result {
        remove_temporary(&temporary, &error)?;
        return Err(error);
    }

    if let Err(error) = create_backup(&executable, &backup) {
        remove_temporary(&temporary, &error.to_string())?;
        return Err(format!("Could not back up the current app: {error}"));
    }
    if let Err(error) = fs::rename(&temporary, &executable) {
        let remove_backup = fs::remove_file(&backup);
        remove_temporary(&temporary, &error.to_string())?;
        if let Err(cleanup_error) = remove_backup {
            return Err(format!(
                "Could not install update: {error}; also failed to remove backup: {cleanup_error}"
            ));
        }
        return Err(format!("Could not install update: {error}"));
    }

    let restart = Command::new(&executable).spawn();
    if let Err(error) = restart {
        let restore = fs::rename(&backup, &executable);
        return match restore {
            Ok(()) => Err(format!(
                "Update installed but restart failed; the previous app was restored: {error}"
            )),
            Err(restore_error) => Err(format!(
                "App restart failed: {error}; also failed to restore the previous app: {restore_error}"
            )),
        };
    }

    if let Err(error) = fs::remove_file(&backup) {
        eprintln!(
            "Update succeeded, but could not remove backup {}: {error}",
            backup.display()
        );
    }
    Ok(())
}

fn client() -> Result<Client, String> {
    Client::builder()
        .user_agent(concat!("portal-solver/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|error| format!("Could not initialize GitHub client: {error}"))
}

fn parse_version(tag: &str) -> Option<Version> {
    Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()
}

fn release_asset_name() -> String {
    format!("portal-solver-linux-{}", std::env::consts::ARCH)
}

fn download_update(url: &str, temporary: &Path, executable: &Path) -> Result<(), String> {
    let response = client()?
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("Could not download update: {error}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_DOWNLOAD_BYTES)
    {
        return Err("The update asset is larger than the 256 MiB safety limit.".to_owned());
    }

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(temporary)
        .map_err(|error| format!("Could not create temporary update file: {error}"))?;
    let mut response = response.take(MAX_DOWNLOAD_BYTES + 1);
    let downloaded = io::copy(&mut response, &mut file)
        .map_err(|error| format!("Could not write update: {error}"))?;
    if downloaded > MAX_DOWNLOAD_BYTES {
        return Err("The update asset is larger than the 256 MiB safety limit.".to_owned());
    }
    file.sync_all()
        .map_err(|error| format!("Could not flush downloaded update: {error}"))?;
    validate_elf(&mut file)?;

    let permissions = fs::metadata(executable)
        .map_err(|error| format!("Could not read current app permissions: {error}"))?
        .permissions();
    fs::set_permissions(temporary, permissions)
        .map_err(|error| format!("Could not make the update executable: {error}"))?;
    Ok(())
}

fn create_backup(executable: &Path, backup: &Path) -> io::Result<()> {
    let permissions = fs::metadata(executable)?.permissions();
    let mut source = File::open(executable)?;
    let mut destination = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(backup)?;
    let result = io::copy(&mut source, &mut destination)
        .and_then(|_| destination.sync_all())
        .and_then(|_| fs::set_permissions(backup, permissions));
    if let Err(error) = result {
        drop(destination);
        if let Err(cleanup_error) = fs::remove_file(backup) {
            return Err(io::Error::other(format!(
                "{error}; failed to remove incomplete backup: {cleanup_error}"
            )));
        }
        return Err(error);
    }
    Ok(())
}

fn validate_elf(file: &mut File) -> Result<(), String> {
    let mut header = [0; 20];
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_exact(&mut header))
        .map_err(|error| format!("Downloaded file is not a valid Linux executable: {error}"))?;
    if &header[..4] != b"\x7fELF" || header[4] != 2 || header[5] != 1 {
        return Err("Downloaded file is not a 64-bit little-endian Linux executable.".to_owned());
    }
    if u16::from_le_bytes([header[18], header[19]]) != 62 {
        return Err("Downloaded executable is not built for x86_64.".to_owned());
    }
    Ok(())
}

fn remove_temporary(path: &PathBuf, original_error: &str) -> Result<(), String> {
    if path.exists() {
        fs::remove_file(path).map_err(|cleanup_error| {
            format!(
                "{original_error}; also failed to remove temporary update {}: {cleanup_error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_version, release_asset_name, validate_elf};
    use std::{
        fs::{self, OpenOptions},
        io::Write,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn parses_version_tags_with_or_without_v_prefix() {
        assert_eq!(parse_version("v1.2.3").unwrap().to_string(), "1.2.3");
        assert_eq!(parse_version("1.2.3").unwrap().to_string(), "1.2.3");
        assert!(parse_version("latest").is_none());
    }

    #[test]
    fn names_release_asset_for_current_architecture() {
        assert_eq!(
            release_asset_name(),
            format!("portal-solver-linux-{}", std::env::consts::ARCH)
        );
    }

    #[test]
    fn validates_elf_header_on_read_write_download_file() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "portal-solver-elf-test-{}-{unique}",
            std::process::id()
        ));
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let mut header = [0_u8; 20];
        header[..6].copy_from_slice(b"\x7fELF\x02\x01");
        header[18..20].copy_from_slice(&62_u16.to_le_bytes());
        file.write_all(&header).unwrap();

        assert!(validate_elf(&mut file).is_ok());

        drop(file);
        fs::remove_file(path).unwrap();
    }
}
