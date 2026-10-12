use super::*;

impl ClientShellState {
    pub(super) fn cycle_layout(&mut self, outcome: &mut ClientShellInput) {
        use crate::api::schema::{
            AppliedLayoutPreset, LayoutApplyPresetParams, LayoutPreset, LayoutSetPresetParams,
            Method,
        };
        use crate::layout::LayoutPreset as CorePreset;

        let Some(tab_id) = self
            .snapshot
            .as_deref()
            .and_then(|snapshot| snapshot.focused_tab_id.clone())
        else {
            return;
        };
        let main_method = Method::LayoutApplyPreset(LayoutApplyPresetParams {
            tab_id: Some(tab_id.clone()),
            pane_id: None,
            preset: AppliedLayoutPreset::MainVertical,
        });
        // A missing advertisement allows baseline methods, but cannot promise
        // this additive method. Keep the original three-step cycle in that case.
        let advertised = self
            .endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id == self.active_endpoint_id)
            .is_some_and(|endpoint| endpoint.methods.is_some());
        let count = if advertised && self.supports_endpoint_method(&main_method) {
            4
        } else {
            3
        };
        let cycle = &CorePreset::NEXT_CYCLE[..count];
        let (preset, name) = match cycle[self.layout_cycle_index % cycle.len()] {
            CorePreset::EvenHorizontal => (Some(LayoutPreset::EvenHorizontal), "even-horizontal"),
            CorePreset::EvenVertical => (Some(LayoutPreset::EvenVertical), "even-vertical"),
            CorePreset::Tiled => (Some(LayoutPreset::Tiled), "tiled"),
            CorePreset::MainVertical => (None, "main-vertical"),
        };
        let method = match preset {
            Some(preset) => Method::LayoutSetPreset(LayoutSetPresetParams {
                tab_id: Some(tab_id),
                pane_id: None,
                preset,
            }),
            None => main_method,
        };
        if self.push_endpoint_method_with_kind(
            method,
            PendingEndpointKind::LayoutPreset { name },
            outcome,
        ) {
            self.layout_cycle_index = (self.layout_cycle_index + 1) % cycle.len();
        }
    }
}
