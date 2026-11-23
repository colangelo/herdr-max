//! The view's one allowlist of client methods (core-phase5 D6, security
//! review M2). A view shows the far herdr and forwards a person's input to
//! it; it changes nothing on the far Mac or on this one beyond that. The
//! same table decides what the endpoint welcome advertises and what the
//! server's dispatch takes, so a client never sees a method the view then
//! refuses, and a client that sends one anyway is refused before dispatch.
//!
//! Every client method is classified here by name. A method added to the
//! schema later is unclassified, so it is refused in a view until someone
//! classifies it, and `every_client_method_is_classified` fails until then.

/// What a view does with one client method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewMethod {
    /// Shows the far herdr, changes only this view's own presentation of it
    /// (focus, zoom, scroll, copy, seen marks, a detach), or forwards a
    /// person's input.
    Allowed,
    /// Changes the far herdr's layout, names or programs, which the far
    /// herdr owns: the view would diverge from it, or start a program here.
    FarLayout,
    /// Runs, writes or reads something on this Mac (a command, git, a file,
    /// a helper, this Mac's own task or integration state).
    Local,
}

/// The class of the client method `name`, `None` when it has none yet.
pub(crate) fn classify(name: &str) -> Option<ViewMethod> {
    use ViewMethod::{Allowed, FarLayout, Local};
    Some(match name {
        // The view's own presentation and the person's input.
        "client_shell.surface.set"
        | "server.reload_config"
        | "notification.list"
        | "notification.mark_seen"
        | "notification.clear"
        | "workspace.focus"
        | "tab.focus"
        | "pane.focus"
        | "pane.focus_direction"
        | "pane.zoom"
        | "pane.scroll"
        | "pane.scroll_application"
        | "pane.clear"
        | "pane.clear_scrollback"
        | "pane.selection.read"
        | "pane.copy_motion"
        | "pane.copy_search"
        | "pane.input.set"
        | "pane.mark_seen"
        // A close in a view is a detach (D6): the far program and pane stay,
        // and the next far document shows them again.
        | "pane.close"
        | "tab.close"
        | "workspace.close" => Allowed,
        // The far herdr's layout, names and programs.
        "workspace.create"
        | "workspace.pin"
        | "workspace.unpin"
        | "workspace.rename"
        | "workspace.move"
        | "workspace.move_block"
        | "tab.create"
        | "tab.rename"
        | "tab.sync"
        | "tab.move"
        | "agent.pin"
        | "agent.unpin"
        | "pane.split"
        | "pane.swap"
        | "pane.move"
        | "pane.sync"
        | "pane.sync_pair"
        | "pane.resize"
        | "pane.rename"
        | "pane.respawn"
        | "layout.set_split_ratio"
        | "layout.balance"
        | "layout.set_preset"
        | "layout.apply_preset" => FarLayout,
        // This Mac's commands, files, helpers and state.
        "command.invoke"
        | "product_announcement.dismiss"
        | "release_notes.dismiss"
        // Typed text and Enter queued into a far pane, or the local
        // `note_command` with a recipient a far title can choose (D-05).
        | "agent.message"
        // Writes the scrollback to a file here and opens an editor.
        | "pane.edit_scrollback"
        // A link's URL opened, or its path resolved against a far folder,
        // on this Mac (M1).
        | "pane.link.activate"
        | "pane.link.resolve"
        // Local git at a far folder (M1).
        | "worktree.list"
        | "worktree.create"
        | "worktree.open"
        | "worktree.remove"
        | "integration.list"
        | "integration.install"
        | "todo.list"
        | "todo.add"
        | "todo.update"
        | "todo.remove"
        | "todo.clear"
        | "task.list"
        | "task.show"
        | "task.add"
        | "task.update"
        | "task.done"
        | "task.drop"
        | "task.reopen"
        | "task.bind"
        | "task.conflicts"
        | "task.resolve_conflict"
        | "issue.list"
        | "ask.list" => Local,
        _ => return None,
    })
}

