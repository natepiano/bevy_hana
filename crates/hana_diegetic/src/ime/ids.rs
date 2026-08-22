//! Stable identifiers for IME sessions and panel elements.

use std::fmt::Display;
use std::fmt::Formatter;

/// Stable id for one active or recently completed IME session.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ImeSessionId(u64);

impl ImeSessionId {
    /// Creates a session id from a raw value.
    #[must_use]
    pub const fn new(value: u64) -> Self { Self(value) }

    /// Returns the raw id value.
    #[must_use]
    pub const fn value(self) -> u64 { self.0 }
}

impl Display for ImeSessionId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Stable id for one commit attempt inside an IME session.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ImeCommitAttemptId(u64);

impl ImeCommitAttemptId {
    /// Creates a commit attempt id from a raw value.
    #[must_use]
    pub const fn new(value: u64) -> Self { Self(value) }

    /// Returns the raw id value.
    #[must_use]
    pub const fn value(self) -> u64 { self.0 }
}

impl Display for ImeCommitAttemptId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Optional app/model revision reported after an applied value changes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ImeValueRevision(u64);

impl ImeValueRevision {
    /// Creates a value revision from a raw value.
    #[must_use]
    pub const fn new(value: u64) -> Self { Self(value) }

    /// Returns the raw revision value.
    #[must_use]
    pub const fn value(self) -> u64 { self.0 }
}

/// Opaque, unforgeable identity for a text element that carries no
/// author-assigned name.
///
/// Minted only by the layout builder's per-build order counter (see
/// [`PanelElementId::auto`]); the inner value is private, so code outside this
/// crate cannot construct one. That makes an [`PanelElementId::Auto`] id incapable
/// of colliding with an author's [`PanelElementId::Named`] id by construction.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AutoElementId(u32);

impl AutoElementId {
    /// Returns the raw counter value (debug/diagnostics only).
    #[must_use]
    pub const fn value(self) -> u32 { self.0 }
}

/// Semantic element identity local to a diegetic panel or app-owned surface.
///
/// Either an author-assigned [`Named`](Self::Named) id — the only variant a
/// caller can build, via [`PanelElementId::named`] or the `From<&str>` /
/// `From<String>` conversions — or an [`Auto`](Self::Auto) id the layout builder
/// mints for an unnamed text element. Because every public constructor yields
/// `Named`, no string can forge an `Auto`, so the two id families share one
/// panel-local namespace without ever colliding.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum PanelElementId {
    /// Author-assigned, publicly addressable name.
    Named(String),
    /// Builder-minted positional id for an unnamed text run; not publicly
    /// addressable.
    Auto(AutoElementId),
}

impl PanelElementId {
    /// Creates a named panel-local element id.
    #[must_use]
    pub fn named(value: impl Into<String>) -> Self { Self::Named(value.into()) }

    /// Mints a builder-order auto id for an unnamed text run.
    ///
    /// Crate-internal: the only path that produces an [`Auto`](Self::Auto)
    /// variant, keeping it unforgeable from outside.
    #[must_use]
    pub(crate) const fn auto(value: u32) -> Self { Self::Auto(AutoElementId(value)) }

    /// Returns the author-assigned name, or `None` for an auto id.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Named(name) => Some(name),
            Self::Auto(_) => None,
        }
    }

    /// Returns `true` when this id is an author-assigned, publicly addressable
    /// [`Named`](Self::Named) id.
    #[must_use]
    pub const fn is_named(&self) -> bool { matches!(self, Self::Named(_)) }
}

impl Display for PanelElementId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Named(name) => formatter.write_str(name),
            Self::Auto(auto) => write!(formatter, "#auto-{}", auto.value()),
        }
    }
}

impl From<&str> for PanelElementId {
    fn from(value: &str) -> Self { Self::Named(value.to_owned()) }
}

impl From<String> for PanelElementId {
    fn from(value: String) -> Self { Self::Named(value) }
}
