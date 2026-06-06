use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use tar::Archive;
use zip::ZipArchive;

use super::platform::Platform;

const BIN_NAME: &str = "agnes-aigc-gen";

pub fn extract_binary(archive_bytes: &[u8], platform: &Platform) -> Result<Vec<u8>> {
    if platform.slug == "windows-x86_64" {
        extract_from_zip(archive_bytes)
    } else {
        extract_from_tar_gz(archive_bytes)
    }
}

fn extract_from_tar_gz(archive_bytes: &[u8]) -> Result<Vec<u8>> {
    let decoder = GzDecoder::new(archive_bytes);
    let mut archive = Archive::new(decoder);
    for entry in archive.entries().context("read tar entries")? {
        let mut entry = entry.context("read tar entry")?;
        let path = entry.path().context("tar entry path")?;
        if path.file_name().and_then(|name| name.to_str()) == Some(BIN_NAME) {
            let mut data = Vec::new();
            entry.read_to_end(&mut data).context("read binary from tar")?;
            return Ok(data);
        }
    }
    bail!("binary {BIN_NAME} not found in release archive")
}

fn extract_from_zip(archive_bytes: &[u8]) -> Result<Vec<u8>> {
    let cursor = std::io::Cursor::new(archive_bytes);
    let mut archive = ZipArchive::new(cursor).context("open zip archive")?;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).context("read zip entry")?;
        let name = file.name().to_string();
        if name == format!("{BIN_NAME}.exe") || name.ends_with(&format!("/{BIN_NAME}.exe")) {
            let mut data = Vec::new();
            file.read_to_end(&mut data).context("read binary from zip")?;
            return Ok(data);
        }
    }
    bail!("binary {BIN_NAME}.exe not found in release archive")
}

pub fn replace_binary(target: &Path, new_binary: &[u8]) -> Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }

    let temp_path = if target.extension().is_some_and(|ext| ext == "exe") {
        target.with_extension("new.exe")
    } else {
        Path::new(&format!("{}.new", target.display())).to_path_buf()
    };

    {
        let mut file = fs::File::create(&temp_path).with_context(|| format!("create {}", temp_path.display()))?;
        file.write_all(new_binary).context("write new binary")?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&temp_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&temp_path, perms)?;
    }

    if target.exists() {
        fs::remove_file(target).with_context(|| format!("remove {}", target.display()))?;
    }
    fs::rename(&temp_path, target).with_context(|| format!("install updated binary to {}", target.display()))?;
    Ok(())
}
