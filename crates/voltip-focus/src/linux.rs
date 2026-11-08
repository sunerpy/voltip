//! The insert check's reader on Linux, X11 and Wayland alike (docs/dictation.md §28): AT-SPI over
//! D-Bus, with zbus's async API driven by `zbus::block_on` on the reader's worker thread: each
//! reading is one `block_on` call, which also runs the connection's I/O on that thread, and zbus's
//! helper thread keeps the connection served in between. The session bus names the accessibility bus
//! (`org.a11y.Bus.GetAddress`); when accessibility is off (`org.a11y.Status.IsEnabled`), nothing is
//! read — Voltip never switches it on, which would turn on accessibility in every application. The
//! focused control is the `STATE_FOCUSED` object under the application window that is
//! `STATE_ACTIVE` (`Collection.GetMatches`, or a bounded walk where the toolkit has no Collection,
//! GTK 4); its identity is its bus name and object path, and its text comes from the `Text`
//! interface. Chromium and Electron expose AT-SPI only when accessibility is on, so they mostly
//! read as unconfirmed. Every call is bounded by the connection's method timeout.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use voltip_platform::insert_check::rules::{FocusText, MAX_SELECTED_CHARS, READ_BEFORE_CARET, UnconfirmedWhy};
use zbus::{Connection, OwnedObjectPath, OwnedVariant};

use crate::FocusPlatform;

/// How long one D-Bus call may take.
pub const CALL_TIMEOUT: Duration = Duration::from_millis(300);
/// The walk for a toolkit without `Collection`: at most this many objects, for at most this long.
pub const WALK_NODES: usize = 3000;
/// See [`WALK_NODES`].
pub const WALK_WITHIN: Duration = Duration::from_millis(150);

const REGISTRY: &str = "org.a11y.atspi.Registry";
const ROOT: &str = "/org/a11y/atspi/accessible/root";
const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const COLLECTION: &str = "org.a11y.atspi.Collection";
const TEXT: &str = "org.a11y.atspi.Text";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// `AtspiStateType`.
const STATE_ACTIVE: u32 = 1;
const STATE_FOCUSED: u32 = 12;
/// `AtspiRole::ATSPI_ROLE_PASSWORD_TEXT` and `ATSPI_ROLE_TERMINAL`.
const ROLE_PASSWORD_TEXT: u32 = 40;
const ROLE_TERMINAL: u32 = 60;
/// `AtspiCollectionMatchType::ATSPI_Collection_MATCH_ALL` and the canonical sort order.
const MATCH_ALL: i32 = 1;
const SORT_CANONICAL: u32 = 1;

/// An AT-SPI object: the application's bus name and the object's path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Node {
    bus: String,
    path: OwnedObjectPath,
}

/// The focused control and the active window it was found under.
#[derive(Clone, Debug)]
pub struct Focused {
    node: Node,
    window: Node,
}

/// The reader's connection to the accessibility bus.
pub struct AtSpi {
    bus: Connection,
    /// The last control found focused: the next reading checks it first (two calls) before it
    /// searches the desktop again.
    last: Option<Focused>,
}

/// Open the accessibility bus, unless accessibility is off.
pub fn open() -> Result<AtSpi, String> {
    zbus::block_on(async {
        let session = zbus::connection::Builder::session().method_timeout(CALL_TIMEOUT).build().await.map_err(|e| format!("session bus: {e}"))?;
        let enabled: OwnedVariant = session
            .call_method(Some("org.a11y.Bus"), "/org/a11y/bus", Some(PROPERTIES), "Get", &("org.a11y.Status", "IsEnabled"))
            .await
            .and_then(|reply| reply.body().deserialize::<OwnedVariant>())
            .map_err(|e| format!("org.a11y.Status: {e}"))?;
        if !bool::try_from(enabled).unwrap_or(false) {
            return Err("accessibility is off (org.a11y.Status.IsEnabled)".into());
        }
        let address: String = session
            .call_method(Some("org.a11y.Bus"), "/org/a11y/bus", Some("org.a11y.Bus"), "GetAddress", &())
            .await
            .and_then(|reply| reply.body().deserialize::<String>())
            .map_err(|e| format!("org.a11y.Bus.GetAddress: {e}"))?;
        let bus =
            zbus::connection::Builder::address(address.as_str()).method_timeout(CALL_TIMEOUT).build().await.map_err(|e| format!("accessibility bus: {e}"))?;
        Ok(AtSpi { bus, last: None })
    })
}

