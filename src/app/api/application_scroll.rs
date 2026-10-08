use bytes::Bytes;

use super::responses::{encode_error, encode_success};
use crate::api::schema::{PaneScrollApplicationParams, ResponseResult};
use crate::app::App;

impl App {
    pub(super) fn handle_pane_scroll_application(
        &mut self,
        id: String,
        params: PaneScrollApplicationParams,
    ) -> String {
        if !(1..=64).contains(&params.count) {
            return encode_error(
                id,
                "invalid_params",
                "scroll count must be between 1 and 64",
            );
        }
        let Some((workspace_index, pane_id)) = self.parse_pane_id(&params.pane_id) else {
            return encode_error(id, "pane_not_found", "pane not found");
        };
        let Some(runtime) = self.state.runtime_for_pane_in_workspace(
            &self.terminal_runtimes,
            workspace_index,
            pane_id,
        ) else {
            return encode_error(id, "pane_not_found", "pane not found");
        };
        let Some(bytes) = runtime.encode_application_scroll(params.intent, params.count) else {
            return encode_error(
                id,
                "pane_scroll_failed",
                "application scroll encoding unavailable",
            );
        };
        // Lost alternate screen or unsupported wheel routing is an intentional
        // no-op. Neither can fall back to host scrollback or typed sync input.
        if !bytes.is_empty() {
            if let Err(err) = runtime.try_send_bytes(Bytes::from(bytes)) {
                return encode_error(id, "pane_scroll_failed", err.to_string());
            }
        }
        encode_success(id, ResponseResult::Ok {})
    }
}
