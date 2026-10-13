use super::projection::{self, Leaf, ParseError};
use crate::api::schema::{EmptyParams, Method, Request};
use clap::Command;
const LEAVES: &[Leaf] = &[Leaf {
    name: "list",
    about: "List attached clients: foreground, focused tab, input idle time, window focus",
    id: "cli:client:list",
    arguments: Vec::new,
    method: |_| Ok(Method::ClientList(EmptyParams::default())),
}];
pub(crate) fn command() -> Command {
    Command::new("client")
        .about("Inspect the clients attached to the server over the socket API")
        .override_usage("herdr client [COMMAND]")
        .subcommand_required(true)
        .subcommands(LEAVES.iter().map(Leaf::command))
}
pub(crate) fn request(args: &[String]) -> Result<Request, ParseError> {
    projection::group_request(command(), LEAVES, args)
}