/// `states` (`GetState`: the state set as 32-bit words) holds `state`.
fn has_state(states: &[u32], state: u32) -> bool {
    let (word, bit) = ((state / 32) as usize, state % 32);
    states.get(word).is_some_and(|w| w & (1 << bit) != 0)
}

impl AtSpi {
    async fn call<B, R>(&self, node: &Node, interface: &str, method: &str, body: &B) -> zbus::Result<R>
    where
        B: serde::Serialize + zbus::DynamicType,
        R: for<'d> serde::Deserialize<'d> + zbus::Type,
    {
        let reply = self.bus.call_method(Some(node.bus.as_str()), &node.path, Some(interface), method, body).await?;
        reply.body().deserialize::<R>()
    }

    async fn property<R>(&self, node: &Node, interface: &str, name: &str) -> zbus::Result<R>
    where
        R: TryFrom<OwnedVariant>,
    {
        let value: OwnedVariant = self.call(node, PROPERTIES, "Get", &(interface, name)).await?;
        R::try_from(value).map_err(|_| zbus::Error::Failure(format!("{interface}.{name}: unexpected type")))
    }

    async fn states(&self, node: &Node) -> zbus::Result<Vec<u32>> {
        self.call(node, ACCESSIBLE, "GetState", &()).await
    }

    async fn children(&self, node: &Node) -> zbus::Result<Vec<Node>> {
        let children: Vec<(String, OwnedObjectPath)> = self.call(node, ACCESSIBLE, "GetChildren", &()).await?;
        Ok(children.into_iter().map(|(bus, path)| Node { bus, path }).collect())
    }

    /// The application window that is active: the registry's applications, then their windows.
    async fn active_window(&self) -> Option<Node> {
        let root = Node { bus: REGISTRY.to_owned(), path: OwnedObjectPath::try_from(ROOT).ok()? };
        for app in self.children(&root).await.ok()? {
            let Ok(windows) = self.children(&app).await else { continue };
            for window in windows {
                if self.states(&window).await.is_ok_and(|s| has_state(&s, STATE_ACTIVE)) {
                    return Some(window);
                }
            }
        }
        None
    }

    /// The `STATE_FOCUSED` object under `window`: one `Collection.GetMatches`, or a walk.
    async fn focused_under(&self, window: &Node) -> Option<Node> {
        let states = vec![(1i32 << STATE_FOCUSED), 0i32];
        let rule = (states, MATCH_ALL, HashMap::<String, String>::new(), MATCH_ALL, Vec::<i32>::new(), MATCH_ALL, Vec::<String>::new(), MATCH_ALL, false);
        let found: zbus::Result<Vec<(String, OwnedObjectPath)>> = self.call(window, COLLECTION, "GetMatches", &(rule, SORT_CANONICAL, 1i32, true)).await;
        match found {
            Ok(found) => found.into_iter().next().map(|(bus, path)| Node { bus, path }),
            Err(_) => self.walk(window).await,
        }
    }

    /// Breadth first under `window` for the focused object, within [`WALK_NODES`] and
    /// [`WALK_WITHIN`].
    async fn walk(&self, window: &Node) -> Option<Node> {
        let started = Instant::now();
        let mut queue = VecDeque::from([window.clone()]);
        let mut seen = 0;
        while let Some(node) = queue.pop_front() {
            seen += 1;
            if seen > WALK_NODES || started.elapsed() > WALK_WITHIN {
                return None;
            }
            if node != *window && self.states(&node).await.is_ok_and(|s| has_state(&s, STATE_FOCUSED)) {
                return Some(node);
            }
            if let Ok(children) = self.children(&node).await {
                queue.extend(children);
            }
        }
        None
    }

    /// The last control found, if it is still the focused object of the window that is still
    /// active.
    async fn still_focused(&self, last: &Focused) -> bool {
        self.states(&last.window).await.is_ok_and(|s| has_state(&s, STATE_ACTIVE)) && self.states(&last.node).await.is_ok_and(|s| has_state(&s, STATE_FOCUSED))
    }

