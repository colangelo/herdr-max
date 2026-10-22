//! Which forge repo a pane is in (fork issue 241).
//!
//! `issue.list {pane}` shows the issues of the repo the pane works in. The
//! repo is named by the git remote of the pane's folder: `internal` first,
//! then `origin`. A linked worktree reads its main repo's config, so it
//! resolves to the same repo. Reading config files touches the disk, so it is
//! done on a short-lived thread, never in the app loop; the answer is cached
//! per folder and read again once it is old.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The remotes to try, best first.
const REMOTES: [&str; 2] = ["internal", "origin"];
/// How long an answer is trusted before it is read again.
const FRESH: Duration = Duration::from_secs(60);
/// Folders remembered at most; the cache is emptied when it grows past this.
const MAX_FOLDERS: usize = 256;

/// A forge repo: the host (lowercase, no port) and the `owner/repo` path
/// (no `.git`, no slashes at the ends).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepoRemote {
    pub(crate) host: String,
    pub(crate) path: String,
}

impl RepoRemote {
    /// Whether a project (`host`, `path`) is this repo. Case is ignored. A
    /// project that names no host matches on the path alone.
    pub(crate) fn is(&self, host: &str, path: &str) -> bool {
        self.path.eq_ignore_ascii_case(path.trim_matches('/'))
            && (host.is_empty() || self.host.eq_ignore_ascii_case(host))
    }
}

/// `https://host/owner/repo.git`, `ssh://git@host:2222/owner/repo` and
/// `git@host:owner/repo.git` all name a repo. Anything else (a local path, no
/// owner) does not.
pub(crate) fn parse_remote_url(url: &str) -> Option<RepoRemote> {
    let url = url.trim();
    let (host, path) = if let Some((_, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        (authority, path)
    } else {
        // scp-like: [user@]host:path. A `/` before the `:` means a local path.
        let (authority, path) = url.split_once(':')?;
        if authority.contains('/') {
            return None;
        }
        (authority, path)
    };
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = if host.starts_with('[') {
        host.split_once(']').map_or(host, |(h, _)| &h[1..])
    } else {
        host.split(':').next().unwrap_or(host)
    };
    let path = path
        .split(['?', '#'])
        .next()
        .unwrap_or(path)
        .trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path).trim_matches('/');
    if host.is_empty() || !path.contains('/') || path.split('/').any(str::is_empty) {
        return None;
    }
    Some(RepoRemote {
        host: host.to_lowercase(),
        path: path.to_owned(),
    })
}

/// The repo for a list of `(remote name, url)` settings: the first of
/// `REMOTES` that is set and names a repo.
pub(crate) fn pick_remote(urls: &[(String, String)]) -> Option<RepoRemote> {
    REMOTES.iter().find_map(|wanted| {
        urls.iter()
            .rev()
            .filter(|(name, _)| name == wanted)
            .find_map(|(_, url)| parse_remote_url(url))
    })
}

/// What is known about a folder's repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RepoState {
    /// The answer is being read; ask again soon.
    Resolving,
    Found(RepoRemote),
    /// Not in a git repo, or no remote that names a repo.
    NoRemote,
}

type Resolve = dyn Fn(&Path) -> Option<RepoRemote> + Send + Sync;

struct Entry {
    answer: Option<Option<RepoRemote>>,
    read_at: Instant,
    reading: bool,
}

/// Resolves folders to repos off the caller's thread and remembers the answers.
#[derive(Clone)]
pub(crate) struct RepoResolver {
    entries: Arc<Mutex<HashMap<PathBuf, Entry>>>,
    resolve: Arc<Resolve>,
}

impl std::fmt::Debug for RepoResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RepoResolver")
    }
}

impl Default for RepoResolver {
    fn default() -> Self {
        Self::with_resolver(read_repo)
    }
}

fn read_repo(cwd: &Path) -> Option<RepoRemote> {
    pick_remote(&crate::workspace::git_remote_urls(cwd))
}

impl RepoResolver {
    pub(crate) fn with_resolver(
        resolve: impl Fn(&Path) -> Option<RepoRemote> + Send + Sync + 'static,
    ) -> Self {
        Self {
            entries: Arc::default(),
            resolve: Arc::new(resolve),
        }
    }

