//! The address core gives a terminal (core-phase5 D4, F1).
//!
//! Core's `terminal.list` merges the terminals of linked machines into the
//! local list, with the far herdr's own labels and pids, and marks them only
//! by an address prefix: `t7` is on this Mac, `ac-mbm5/t7` on a machine. Every
//! address herdr reads from core is parsed here, once, so each scan states
//! which host it wants instead of testing a string.

use serde_json::Value;

use crate::core_machine::MachineName;

/// Where a terminal lives and what this core calls it there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CoreAddress {
    pub machine: Option<MachineName>,
    pub terminal: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AddressError(String);

impl std::fmt::Display for AddressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl CoreAddress {
    /// `t7` is local, `m/t7` is terminal `t7` on machine `m`. An empty part
    /// (`m/`, `/t7`, the empty string) or a second `/` (`m/n/t7`) is refused.
    pub(crate) fn parse(raw: &str) -> Result<Self, AddressError> {
        let refuse = |why: &str| AddressError(format!("core address {raw:?} {why}"));
        let part_ok = |part: &str| !part.is_empty() && !part.chars().any(char::is_whitespace);
        match raw.split('/').collect::<Vec<_>>().as_slice() {
            [terminal] if part_ok(terminal) => Ok(Self {
                machine: None,
                terminal: (*terminal).to_string(),
            }),
            [machine, terminal] if part_ok(machine) && part_ok(terminal) => Ok(Self {
                machine: Some(MachineName::parse(machine).map_err(|err| refuse(&err.0))?),
                terminal: (*terminal).to_string(),
            }),
            [_, _, _, ..] => Err(refuse("has more than one machine part")),
            _ => Err(refuse("has an empty or blank part")),
        }
    }

    pub(crate) fn is_local(&self) -> bool {
        self.machine.is_none()
    }
}

/// The address of a `terminal.list` or `terminal.get` record when it is a
/// terminal of this core's own host. A record of a machine, or one whose
/// address does not parse, is not ours: its labels and its pid describe
/// another Mac's process, so no local scan may adopt, kill or sweep it.
pub(crate) fn local_address(record: &Value) -> Option<&str> {
    let raw = record["address"].as_str()?;
    match CoreAddress::parse(raw) {
        Ok(address) if address.is_local() => Some(raw),
        Ok(_) => None,
        Err(err) => {
            tracing::debug!(%err, "ignoring a terminal with an unreadable address");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // V26 part 1 (core-phase5 F1).
    #[test]
    fn local_and_machine_addresses_parse() {
        let local = CoreAddress::parse("t7").expect("local");
        assert!(local.is_local());
        assert_eq!(local.terminal, "t7");
        let far = CoreAddress::parse("loop/t7").expect("machine");
        assert!(!far.is_local());
        assert_eq!(far.machine, Some(MachineName::parse("loop").expect("name")));
        assert_eq!(far.terminal, "t7");
    }

    #[test]
    fn malformed_addresses_are_refused() {
        for raw in [
            "", "m/", "/t7", "/", "m/n/t7", "m//t7", "m/n/", "a b", "m/t 7", "-m/t7", "m;x/t7",
        ] {
            assert!(CoreAddress::parse(raw).is_err(), "{raw:?} was accepted");
        }
    }

    #[test]
    fn only_a_local_record_has_a_local_address() {
        let record = |address: Value| serde_json::json!({"address": address});
        assert_eq!(local_address(&record("t1".into())), Some("t1"));
        assert_eq!(local_address(&record("loop/t1".into())), None);
        assert_eq!(local_address(&record("loop/".into())), None);
        assert_eq!(local_address(&record(Value::Null)), None);
        assert_eq!(local_address(&serde_json::json!({})), None);
    }
}
