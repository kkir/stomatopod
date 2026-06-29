//! Startup guard that refuses to run the embedded backend on ephemeral
//! container storage.
//!
//! The embedded backend keeps *all* state (SQLite metadata, WAL, and Parquet
//! files) under `data_dir`. Inside a container that directory lives in the
//! writable image layer unless a volume is mounted over it, so every redeploy
//! silently wipes the data. This guard turns that silent data loss into a hard
//! startup failure with an actionable message.
//!
//! It only enforces when running inside a container; bare-metal installs and
//! local development (where the operator owns the filesystem) are untouched.
//! The postgres/clickhouse backends never call this — their durability lives in
//! the external database.

use std::path::Path;

use stomatopod_core::config::EmbeddedConfig;
use tracing::warn;

/// Verify that `data_dir` will survive a container redeploy, or bail with an
/// actionable error. See the module docs for the decision logic.
pub fn ensure_persistent(cfg: &EmbeddedConfig, data_dir: &Path) -> anyhow::Result<()> {
    // Operator opted into throwaway storage (demos / ephemeral test containers).
    if cfg.allow_ephemeral {
        warn!(
            "storage.allow_ephemeral is set: embedded data dir {} is not required to be \
             persistent — data may be lost on redeploy",
            data_dir.display()
        );
        return Ok(());
    }

    // Only containers conflate "the app's data dir" with "ephemeral storage".
    // Outside a container the operator manages persistence themselves.
    if !in_container() {
        return Ok(());
    }

    if data_dir_is_mounted(data_dir) {
        return Ok(());
    }

    anyhow::bail!(
        "embedded data dir {dir} is on ephemeral container storage — it is NOT a mounted \
         volume, so all analytics data (sites, users, API keys, events) will be lost on the \
         next redeploy.\n\n\
         Fix one of the following:\n  \
         • docker run:    add  -v stomatopod_data:/app/data\n  \
         • docker compose: use the bundled docker-compose.yml (named volume)\n  \
         • PaaS / Kubernetes: mount a persistent disk at {dir} (or set \
         STOMATOPOD_STORAGE__DATA_DIR to the mount path)\n  \
         • or switch to a managed database with  backend = \"postgres\"\n\n\
         See DEPLOY.md for details. To intentionally run without persistence (demos, tests), \
         set  STOMATOPOD_STORAGE__ALLOW_EPHEMERAL=true",
        dir = data_dir.display(),
    )
}

/// True when running inside a Docker (`/.dockerenv`) or Podman
/// (`/run/.containerenv`) container.
fn in_container() -> bool {
    Path::new("/.dockerenv").exists() || Path::new("/run/.containerenv").exists()
}

/// True when `data_dir`, or any ancestor below the filesystem root, is a mount
/// point listed in `/proc/self/mountinfo` — i.e. backed by a bind mount or
/// named/anonymous volume rather than the container's root overlay.
fn data_dir_is_mounted(data_dir: &Path) -> bool {
    // Resolve to an absolute, symlink-free path so it matches mountinfo entries.
    // `data_dir` was just created by the caller, so the nearest existing
    // ancestor we can canonicalize is the dir itself.
    let canonical = match data_dir.canonicalize() {
        Ok(p) => p,
        // If we cannot resolve the path we cannot prove persistence; treat it
        // as not mounted so the guard errs on the side of failing loudly.
        Err(_) => return false,
    };

    let mountinfo = match std::fs::read_to_string("/proc/self/mountinfo") {
        Ok(s) => s,
        // No mountinfo (non-Linux container?) — can't verify, don't block.
        Err(_) => return true,
    };

    // mountinfo line layout: "ID parentID major:minor root MOUNTPOINT ...".
    // The mount point is the 5th whitespace-separated field; paths are escaped
    // with octal sequences (e.g. a space is "\040").
    let mount_points: Vec<String> = mountinfo
        .lines()
        .filter_map(|line| line.split_whitespace().nth(4))
        .map(unescape_mountinfo)
        .collect();

    // The root mount "/" always exists; matching it would defeat the check, so
    // we only accept a mount at data_dir or an intermediate ancestor.
    canonical
        .ancestors()
        .take_while(|p| p.as_os_str() != "/")
        .any(|ancestor| mount_points.iter().any(|mp| Path::new(mp) == ancestor))
}

/// Decode the octal escape sequences (`\040`, `\011`, `\012`, `\134`) that the
/// kernel uses for whitespace and backslashes in mountinfo paths.
fn unescape_mountinfo(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            let oct = &s[i + 1..i + 4];
            if let Ok(code) = u8::from_str_radix(oct, 8) {
                out.push(code as char);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}