/// Whether a view takes the client method `name`; an unclassified one is
/// refused.
pub(crate) fn allows(name: &str) -> bool {
    classify(name) == Some(ViewMethod::Allowed)
}

/// Why a view refuses the client method `name`, `None` when it takes it.
pub(crate) fn refusal(machine: &crate::core_machine::MachineName, name: &str) -> Option<String> {
    let why = match classify(name)? {
        ViewMethod::Allowed => return None,
        ViewMethod::FarLayout => format!("{machine}'s herdr owns its layout; do this there"),
        ViewMethod::Local => format!("it would run or write on this Mac, not on {machine}"),
    };
    Some(format!(
        "this is a view of {machine}: {name} is refused ({why})"
    ))
}

/// The refusal of a method no one has classified yet.
pub(crate) fn unclassified_refusal(
    machine: &crate::core_machine::MachineName,
    name: &str,
) -> String {
    format!("this is a view of {machine}: {name} is not one a view takes")
}

/// The client methods a server advertises in its endpoint welcome: every
/// client-shell method for a primary, the allowed ones for a view.
pub(crate) fn advertised(all: &[&'static str], view: bool) -> Vec<String> {
    all.iter()
        .filter(|name| !view || allows(name))
        .map(|name| (*name).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> &'static [&'static str] {
        crate::api::schema::operations::client_shell_method_names()
    }

    // M2: every client method has a decision. A method added to the schema
    // fails here until it is classified (and is refused in a view until then).
    #[test]
    fn every_client_method_is_classified() {
        let unclassified: Vec<&str> = all()
            .iter()
            .copied()
            .filter(|name| classify(name).is_none())
            .collect();
        assert!(
            unclassified.is_empty(),
            "classify these for a view in src/view/policy.rs: {unclassified:?}"
        );
    }

    // M2: the view's allowlist, pinned: what a view takes, and the review's
    // named refusals among the rest.
    #[test]
    fn a_view_takes_only_its_presentation_and_input() {
        let allowed: Vec<&str> = all().iter().copied().filter(|name| allows(name)).collect();
        assert_eq!(
            allowed,
            [
                "client_shell.surface.set",
                "notification.clear",
                "notification.list",
                "notification.mark_seen",
                "pane.clear",
                "pane.clear_scrollback",
                "pane.close",
                "pane.copy_motion",
                "pane.copy_search",
                "pane.focus",
                "pane.focus_direction",
                "pane.input.set",
                "pane.mark_seen",
                "pane.scroll",
                "pane.scroll_application",
                "pane.selection.read",
                "pane.zoom",
                "server.reload_config",
                "tab.close",
                "tab.focus",
                "workspace.close",
                "workspace.focus",
            ]
        );
        for refused in [
            "agent.message",
            "integration.install",
            "pane.edit_scrollback",
            "command.invoke",
            "worktree.list",
            "worktree.open",
            "worktree.create",
            "worktree.remove",
            "pane.split",
            "release_notes.dismiss",
        ] {
            assert!(!allows(refused), "{refused}");
        }
        assert!(!allows("a.method.nobody.classified"));
    }

    // M2: the welcome advertises the same list the dispatch takes.
    #[test]
    fn a_views_welcome_advertises_only_what_it_takes() {
        let primary = advertised(all(), false);
        assert_eq!(primary.len(), all().len());
        let view = advertised(all(), true);
        assert!(view.iter().all(|name| allows(name)), "{view:?}");
        assert_eq!(view.len(), all().iter().filter(|name| allows(name)).count());
        assert!(!view.iter().any(|name| name == "agent.message"));
        let machine = crate::core_machine::MachineName::parse("loop").unwrap();
        for name in all() {
            assert_eq!(
                refusal(&machine, name).is_none(),
                view.iter().any(|m| m == name),
                "{name}"
            );
        }
    }
}
