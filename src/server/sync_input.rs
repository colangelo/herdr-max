//! Private held-key routing for synchronized shell input. Neither these leases
//! nor runtime-instance identities participate in a published codec.

use std::collections::HashMap;

use crate::protocol::{ClientKeyCode, ClientKeyKind, ClientPaneInputEvent};
use crate::terminal::{TerminalId, TerminalRuntimeInputIdentity};

#[derive(Clone, Debug)]
pub(crate) struct SyncInputRecipient {
    pub(crate) terminal_id: TerminalId,
    pub(crate) input_identity: TerminalRuntimeInputIdentity,
    pub(crate) disposition: SyncInputDisposition,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum SyncInputDisposition {
    Typed,
    HostPage { up: bool },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum PressId {
    Physical(u32),
    Semantic(ClientKeyCode),
}

fn press_id(event: &ClientPaneInputEvent) -> Option<PressId> {
    let ClientPaneInputEvent::Key {
        code,
        physical_key_id,
        ..
    } = event
    else {
        return None;
    };
    Some(physical_key_id.map_or_else(|| PressId::Semantic(code.clone()), PressId::Physical))
}

pub(crate) struct HeldSyncInput {
    pub(crate) recipients: Vec<SyncInputRecipient>,
    pub(crate) release: ClientPaneInputEvent,
}

#[derive(Default)]
pub(crate) struct SyncInputLeases {
    held: HashMap<PressId, HeldSyncInput>,
}

impl SyncInputLeases {
    /// Resolve a continuation before looking up the origin's public pane ID:
    /// that ID may have moved or disappeared while its original peers survive.
    pub(crate) fn continuation(
        &mut self,
        event: &ClientPaneInputEvent,
    ) -> Option<(Vec<SyncInputRecipient>, ClientPaneInputEvent)> {
        let id = press_id(event)?;
        match event {
            ClientPaneInputEvent::Key {
                kind: ClientKeyKind::Release,
                ..
            } => {
                let held = self.held.remove(&id)?;
                Some((held.recipients, event.clone()))
            }
            ClientPaneInputEvent::Key {
                kind: ClientKeyKind::Repeat,
                tracks_release: true,
                ..
            } => {
                let held = self.held.get(&id)?;
                Some((held.recipients.clone(), event.clone()))
            }
            _ => None,
        }
    }

    /// A fresh press retires an older tracked lifecycle even when keyboard
    /// negotiation has since switched to untracked legacy events.
    pub(crate) fn retire_before_press(
        &mut self,
        event: &ClientPaneInputEvent,
    ) -> Option<HeldSyncInput> {
        if !matches!(
            event,
            ClientPaneInputEvent::Key {
                kind: ClientKeyKind::Press,
                ..
            }
        ) {
            return None;
        }
        self.held.remove(&press_id(event)?)
    }

    /// The exact press recipients own its eventual repeat/release. Return any
    /// displaced press so its recipients can be released before a fresh press.
    pub(crate) fn press(
        &mut self,
        recipients: Vec<SyncInputRecipient>,
        event: &ClientPaneInputEvent,
    ) -> Option<HeldSyncInput> {
        let ClientPaneInputEvent::Key {
            kind: ClientKeyKind::Press,
            tracks_release: true,
            ..
        } = event
        else {
            return None;
        };
        let id = press_id(event)?;
        let mut release = event.clone();
        if let ClientPaneInputEvent::Key {
            kind,
            repeat_count,
            generated_text,
            ..
        } = &mut release
        {
            *kind = ClientKeyKind::Release;
            *repeat_count = 1;
            *generated_text = None;
        }
        self.held.insert(
            id,
            HeldSyncInput {
                recipients,
                release,
            },
        )
    }

    pub(crate) fn drain(&mut self) -> impl Iterator<Item = HeldSyncInput> + '_ {
        self.held.drain().map(|(_, held)| held)
    }
}
