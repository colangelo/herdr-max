mod history_read;
mod id;
mod runtime;
mod runtime_registry;
pub mod state;
mod title;
pub mod todo;

pub(crate) use history_read::{merge_scrolled_up, snapshot_text, ScreenSnapshot, UpwardMerge};
pub use id::TerminalId;
pub use runtime::TerminalRuntime;
pub(crate) use runtime::TerminalRuntimeInputIdentity;
pub(crate) use runtime_registry::TerminalRuntimeRegistry;
pub use state::{
    AgentHintReport, AgentMetadataReport, EffectivePresentation, EffectiveStateChange,
    TerminalState, TerminalStateMutation,
};
pub(crate) use title::{stripped_terminal_title, title_is_agent_name};

/// When input from a user or caller last reached a pane, in unix ms: the newer
/// of the time carried over a restore and the live runtime's.
pub(crate) fn pane_last_input_at_ms(
    terminal: &TerminalState,
    runtime: Option<&TerminalRuntime>,
) -> Option<i64> {
    newer_input_stamp(
        terminal.restored_last_input_at_ms,
        runtime.and_then(TerminalRuntime::last_input_at_ms),
    )
}

fn newer_input_stamp(restored: Option<i64>, live: Option<i64>) -> Option<i64> {
    restored.max(live)
}

#[cfg(test)]
mod last_input_tests {
    use super::newer_input_stamp;

    #[tokio::test]
    async fn input_stamps_the_pane_and_automatic_writes_do_not() {
        let (runtime, _rx) = super::TerminalRuntime::test_with_channel(80, 24);
        assert_eq!(runtime.last_input_at_ms(), None);

        runtime
            .try_send_bytes_untracked(bytes::Bytes::from_static(b"\x1b[<65;1;1M"))
            .expect("harvest write");
        assert_eq!(runtime.last_input_at_ms(), None, "a read is not input");

        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_millis() as i64;
        runtime
            .try_send_bytes(bytes::Bytes::from_static(b"ls\r"))
            .expect("input write");
        let stamped = runtime.last_input_at_ms().expect("input is stamped");
        assert!(stamped >= before, "{stamped} < {before}");
    }

    #[tokio::test]
    async fn a_queued_submit_is_input_until_it_is_delivered() {
        let (runtime, mut rx) = super::TerminalRuntime::test_with_channel(80, 24);
        let delivered = runtime
            .queue_user_input_submission(
                bytes::Bytes::from_static(b"hello"),
                bytes::Bytes::from_static(b"\r"),
                std::time::Duration::from_millis(200),
                None,
            )
            .expect("queued");
        let input = runtime.input_activity();
        assert!(input.submit_pending(), "a queued submit is input now");
        assert!(
            runtime.last_input_at_ms().is_some(),
            "queueing stamps the pane"
        );

        delivered
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the writer answered")
            .expect("delivered");
        assert!(
            !input.submit_pending(),
            "a delivered submit is no longer pending"
        );
        assert_eq!(rx.recv().await.as_deref(), Some(&b"hello"[..]));
    }

    #[test]
    fn the_newer_stamp_wins_and_unknown_stays_unknown() {
        assert_eq!(newer_input_stamp(None, None), None);
        assert_eq!(newer_input_stamp(Some(5), None), Some(5));
        assert_eq!(newer_input_stamp(None, Some(7)), Some(7));
        assert_eq!(newer_input_stamp(Some(9), Some(7)), Some(9));
        assert_eq!(newer_input_stamp(Some(5), Some(7)), Some(7));
    }
}
