//! Prefix-indexed matching for completed keystroke sequences.

use std::collections::HashMap;
use std::time::Duration;
use std::time::Instant;

use super::Keystroke;
use super::KeystrokeSequence;

/// Matches completed [`Keystroke`] values against a prefix index.
///
/// Construction creates the prefix index. [`SequenceMatcher::match_keystroke`] only follows the
/// current prefix and never parses text or allocates.
pub struct SequenceMatcher<T: Copy> {
    prefix_index: Vec<PrefixNode<T>>,
    pending:      Option<PendingSequence<T>>,
}

/// Result of matching one completed [`Keystroke`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatchOutcome<T> {
    /// A complete sequence matched with no longer candidate remaining.
    Matched(T),
    /// A complete sequence matched, but a longer candidate may still match.
    Deferred(T),
    /// A prefix matched, but no complete sequence has matched yet.
    Pending,
    /// A pending prefix did not match this keystroke.
    ///
    /// Resolve `deferred_match`, then pass `keystroke` to
    /// [`SequenceMatcher::match_keystroke`] once more from the root.
    Reprocess {
        /// The completed prefix, if any, that must fire before reprocessing.
        deferred_match: DeferredMatch<T>,
        /// The completed keystroke to match again from the root.
        keystroke:      Keystroke,
    },
    /// No prefix matched and no pending sequence existed.
    NoMatch,
}

/// Whether a completed sequence is waiting to fire ahead of a reprocessed keystroke.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeferredMatch<T> {
    /// The completed sequence's value, which fires before the keystroke is matched again.
    Fire(T),
    /// The prefix that was pending never completed a sequence, so nothing fires ahead of the
    /// keystroke.
    NothingToFire,
}

/// What a pending prefix was doing when [`SequenceMatcher::resolve_timeout`] examined it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeoutOutcome<T> {
    /// A completed sequence timed out; its deferred value is now the caller's to fire.
    Resolved(T),
    /// A partial prefix timed out with no completed sequence to fire, and was discarded.
    DiscardedPartialPrefix,
    /// A prefix is pending and its timeout has not elapsed yet.
    AwaitingKeystroke,
    /// No prefix is pending.
    NoPendingSequence,
}

struct PrefixNode<T> {
    children: HashMap<Keystroke, PrefixNodeId>,
    value:    Option<T>,
}

impl<T> PrefixNode<T> {
    fn empty() -> Self {
        Self {
            children: HashMap::new(),
            value:    None,
        }
    }
}

#[derive(Clone, Copy)]
struct PrefixNodeId(usize);

impl PrefixNodeId {
    const ROOT: Self = Self(0);
}

struct PendingSequence<T> {
    prefix:         PrefixNodeId,
    deferred_match: DeferredMatch<T>,
    last_stroke_at: Instant,
}

impl<T: Copy> SequenceMatcher<T> {
    /// Builds a prefix index from non-empty keystroke sequences and their opaque values.
    #[must_use]
    pub fn new(sequences: impl IntoIterator<Item = (KeystrokeSequence, T)>) -> Self {
        let mut sequence_matcher = Self {
            prefix_index: vec![PrefixNode::empty()],
            pending:      None,
        };

        for (sequence, value) in sequences {
            sequence_matcher.insert(sequence, value);
        }

        sequence_matcher
    }

    /// Discards a pending prefix without resolving its deferred value.
    pub const fn cancel_pending(&mut self) { self.pending = None; }

    /// Reports whether a prefix is waiting for another keystroke or its timeout.
    #[must_use]
    pub const fn is_pending(&self) -> bool { self.pending.is_some() }

    /// Resolves a pending complete sequence when its timeout has elapsed.
    ///
    /// The timeout is measured from the most recent matched keystroke. A partial prefix without a
    /// deferred value is also discarded when the timeout elapses.
    pub fn resolve_timeout(&mut self, now: Instant, timeout: Duration) -> TimeoutOutcome<T> {
        let Some(pending_sequence) = self.pending.as_ref() else {
            return TimeoutOutcome::NoPendingSequence;
        };
        if now.duration_since(pending_sequence.last_stroke_at) < timeout {
            return TimeoutOutcome::AwaitingKeystroke;
        }

        match self
            .pending
            .take()
            .map_or(DeferredMatch::NothingToFire, |pending_sequence| {
                pending_sequence.deferred_match
            }) {
            DeferredMatch::Fire(value) => TimeoutOutcome::Resolved(value),
            DeferredMatch::NothingToFire => TimeoutOutcome::DiscardedPartialPrefix,
        }
    }

