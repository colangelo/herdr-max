use super::*;

/// Internal events for the client event loop.
pub(super) enum ClientLoopEvent {
    /// Host input bytes, one framed chunk. `alt_from_held_escape`: the chunk is a legacy Alt key
    /// the reader made of an ESC held from its previous read (fork issue 331).
    #[cfg(unix)]
    StdinInput {
        data: Vec<u8>,
        alt_from_held_escape: bool,
    },
    #[cfg(unix)]
    PixelMouse(Vec<u8>, crate::input::mouse::HostGeometry),
    #[cfg(unix)]
    DirectGraphicsResponse(direct_graphics::Response),
    #[cfg(windows)]
    StdinEvents(Vec<crate::protocol::ClientInputEvent>),
    #[cfg(windows)]
    NotificationActivated(shell::ClientSystemNotificationTarget),
    /// SIGCONT: this client has the terminal back after a stop.
    #[cfg(unix)]
    TerminalRegained,
    Resize(u16, u16, u32, u32, bool),
    TerminalUnavailable(io::Error),
    ServerMessage {
        endpoint_id: endpoint::ClientEndpointId,
        generation: u64,
        message: Box<ServerMessage>,
    },
    ServerDisconnected {
        endpoint_id: endpoint::ClientEndpointId,
        generation: u64,
    },
    EndpointSupervisor(endpoint::EndpointSupervisorEvent),
    EndpointCatalog(Result<Vec<endpoint::SavedSshEndpoint>, String>),
    /// The core machines this session's primary core lists now (core-phase5
    /// D6, PR D's second read, item 3).
    #[cfg(unix)]
    CoreMachines(Vec<crate::core_machine::command::LinkedMachine>),
    HostPower(crate::platform::HostPowerEvent),
    ActivateEndpoint {
        endpoint_id: endpoint::ClientEndpointId,
        target: Option<shell::ClientEndpointFocusTarget>,
        /// A superseded handoff deliberately starts a fresh target-on epoch even when source and
        /// latest target have the same identity after restoration.
        force: bool,
    },
    Timer,
}
