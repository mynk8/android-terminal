use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use ndk::asset::AssetManager;
use std::ffi::CString;
use zip::ZipArchive;

const BOOTSTRAP_ASSET: &str = "bootstrap-aarch64.zip";
const PREFIX_DIR: &str = "prefix";
const STAGING_DIR: &str = "prefix-staging";
const SYMLINKS_FILE: &str = "SYMLINKS.txt";
const SHELL_REL_PATH: &str = "bin/sh";
const TERMUX_EXEC_REL_PATH: &str = "lib/libtermux-exec.so";
const TERMUX_EXEC_COMPAT_ASSET: &str = "libtermux-exec.so";
const PATH_PATCH_STAMP: &str = ".gui-engine-termux-paths-v3";
const LEGACY_TERMUX_PREFIX: &str = "/data/data/com.termux/files/usr";
const LEGACY_TERMUX_PREFIX_USER: &str = "/data/user/0/com.termux/files/usr";
const LEGACY_TERMUX_HOME: &str = "/data/data/com.termux/files/home";
const LEGACY_TERMUX_HOME_USER: &str = "/data/user/0/com.termux/files/home";
const LEGACY_TERMUX_CACHE: &str = "/data/data/com.termux/cache";
const LEGACY_TERMUX_CACHE_USER: &str = "/data/user/0/com.termux/cache";
const LEGACY_TERMUX_REPO_CF_HOST: &str = "packages-cf.termux.org";
const LEGACY_TERMUX_REPO_HOST: &str = "packages.termux.org";
const CURRENT_TERMUX_REPO_CF_HOST: &str = "packages-cf.termux.dev";
const CURRENT_TERMUX_REPO_HOST: &str = "packages.termux.dev";
const APT_CONFIG_REL_PATH: &str = "etc/apt/apt.conf";

pub struct BootstrapPaths {
    pub prefix: PathBuf,
    pub home: PathBuf,
    pub tmp: PathBuf,
}

pub fn setup_bootstrap_if_needed(base: &Path, assets: &AssetManager) -> io::Result<BootstrapPaths> {
    let prefix = base.join(PREFIX_DIR);
    let home = base.join("home");
    let tmp = base.join("tmp");

    log::info!("Bootstrap base dir: {:?}", base);
    if is_prefix_ready(&prefix)? {
        apply_termux_path_rewrites_if_needed(base, &prefix, &home)?;
        ensure_apt_runtime_config(base, &prefix)?;
        install_termux_exec_compat_if_available(assets, &prefix)?;
        log::info!("Bootstrap prefix already initialized: {:?}", prefix);
        return Ok(BootstrapPaths { prefix, home, tmp });
    }
    if prefix.exists() {
        log::warn!("Existing prefix is incomplete; reinstalling bootstrap");
        let _ = fs::remove_dir_all(&prefix);
    }

    let staging = base.join(STAGING_DIR);
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    fs::create_dir_all(&staging)?;
    fs::create_dir_all(&home)?;
    fs::create_dir_all(&tmp)?;
    set_permissions_best_effort(&staging, 0o700);
    set_permissions_best_effort(&home, 0o700);
    set_permissions_best_effort(&tmp, 0o700);

    log::info!("Extracting bootstrap asset: {}", BOOTSTRAP_ASSET);
    let zip_bytes = load_asset(assets, BOOTSTRAP_ASSET)?;
    let reader = std::io::Cursor::new(zip_bytes);
    let mut archive =
        ZipArchive::new(reader).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let mut symlinks: Vec<(String, String)> = Vec::new();

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let name = file.name().to_string();
        if name == SYMLINKS_FILE {
            let mut buf = String::new();
            file.read_to_string(&mut buf)?;
            for line in buf.lines() {
                let parts: Vec<&str> = line.split('←').collect();
                if parts.len() != 2 {
                    continue;
                }
                let old_path = parts[0].to_string();
                let new_path = staging.join(parts[1]).to_string_lossy().to_string();
                if let Some(parent) = Path::new(&new_path).parent() {
                    let _ = fs::create_dir_all(parent);
                }
                symlinks.push((old_path, new_path));
            }
            continue;
        }

        let out_path = staging.join(&name);
        if file.is_dir() {
            fs::create_dir_all(&out_path)?;
            set_permissions_best_effort(&out_path, dir_mode(file.unix_mode()));
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
                set_permissions_best_effort(parent, 0o700);
            }
            let mut out = fs::File::create(&out_path)?;
            io::copy(&mut file, &mut out)?;
            set_permissions_best_effort(&out_path, file_mode(&name, file.unix_mode()));
        }
    }

    log::info!("Applying {} symlinks", symlinks.len());
    for (old_path, new_path) in symlinks {
        let _ = fs::remove_file(&new_path);
        let _ = std::os::unix::fs::symlink(old_path, new_path);
    }

    if prefix.exists() {
        let _ = fs::remove_dir_all(&prefix);
    }
    fs::rename(&staging, &prefix)?;
    set_permissions_best_effort(&prefix, 0o700);
    apply_termux_path_rewrites_if_needed(base, &prefix, &home)?;
    ensure_apt_runtime_config(base, &prefix)?;
    install_termux_exec_compat_if_available(assets, &prefix)?;

    log::info!("Bootstrap installed at {:?}", prefix);

    Ok(BootstrapPaths { prefix, home, tmp })
}

