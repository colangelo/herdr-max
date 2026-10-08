use super::*;

impl ClientShellState {
    pub(super) fn cycle_layout(&mut self, outcome: &mut ClientShellInput) {
        let Some(tab_id) = self
            .snapshot
            .as_deref()
            .and_then(|snapshot| snapshot.focused_tab_id.clone())
        else {
            return;
        };
        let cycle = crate::layout::LayoutPreset::NEXT_CYCLE;
        let preset = match cycle[self.layout_cycle_index % cycle.len()] {
            crate::layout::LayoutPreset::EvenHorizontal => {
                crate::api::schema::LayoutPreset::EvenHorizontal
            }
            crate::layout::LayoutPreset::EvenVertical => {
                crate::api::schema::LayoutPreset::EvenVertical
            }
            crate::layout::LayoutPreset::Tiled => crate::api::schema::LayoutPreset::Tiled,
        };
        if self.push_endpoint_method_with_kind(
            crate::api::schema::Method::LayoutSetPreset(
                crate::api::schema::LayoutSetPresetParams {
                    tab_id: Some(tab_id),
                    pane_id: None,
                    preset,
                },
            ),
            PendingEndpointKind::Generic,
            outcome,
        ) {
            self.layout_cycle_index = (self.layout_cycle_index + 1) % cycle.len();
        }
    }
}
