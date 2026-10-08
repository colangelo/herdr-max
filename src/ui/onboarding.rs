use ratatui::layout::Rect;

pub(crate) const ONBOARDING_TITLE: &str = "  herdr";
pub(crate) const ONBOARDING_SUBTITLE: &str = "  terminal workspace manager for coding agents";
pub(crate) const ONBOARDING_DESCRIPTION: [&str; 3] = [
    "  this is a mouse-first terminal.",
    "  click the sidebar to switch workspaces, drag pane",
    "  borders to resize, right-click for context menus.",
];
pub(crate) const ONBOARDING_PREFIX_SUFFIX: &str = " enters prefix mode · ";
pub(crate) const ONBOARDING_HELP_LABEL: &str = "?";
pub(crate) const ONBOARDING_HELP_SUFFIX: &str = " shows keybinds and settings";
pub(crate) const ONBOARDING_NEXT: &str =
    "  next: install optional agent integrations for more reliable state";

pub(crate) fn onboarding_welcome_continue_rect(area: Rect) -> Rect {
    super::widgets::continue_button_rect(area)
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    #[test]
    fn snapshot_onboarding() {
        crate::ui::test_support::overlay_snapshot_of(|app| {
            app.mode = crate::app::state::Mode::Onboarding;
        })
        .assert(
            Rect::new(8, 4, 64, 16),
            &[
                "┌──────────────────────────────────────────────────────────────┐",
                "│  herdr                                                       │",
                "│  terminal workspace manager for coding agents                │",
                "│                                                              │",
                "│  this is a mouse-first terminal.                             │",
                "│  click the sidebar to switch workspaces, drag pane           │",
                "│  borders to resize, right-click for context menus.           │",
                "│                                                              │",
                "│  ctrl+b enters prefix mode · ? shows keybinds and settings   │",
                "│  next: install optional agent integrations for more reliable │",
                "│                                                              │",
                "│                                                              │",
                "│                                                              │",
                "│                                                              │",
                "│ ↵ continue                                                   │",
                "└──────────────────────────────────────────────────────────────┘",
            ],
        );
    }
}
