mod config;
#[cfg(test)]
mod config_tests;
mod discovery;
mod status;
#[cfg(test)]
pub(super) mod test_support;

pub(crate) use self::discovery::automatic_workspace_label;

/// The `remote.<name>.url` settings of the repo `cwd` is in, as `(name, url)`.
/// Reads config files only; it runs no git process.
pub fn git_remote_urls(cwd: &std::path::Path) -> Vec<(String, String)> {
    discovery::git_worktree_info(cwd)
        .map(|info| config::read_remote_urls(&info))
        .unwrap_or_default()
}

pub use self::{
    discovery::{
        derive_label_from_cwd, fallback_label_from_cwd, git_branch, git_detached_head,
        git_space_metadata, DetachedHead, GitSpaceMetadata,
    },
    status::{
        git_status_cache_key, git_status_cache_key_for_space,
        git_status_snapshot_for_cwd_with_demand, GitStatusCacheEntry, GitStatusRefreshDemand,
    },
};