    fn insert(&mut self, sequence: KeystrokeSequence, value: T) {
        let mut prefix = PrefixNodeId::ROOT;

        for keystroke in sequence.iter().copied() {
            prefix = self.child_or_insert(prefix, keystroke);
        }

        self.prefix_index[prefix.0].value = Some(value);
    }

    fn child_or_insert(&mut self, prefix: PrefixNodeId, keystroke: Keystroke) -> PrefixNodeId {
        if let Some(child) = self.prefix_index[prefix.0].children.get(&keystroke) {
            return *child;
        }

        let child = PrefixNodeId(self.prefix_index.len());
        self.prefix_index.push(PrefixNode::empty());
        self.prefix_index[prefix.0]
            .children
            .insert(keystroke, child);

        child
    }

    fn matched_node(&self, prefix: PrefixNodeId, keystroke: Keystroke) -> Option<PrefixNodeId> {
        self.prefix_index[prefix.0]
            .children
            .get(&keystroke)
            .copied()
    }
}

impl<T: Copy> SequenceMatcher<T> {
    /// Matches one completed keystroke against the current prefix.
    ///
    /// When a pending completed sequence expires, this returns [`MatchOutcome::Reprocess`] so the
    /// caller can fire that sequence before matching this keystroke from the root.
    pub fn match_keystroke(
        &mut self,
        keystroke: Keystroke,
        now: Instant,
        timeout: Duration,
    ) -> MatchOutcome<T> {
        if let TimeoutOutcome::Resolved(value) = self.resolve_timeout(now, timeout) {
            return MatchOutcome::Reprocess {
                deferred_match: DeferredMatch::Fire(value),
                keystroke,
            };
        }

        let pending_sequence = self.pending.take();
        let prefix = pending_sequence
            .as_ref()
            .map_or(PrefixNodeId::ROOT, |pending_sequence| {
                pending_sequence.prefix
            });
        let carried = pending_sequence
            .as_ref()
            .map_or(DeferredMatch::NothingToFire, |pending_sequence| {
                pending_sequence.deferred_match
            });
        let Some(matched_prefix) = self.matched_node(prefix, keystroke) else {
            return pending_sequence.map_or(MatchOutcome::NoMatch, |pending_sequence| {
                MatchOutcome::Reprocess {
                    deferred_match: pending_sequence.deferred_match,
                    keystroke,
                }
            });
        };

        self.outcome_for_prefix(matched_prefix, carried, now)
    }