    /// [`FocusPlatform::focused`]: the last control while it keeps the focus, else a search.
    async fn find_focused(&mut self) -> Result<Node, UnconfirmedWhy> {
        let last = match self.last.clone() {
            Some(last) if self.still_focused(&last).await => Some(last),
            _ => None,
        };
        let found = match last {
            Some(last) => last,
            None => {
                let window = self.active_window().await.ok_or(UnconfirmedWhy::Unreadable)?;
                let node = self.focused_under(&window).await.ok_or(UnconfirmedWhy::Unreadable)?;
                Focused { node, window }
            }
        };
        self.last = Some(found.clone());
        let role: u32 = self.call(&found.node, ACCESSIBLE, "GetRole", &()).await.map_err(|_| UnconfirmedWhy::Unreadable)?;
        if skipped(role) {
            return Err(UnconfirmedWhy::Skipped);
        }
        Ok(found.node)
    }

    /// [`FocusPlatform::text`].
    async fn read_text(&self, node: &Node) -> Option<FocusText> {
        let chars: i32 = self.property(node, TEXT, "CharacterCount").await.ok()?;
        let caret: i32 = self.property(node, TEXT, "CaretOffset").await.ok()?;
        let (chars, caret) = (usize::try_from(chars).ok()?, usize::try_from(caret).ok()?.min(usize::try_from(chars).ok()?));
        let selections: i32 = self.call(node, TEXT, "GetNSelections", &()).await.ok()?;
        let selection = if selections > 0 {
            let (start, end): (i32, i32) = self.call(node, TEXT, "GetSelection", &(0i32,)).await.ok()?;
            (usize::try_from(start).ok()?, usize::try_from(end).ok()?)
        } else {
            (caret, caret)
        };
        let (from, to) = (selection.0.min(selection.1), selection.0.max(selection.1));
        let selected = if to > from && to - from <= MAX_SELECTED_CHARS {
            self.call::<_, String>(node, TEXT, "GetText", &(i32::try_from(from).ok()?, i32::try_from(to).ok()?)).await.ok()
        } else {
            None
        };
        let start = caret.saturating_sub(READ_BEFORE_CARET);
        let before: String = self.call(node, TEXT, "GetText", &(i32::try_from(start).ok()?, i32::try_from(caret).ok()?)).await.ok()?;
        Some(FocusText::read(Some(chars), Some(caret), Some(selection), selected.as_deref(), Some(&before)))
    }
}

impl FocusPlatform for AtSpi {
    type Control = Node;

    fn focused(&mut self) -> Result<Node, UnconfirmedWhy> {
        zbus::block_on(self.find_focused())
    }

    fn same(&mut self, a: &Node, b: &Node) -> bool {
        a == b
    }

    fn text(&mut self, node: &Node) -> Option<FocusText> {
        zbus::block_on(self.read_text(node))
    }
}

/// Controls whose text is not read: a password field, and a terminal wherever it is embedded (an
/// IDE's panel as well as a terminal application): what a paste does there is the program's
/// business, and a prompt that does not echo shows nothing (docs/dictation.md §28.3).
fn skipped(role: u32) -> bool {
    matches!(role, ROLE_PASSWORD_TEXT | ROLE_TERMINAL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_field_and_a_terminal_are_not_read() {
        assert!(skipped(ROLE_PASSWORD_TEXT));
        assert!(skipped(ROLE_TERMINAL));
        assert!(!skipped(61), "ATSPI_ROLE_TEXT");
        assert!(!skipped(79), "ATSPI_ROLE_ENTRY");
    }

    #[test]
    fn the_state_set_is_read_word_by_word() {
        assert!(has_state(&[1 << STATE_ACTIVE, 0], STATE_ACTIVE));
        assert!(has_state(&[1 << STATE_FOCUSED, 0], STATE_FOCUSED));
        assert!(!has_state(&[1 << STATE_ACTIVE, 0], STATE_FOCUSED));
        assert!(has_state(&[0, 1 << 3], 35), "the second word");
        assert!(!has_state(&[], STATE_ACTIVE), "an empty set");
    }
}
