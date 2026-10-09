//! Explicit connection lifecycle.

use serde::{Deserialize, Serialize};

/// Where a link is in its life.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    /// Not yet started, or stopped without intent to reconnect.
    Disconnected,
    /// TCP / WebSocket handshake in flight.
    Connecting,
    /// Socket up; exchanging `hello` / `hello_ack`.
    Authenticating,
    /// Ready for frames.
    Connected,
    /// Lost the socket; backing off before `Connecting` again.
    Reconnecting,
    /// Terminal: closed on purpose or gave up.
    Closed,
}

impl ConnectionState {
    /// Terminal state.
    pub fn is_closed(self) -> bool {
        matches!(self, Self::Closed)
    }

    /// Frames may be sent.
    pub fn is_connected(self) -> bool {
        matches!(self, Self::Connected)
    }
}

/// One transition, for logs and UI.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StateChange {
    /// Previous state.
    pub from: ConnectionState,
    /// New state.
    pub to: ConnectionState,
}

/// The legal transitions, enforced in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionMachine {
    state: ConnectionState,
    attempts: u32,
}

impl Default for ConnectionMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectionMachine {
    /// Fresh machine in `Disconnected`.
    pub fn new() -> Self {
        Self { state: ConnectionState::Disconnected, attempts: 0 }
    }

    /// Current state.
    pub fn state(&self) -> ConnectionState {
        self.state
    }

    /// Consecutive failed attempts since the last `Connected`.
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    fn go(&mut self, to: ConnectionState) -> Option<StateChange> {
        if self.state == to {
            return None;
        }
        let from = self.state;
        self.state = to;
        Some(StateChange { from, to })
    }

    /// Start (or retry) connecting. Illegal from `Closed`.
    pub fn connecting(&mut self) -> Option<StateChange> {
        if self.state.is_closed() {
            return None;
        }
        self.go(ConnectionState::Connecting)
    }

    /// Socket established; hello in flight.
    pub fn authenticating(&mut self) -> Option<StateChange> {
        if self.state != ConnectionState::Connecting {
            return None;
        }
        self.go(ConnectionState::Authenticating)
    }

    /// Hello acknowledged.
    pub fn connected(&mut self) -> Option<StateChange> {
        if self.state != ConnectionState::Authenticating {
            return None;
        }
        self.attempts = 0;
        self.go(ConnectionState::Connected)
    }

    /// Socket lost or connect failed; will retry. Returns the new attempt count alongside.
    pub fn reconnecting(&mut self) -> Option<StateChange> {
        if self.state.is_closed() || self.state == ConnectionState::Disconnected {
            return None;
        }
        self.attempts = self.attempts.saturating_add(1);
        self.go(ConnectionState::Reconnecting)
    }

    /// Stop for good.
    pub fn closed(&mut self) -> Option<StateChange> {
        self.go(ConnectionState::Closed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_and_reconnect_counting() {
        let mut m = ConnectionMachine::new();
        assert_eq!(m.state(), ConnectionState::Disconnected);
        assert_eq!(m.connecting().unwrap(), StateChange { from: ConnectionState::Disconnected, to: ConnectionState::Connecting });
        assert!(m.connecting().is_none(), "same state is not a change");
        assert!(m.connected().is_none(), "cannot skip authenticating");
        m.authenticating().unwrap();
        m.connected().unwrap();
        assert!(m.state().is_connected());
        assert_eq!(m.attempts(), 0);
        m.reconnecting().unwrap();
        assert_eq!(m.attempts(), 1);
        m.connecting().unwrap();
        m.reconnecting().unwrap();
        assert_eq!(m.attempts(), 2);
        m.connecting().unwrap();
        m.authenticating().unwrap();
        m.connected().unwrap();
        assert_eq!(m.attempts(), 0, "success resets the counter");
    }

    #[test]
    fn closed_is_terminal_and_disconnected_cannot_reconnect() {
        let mut m = ConnectionMachine::new();
        assert!(m.reconnecting().is_none(), "nothing to reconnect from Disconnected");
        m.closed().unwrap();
        assert!(m.state().is_closed());
        assert!(m.connecting().is_none());
        assert!(m.reconnecting().is_none());
        assert!(m.authenticating().is_none());
        assert!(m.closed().is_none());
        assert_eq!(ConnectionMachine::default(), ConnectionMachine::new());
        assert_eq!(serde_json::to_string(&ConnectionState::Reconnecting).unwrap(), r#""reconnecting""#);
    }
}