fn load_asset(assets: &AssetManager, name: &str) -> io::Result<Vec<u8>> {
    let c_name = CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid asset name"))?;
    let mut asset = assets
        .open(&c_name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "bootstrap asset not found"))?;
    let mut buf = Vec::new();
    asset.read_to_end(&mut buf)?;
    Ok(buf)
}

fn has_files(path: &Path) -> io::Result<bool> {
    let mut iter = fs::read_dir(path)?;
    Ok(iter.next().is_some())
}

fn is_prefix_ready(prefix: &Path) -> io::Result<bool> {
    if !prefix.exists() || !has_files(prefix)? {
        return Ok(false);
    }
    let shell = prefix.join(SHELL_REL_PATH);
    if !shell.is_file() {
        return Ok(false);
    }
    let metadata = fs::metadata(shell)?;
    if metadata.permissions().mode() & 0o111 == 0 {
        return Ok(false);
    }
    let termux_exec = prefix.join(TERMUX_EXEC_REL_PATH);
    Ok(termux_exec.is_file())
}

fn set_permissions_best_effort(path: &Path, mode: u32) {
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777));
}

fn normalize_mode(zip_mode: Option<u32>, fallback: u32) -> u32 {
    zip_mode
        .map(|m| m & 0o777)
        .filter(|m| *m != 0)
        .unwrap_or(fallback)
}

fn dir_mode(zip_mode: Option<u32>) -> u32 {
    normalize_mode(zip_mode, 0o700)
}

fn file_mode(path: &str, zip_mode: Option<u32>) -> u32 {
    let mode = normalize_mode(
        zip_mode,
        if should_be_executable(path) {
            0o700
        } else {
            0o600
        },
    );
    if should_be_executable(path) {
        mode | 0o100
    } else {
        mode
    }
}

fn should_be_executable(path: &str) -> bool {
    path.starts_with("bin/")
        || path.starts_with("libexec/")
        || path == "lib/apt/apt-helper"
        || path.starts_with("lib/apt/methods/")
}

fn apply_termux_path_rewrites_if_needed(base: &Path, prefix: &Path, home: &Path) -> io::Result<()> {
    let app_data_dir = base.parent().unwrap_or(base);
    let cache = app_data_dir.join("cache");
    fs::create_dir_all(&cache)?;
    set_permissions_best_effort(&cache, 0o700);

    let prefix_str = prefix.to_string_lossy().to_string();
    let home_str = home.to_string_lossy().to_string();
    let cache_str = cache.to_string_lossy().to_string();

    let stamp_payload = format!(
        "prefix={}\nhome={}\ncache={}\n",
        prefix_str, home_str, cache_str
    );
    let stamp_path = prefix.join(PATH_PATCH_STAMP);
    let replacements = vec![
        (
            LEGACY_TERMUX_REPO_CF_HOST.to_string(),
            CURRENT_TERMUX_REPO_CF_HOST.to_string(),
        ),
        (
            LEGACY_TERMUX_REPO_HOST.to_string(),
            CURRENT_TERMUX_REPO_HOST.to_string(),
        ),
        (LEGACY_TERMUX_PREFIX.to_string(), prefix_str.clone()),
        (LEGACY_TERMUX_PREFIX_USER.to_string(), prefix_str),
        (LEGACY_TERMUX_HOME.to_string(), home_str.clone()),
        (LEGACY_TERMUX_HOME_USER.to_string(), home_str),
        (LEGACY_TERMUX_CACHE.to_string(), cache_str.clone()),
        (LEGACY_TERMUX_CACHE_USER.to_string(), cache_str),
    ];

    if let Ok(existing) = fs::read_to_string(&stamp_path) {
        if existing == stamp_payload {
            rewrite_dynamic_termux_paths(prefix, &replacements)?;
            return Ok(());
        }
    }

    let mut stats = RewriteStats::default();
    rewrite_legacy_termux_paths(prefix, &replacements, &mut stats)?;
    rewrite_dynamic_termux_paths(prefix, &replacements)?;
    fs::write(&stamp_path, stamp_payload)?;
    set_permissions_best_effort(&stamp_path, 0o600);

    log::info!(
        "Patched legacy Termux paths: files_changed={}, replacements={}",
        stats.files_changed,
        stats.replacements
    );
    Ok(())
}

#[derive(Default)]
struct RewriteStats {
    files_changed: usize,
    replacements: usize,
}