    /// The repo of `cwd`, without waiting. The first call for a folder starts
    /// a read and answers `Resolving`; a later call answers what the read
    /// found. An old answer is returned as it is while a new read runs.
    pub(crate) fn lookup(&self, cwd: &Path) -> RepoState {
        let Ok(mut entries) = self.entries.lock() else {
            return RepoState::NoRemote;
        };
        if entries.len() >= MAX_FOLDERS && !entries.contains_key(cwd) {
            entries.retain(|_, entry| entry.reading);
        }
        let entry = entries.entry(cwd.to_path_buf()).or_insert(Entry {
            answer: None,
            read_at: Instant::now(),
            reading: false,
        });
        let due = !entry.reading && (entry.answer.is_none() || entry.read_at.elapsed() >= FRESH);
        let state = match &entry.answer {
            None => RepoState::Resolving,
            Some(Some(repo)) => RepoState::Found(repo.clone()),
            Some(None) => RepoState::NoRemote,
        };
        if due {
            entry.reading = true;
            drop(entries);
            self.start_read(cwd.to_path_buf());
        }
        state
    }

    fn start_read(&self, cwd: PathBuf) {
        let entries = Arc::clone(&self.entries);
        let resolve = Arc::clone(&self.resolve);
        let folder = cwd.clone();
        let spawned = crate::thread_spawn::spawn_named("herdr-repo-remote", move || {
            // A reader that panics must not leave the folder reading forever.
            let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| resolve(&cwd)))
                .unwrap_or_else(|_| {
                    tracing::warn!(folder = %cwd.display(), "the repo remote read panicked");
                    None
                });
            if let Ok(mut entries) = entries.lock() {
                if let Some(entry) = entries.get_mut(&cwd) {
                    entry.answer = Some(found);
                    entry.read_at = Instant::now();
                    entry.reading = false;
                }
            }
        });
        if let Err(error) = spawned {
            tracing::warn!(%error, "could not start the repo remote read");
            if let Ok(mut entries) = self.entries.lock() {
                if let Some(entry) = entries.get_mut(&folder) {
                    entry.answer.get_or_insert(None);
                    entry.read_at = Instant::now();
                    entry.reading = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(host: &str, path: &str) -> RepoRemote {
        RepoRemote {
            host: host.to_owned(),
            path: path.to_owned(),
        }
    }

    #[test]
    fn urls_of_every_shape_name_the_same_repo() {
        let want = Some(remote("gitea.example.net", "AC-forks/herdr-max"));
        for url in [
            "https://gitea.example.net/AC-forks/herdr-max.git",
            "https://user:token@gitea.example.net/AC-forks/herdr-max",
            "https://gitea.example.net:3000/AC-forks/herdr-max/",
            "ssh://git@Gitea.Example.net:2222/AC-forks/herdr-max.git",
            "git@gitea.example.net:AC-forks/herdr-max.git",
            "gitea.example.net:AC-forks/herdr-max",
        ] {
            assert_eq!(parse_remote_url(url), want, "{url}");
        }
    }

    #[test]
    fn urls_that_name_no_repo_are_refused() {
        for url in [
            "",
            "/Users/ac/dev/herdr",
            "../herdr",
            "file:///srv/git/herdr.git",
            "https://gitea.example.net/herdr",
            "https://gitea.example.net/",
            "git@gitea.example.net:herdr.git",
        ] {
            assert_eq!(parse_remote_url(url), None, "{url}");
        }
    }

    #[test]
    fn a_subgroup_path_is_kept_whole() {
        assert_eq!(
            parse_remote_url("git@gitlab.example:group/sub/repo.git"),
            Some(remote("gitlab.example", "group/sub/repo"))
        );
    }

    #[test]
    fn internal_wins_over_origin_and_a_bad_internal_falls_back() {
        let set = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(n, u)| ((*n).to_owned(), (*u).to_owned()))
                .collect()
        };
        let both = set(&[
            ("origin", "git@github.com:colangelo/herdr-max.git"),
            (
                "internal",
                "https://gitea.example.net/AC-forks/herdr-max.git",
            ),
        ]);
        assert_eq!(
            pick_remote(&both),
            Some(remote("gitea.example.net", "AC-forks/herdr-max"))
        );
        let broken = set(&[
            ("origin", "git@github.com:colangelo/herdr-max.git"),
            ("internal", "/srv/git/herdr.git"),
        ]);
        assert_eq!(
            pick_remote(&broken),
            Some(remote("github.com", "colangelo/herdr-max"))
        );
        assert_eq!(pick_remote(&set(&[("fork", "git@x.y:a/b.git")])), None);
    }

    #[test]
    fn the_remote_matches_a_project_ignoring_case_and_a_missing_host() {
        let repo = remote("gitea.example.net", "AC-forks/herdr-max");
        assert!(repo.is("gitea.example.net", "ac-forks/herdr-max"));
        assert!(repo.is("", "AC-forks/herdr-max"));
        assert!(!repo.is("github.com", "AC-forks/herdr-max"));
        assert!(!repo.is("gitea.example.net", "AC-forks/worklink"));
    }

    fn wait_for(resolver: &RepoResolver, cwd: &Path) -> RepoState {
        for _ in 0..200 {
            let state = resolver.lookup(cwd);
            if state != RepoState::Resolving {
                return state;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the read never finished");
    }

    #[test]
    fn a_folder_is_read_once_off_the_caller_and_then_cached() {
        let calls = Arc::new(Mutex::new(0u32));
        let counted = Arc::clone(&calls);
        let resolver = RepoResolver::with_resolver(move |_| {
            *counted.lock().unwrap() += 1;
            std::thread::sleep(Duration::from_millis(30));
            Some(remote("gitea.example.net", "ac/worklink"))
        });
        let cwd = Path::new("/work/worklink");
        // The first answer does not wait for the read.
        assert_eq!(resolver.lookup(cwd), RepoState::Resolving);
        assert_eq!(resolver.lookup(cwd), RepoState::Resolving);
        let found = RepoState::Found(remote("gitea.example.net", "ac/worklink"));
        assert_eq!(wait_for(&resolver, cwd), found);
        assert_eq!(resolver.lookup(cwd), found);
        assert_eq!(*calls.lock().unwrap(), 1);
    }

    #[test]
    fn a_folder_with_no_remote_is_answered_as_such() {
        let resolver = RepoResolver::with_resolver(|_| None);
        let cwd = Path::new("/tmp/not-a-repo");
        assert_eq!(wait_for(&resolver, cwd), RepoState::NoRemote);
    }

    #[test]
    fn a_reader_that_panics_leaves_the_folder_answered_not_reading_forever() {
        let calls = Arc::new(Mutex::new(0u32));
        let counted = Arc::clone(&calls);
        let resolver = RepoResolver::with_resolver(move |_| {
            *counted.lock().unwrap() += 1;
            panic!("the reader broke");
        });
        let cwd = Path::new("/work/broken");
        assert_eq!(wait_for(&resolver, cwd), RepoState::NoRemote);
        assert_eq!(*calls.lock().unwrap(), 1);
        // The entry can be dropped to make room: it is not stuck as reading.
        let entries = resolver.entries.lock().unwrap();
        assert!(!entries[cwd].reading);
    }

    #[test]
    fn a_real_repo_resolves_through_its_config_and_a_worktree_shares_it() {
        let root = std::env::temp_dir().join(format!("herdr-repo-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // No remote in the user's own git config may leak into the answer.
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
        let main = root.join("main");
        std::fs::create_dir_all(main.join(".git/worktrees/wt")).unwrap();
        std::fs::write(main.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            main.join(".git/config"),
            "[remote \"origin\"]\n\turl = git@github.com:colangelo/herdr-max.git\n\
             [remote \"internal\"]\n\turl = https://gitea.example.net/AC-forks/herdr-max.git\n",
        )
        .unwrap();
        let want = Some(remote("gitea.example.net", "AC-forks/herdr-max"));
        assert_eq!(read_repo(&main), want);
        let wt = root.join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        let gitdir = main.join(".git/worktrees/wt");
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
        std::fs::write(gitdir.join("HEAD"), "ref: refs/heads/issue/1\n").unwrap();
        std::fs::write(gitdir.join("commondir"), "../..\n").unwrap();
        assert_eq!(read_repo(&wt), want);
        let _ = std::fs::remove_dir_all(&root);
    }
}