    fn outcome_for_prefix(
        &mut self,
        prefix: PrefixNodeId,
        carried: DeferredMatch<T>,
        now: Instant,
    ) -> MatchOutcome<T> {
        let prefix_node = &self.prefix_index[prefix.0];
        let value = prefix_node.value;

        match (value, prefix_node.children.is_empty()) {
            (Some(value), true) => MatchOutcome::Matched(value),
            (Some(value), false) => {
                self.pending = Some(PendingSequence {
                    prefix,
                    deferred_match: DeferredMatch::Fire(value),
                    last_stroke_at: now,
                });
                MatchOutcome::Deferred(value)
            },
            (None, false) => {
                self.pending = Some(PendingSequence {
                    prefix,
                    deferred_match: carried,
                    last_stroke_at: now,
                });
                MatchOutcome::Pending
            },
            (None, true) => MatchOutcome::NoMatch,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use bevy::input::keyboard::KeyCode;

    use super::DeferredMatch;
    use super::Keystroke;
    use super::KeystrokeSequence;
    use super::MatchOutcome;
    use super::SequenceMatcher;
    use super::TimeoutOutcome;
    use crate::KeystrokeSequenceParseError;
    use crate::keystroke::OrdinaryKey;

    const TIMEOUT: Duration = Duration::from_millis(500);

    fn keystroke(key: KeyCode) -> Keystroke {
        Keystroke::from_ordinary_key(crate::Modifiers::default(), OrdinaryKey(key))
    }

    fn matcher(
        sequences: &[(&str, usize)],
    ) -> Result<SequenceMatcher<usize>, KeystrokeSequenceParseError> {
        let parsed_sequences = sequences
            .iter()
            .map(|(source, value)| Ok((source.parse::<KeystrokeSequence>()?, *value)))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(SequenceMatcher::new(parsed_sequences))
    }

    #[test]
    fn complete_sequence_without_longer_candidate_matches_immediately()
    -> Result<(), KeystrokeSequenceParseError> {
        let mut sequence_matcher = matcher(&[("g", 1)])?;
        let now = Instant::now();

        assert_eq!(
            sequence_matcher.match_keystroke(keystroke(KeyCode::KeyG), now, TIMEOUT),
            MatchOutcome::Matched(1)
        );

        Ok(())
    }

    #[test]
    fn complete_prefix_defers_until_timeout() -> Result<(), KeystrokeSequenceParseError> {
        let mut sequence_matcher = matcher(&[("g", 1), ("g g", 2)])?;
        let now = Instant::now();

        assert_eq!(
            sequence_matcher.match_keystroke(keystroke(KeyCode::KeyG), now, TIMEOUT),
            MatchOutcome::Deferred(1)
        );
        assert_eq!(
            sequence_matcher.resolve_timeout(now + TIMEOUT, TIMEOUT),
            TimeoutOutcome::Resolved(1)
        );

        Ok(())
    }

    #[test]
    fn mismatched_stroke_fires_deferred_prefix_then_reprocesses_from_root()
    -> Result<(), KeystrokeSequenceParseError> {
        let mut sequence_matcher = matcher(&[("g", 1), ("g g", 2), ("h", 3)])?;
        let now = Instant::now();
        let g = keystroke(KeyCode::KeyG);
        let h = keystroke(KeyCode::KeyH);

        assert_eq!(
            sequence_matcher.match_keystroke(g, now, TIMEOUT),
            MatchOutcome::Deferred(1)
        );
        assert_eq!(
            sequence_matcher.match_keystroke(h, now, TIMEOUT),
            MatchOutcome::Reprocess {
                deferred_match: DeferredMatch::Fire(1),
                keystroke:      h,
            }
        );
        assert_eq!(
            sequence_matcher.match_keystroke(h, now, TIMEOUT),
            MatchOutcome::Matched(3)
        );

        Ok(())
    }

    #[test]
    fn cancel_pending_discards_deferred_prefix() -> Result<(), KeystrokeSequenceParseError> {
        let mut sequence_matcher = matcher(&[("g", 1), ("g g", 2)])?;
        let now = Instant::now();

        assert_eq!(
            sequence_matcher.match_keystroke(keystroke(KeyCode::KeyG), now, TIMEOUT),
            MatchOutcome::Deferred(1)
        );
        sequence_matcher.cancel_pending();
        assert!(!sequence_matcher.is_pending());
        assert_eq!(
            sequence_matcher.resolve_timeout(now + TIMEOUT, TIMEOUT),
            TimeoutOutcome::NoPendingSequence
        );

        Ok(())
    }

    #[test]
    fn three_stroke_sequence_matches_after_each_prefix() -> Result<(), KeystrokeSequenceParseError>
    {
        let mut sequence_matcher = matcher(&[("g g g", 1)])?;
        let now = Instant::now();
        let g = keystroke(KeyCode::KeyG);

        assert_eq!(
            sequence_matcher.match_keystroke(g, now, TIMEOUT),
            MatchOutcome::Pending
        );
        assert_eq!(
            sequence_matcher.match_keystroke(g, now, TIMEOUT),
            MatchOutcome::Pending
        );
        assert_eq!(
            sequence_matcher.match_keystroke(g, now, TIMEOUT),
            MatchOutcome::Matched(1)
        );

        Ok(())
    }

    #[test]
    fn shared_two_stroke_prefix_disambiguates_on_third_stroke()
    -> Result<(), KeystrokeSequenceParseError> {
        let mut sequence_matcher = matcher(&[("g g a", 1), ("g g b", 2)])?;
        let now = Instant::now();
        let g = keystroke(KeyCode::KeyG);

        assert_eq!(
            sequence_matcher.match_keystroke(g, now, TIMEOUT),
            MatchOutcome::Pending
        );
        assert_eq!(
            sequence_matcher.match_keystroke(g, now, TIMEOUT),
            MatchOutcome::Pending
        );
        assert_eq!(
            sequence_matcher.match_keystroke(keystroke(KeyCode::KeyA), now, TIMEOUT),
            MatchOutcome::Matched(1)
        );
        assert_eq!(
            sequence_matcher.match_keystroke(g, now, TIMEOUT),
            MatchOutcome::Pending
        );
        assert_eq!(
            sequence_matcher.match_keystroke(g, now, TIMEOUT),
            MatchOutcome::Pending
        );
        assert_eq!(
            sequence_matcher.match_keystroke(keystroke(KeyCode::KeyB), now, TIMEOUT),
            MatchOutcome::Matched(2)
        );

        Ok(())
    }

    #[test]
    fn incomplete_prefix_retains_deferred_match_until_timeout()
    -> Result<(), KeystrokeSequenceParseError> {
        let mut sequence_matcher = matcher(&[("g", 1), ("g g g", 2)])?;
        let now = Instant::now();
        let g = keystroke(KeyCode::KeyG);

        assert_eq!(
            sequence_matcher.match_keystroke(g, now, TIMEOUT),
            MatchOutcome::Deferred(1)
        );
        assert_eq!(
            sequence_matcher.match_keystroke(g, now, TIMEOUT),
            MatchOutcome::Pending
        );
        assert_eq!(
            sequence_matcher.resolve_timeout(now + TIMEOUT, TIMEOUT),
            TimeoutOutcome::Resolved(1)
        );

        Ok(())
    }

    #[test]
    fn incomplete_prefix_reprocesses_the_deferred_match() -> Result<(), KeystrokeSequenceParseError>
    {
        let mut sequence_matcher = matcher(&[("g", 1), ("g g g", 2)])?;
        let now = Instant::now();
        let g = keystroke(KeyCode::KeyG);
        let h = keystroke(KeyCode::KeyH);

        assert_eq!(
            sequence_matcher.match_keystroke(g, now, TIMEOUT),
            MatchOutcome::Deferred(1)
        );
        assert_eq!(
            sequence_matcher.match_keystroke(g, now, TIMEOUT),
            MatchOutcome::Pending
        );
        assert_eq!(
            sequence_matcher.match_keystroke(h, now, TIMEOUT),
            MatchOutcome::Reprocess {
                deferred_match: DeferredMatch::Fire(1),
                keystroke:      h,
            }
        );

        Ok(())
    }

    #[test]
    fn longer_complete_match_supersedes_the_deferred_match()
    -> Result<(), KeystrokeSequenceParseError> {
        let mut sequence_matcher = matcher(&[("g", 1), ("g g", 2)])?;
        let now = Instant::now();
        let g = keystroke(KeyCode::KeyG);

        assert_eq!(
            sequence_matcher.match_keystroke(g, now, TIMEOUT),
            MatchOutcome::Deferred(1)
        );
        assert_eq!(
            sequence_matcher.match_keystroke(g, now, TIMEOUT),
            MatchOutcome::Matched(2)
        );
        assert_eq!(
            sequence_matcher.resolve_timeout(now + TIMEOUT, TIMEOUT),
            TimeoutOutcome::NoPendingSequence
        );

        Ok(())
    }

    #[test]
    fn matching_after_construction_does_not_allocate() -> Result<(), KeystrokeSequenceParseError> {
        let mut sequence_matcher = matcher(&[("g", 1)])?;
        let g = keystroke(KeyCode::KeyG);
        let now = Instant::now();
        let allocations_before = crate::TEST_ALLOCATOR.allocation_count();

        for _ in 0..10 {
            assert_eq!(
                sequence_matcher.match_keystroke(g, now, TIMEOUT),
                MatchOutcome::Matched(1)
            );
        }

        assert_eq!(crate::TEST_ALLOCATOR.allocation_count(), allocations_before);

        Ok(())
    }
}