fn rewrite_legacy_termux_paths(
    path: &Path,
    replacements: &[(String, String)],
    stats: &mut RewriteStats,
) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Ok(());
    }

    if file_type.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            rewrite_legacy_termux_paths(&entry.path(), replacements, stats)?;
        }
        return Ok(());
    }

    if !file_type.is_file() || metadata.len() > 4 * 1024 * 1024 {
        return Ok(());
    }

    let mut data = fs::read(path)?;
    if data.len() >= 4 && data[..4] == [0x7f, b'E', b'L', b'F'] {
        return Ok(());
    }
    if data.contains(&0) {
        return Ok(());
    }

    let mut replaced_any = false;
    for (from, to) in replacements {
        let (next, count) = replace_all_bytes(&data, from.as_bytes(), to.as_bytes());
        if count > 0 {
            data = next;
            stats.replacements += count;
            replaced_any = true;
        }
    }

    if replaced_any {
        fs::write(path, &data)?;
        stats.files_changed += 1;
    }

    Ok(())
}

fn rewrite_dynamic_termux_paths(prefix: &Path, replacements: &[(String, String)]) -> io::Result<()> {
    let mut stats = RewriteStats::default();
    let dynamic_dirs = [
        prefix.join("var/lib/dpkg/info"),
        prefix.join("var/lib/dpkg/triggers"),
        prefix.join("var/lib/dpkg/updates"),
    ];

    for dir in dynamic_dirs {
        if dir.exists() {
            rewrite_legacy_termux_paths(&dir, replacements, &mut stats)?;
        }
    }

    if stats.files_changed > 0 {
        log::info!(
            "Patched dynamic dpkg metadata: files_changed={}, replacements={}",
            stats.files_changed,
            stats.replacements
        );
    }
    Ok(())
}

fn replace_all_bytes(haystack: &[u8], needle: &[u8], replacement: &[u8]) -> (Vec<u8>, usize) {
    if needle.is_empty() {
        return (haystack.to_vec(), 0);
    }

    let mut out = Vec::with_capacity(haystack.len());
    let mut i = 0;
    let mut count = 0;

    while i < haystack.len() {
        if i + needle.len() <= haystack.len() && &haystack[i..i + needle.len()] == needle {
            out.extend_from_slice(replacement);
            i += needle.len();
            count += 1;
        } else {
            out.push(haystack[i]);
            i += 1;
        }
    }

    (out, count)
}

fn ensure_apt_runtime_config(base: &Path, prefix: &Path) -> io::Result<()> {
    let app_data_dir = base.parent().unwrap_or(base);
    let cache_dir = app_data_dir.join("cache").join("apt");
    fs::create_dir_all(&cache_dir)?;
    set_permissions_best_effort(&cache_dir, 0o700);

    let apt_cfg = format!(
        "Dir \"{prefix}\";\n\
Dir::Etc \"{prefix}/etc/apt\";\n\
Dir::Etc::trusted \"{prefix}/etc/apt/trusted.gpg\";\n\
Dir::Etc::trustedparts \"{prefix}/etc/apt/trusted.gpg.d\";\n\
Dir::State \"{prefix}/var/lib/apt\";\n\
Dir::State::status \"{prefix}/var/lib/dpkg/status\";\n\
Dir::Cache \"{cache}\";\n\
Dir::Bin \"{prefix}/bin\";\n\
Dir::Bin::dpkg \"{prefix}/bin/dpkg\";\n\
Dir::Bin::apt-key \"{prefix}/bin/apt-key\";\n\
Dir::Bin::gpg \"{prefix}/bin/gpg\";\n\
Dir::Bin::gpgv \"{prefix}/bin/gpgv\";\n\
APT::Key::gpgvcommand \"{prefix}/bin/gpgv\";\n\
Dir::Bin::methods \"{prefix}/lib/apt/methods\";\n\
Dir::Log \"{prefix}/var/log/apt\";\n\
DPkg::Options:: \"--instdir={prefix}\";\n\
DPkg::Options:: \"--admindir={prefix}/var/lib/dpkg\";\n\
APT::Get::AllowUnauthenticated \"false\";\n\
Acquire::AllowInsecureRepositories \"false\";\n\
Acquire::AllowDowngradeToInsecureRepositories \"false\";\n\
Acquire::https::Verify-Peer \"true\";\n\
Acquire::https::Verify-Host \"true\";\n\
Acquire::https::CaInfo \"{prefix}/etc/tls/cert.pem\";\n",
        prefix = prefix.to_string_lossy(),
        cache = cache_dir.to_string_lossy()
    );

    let cfg_path = prefix.join(APT_CONFIG_REL_PATH);
    if let Some(parent) = cfg_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&cfg_path, apt_cfg)?;
    set_permissions_best_effort(&cfg_path, 0o600);
    Ok(())
}

fn install_termux_exec_compat_if_available(assets: &AssetManager, prefix: &Path) -> io::Result<()> {
    match load_asset(assets, TERMUX_EXEC_COMPAT_ASSET) {
        Ok(bytes) => {
            let target = prefix.join(TERMUX_EXEC_REL_PATH);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&target, bytes)?;
            set_permissions_best_effort(&target, 0o700);
            log::info!(
                "Installed termux-exec compatibility library at {:?}",
                target
            );
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            log::warn!(
                "No '{}' asset found; keeping bootstrap termux-exec library",
                TERMUX_EXEC_COMPAT_ASSET
            );
            Ok(())
        }
        Err(e) => Err(e),
    }
}
