//! The register of identity questions only a human can settle, and the answers that settle them.
//!
//! A reporter that claims a device-reported identity is claiming the value is stable for that unit.
//! When such a key later fails to match while a same-kind unit occupies the attachment the saved
//! one left, `crate::reconcile` concludes `crate::IdentityVerdict::Displaced` or
//! `crate::IdentityVerdict::WrongUnit` and records the debt in
//! `crate::IdentityDecisionOwed::HumanDecision`. Short of retiring the role, this module is the
//! only thing that discharges that debt; a unit carrying it undischarged is unusable, because both
//! authorization predicates reject it through `crate::IdentityVerdict::identified`.
//!
//! `IdentityDecisions` is where the question waits. A human answers many frames after the question
//! arose, from whatever system the application's interface runs in, so `IdentityDecisions::answer`
//! only *records* the answer: the kernel applies it in `adjudicate_identity_questions` during its
//! next `crate::RiggingSystems::Reconcile`. Everything a dialog needs to survive that gap follows
//! from it — a question is named by its role and its candidate `crate::DeviceKey` rather than by a
//! position in the list, and an answer whose question expired while the operator was reading it
//! resolves to `AdoptionOutcome::NoSuchQuestion` instead of panicking or silently doing nothing.
//!
//! A `crate::DeviceIdSource::Synthesized` key raises nothing here. A value that rotates on every
//! replug is not an identifier, and a reporter that synthesizes a key from stable evidence has
//! already made the decision this register exists for: the kernel takes the claim at face value,
//! answers `crate::IdentityVerdict::RestoreOnly`, and never asks the application anything.

use bevy::ecs::reflect::ReflectResource;
use bevy::ecs::system::Commands;
use bevy::ecs::system::Res;
use bevy::ecs::system::ResMut;
use bevy::prelude::Reflect;
use bevy::prelude::Resource;

use crate::Bindings;
use crate::DeviceIdSource;
use crate::DeviceKey;
use crate::DeviceResolution;
use crate::DeviceStateLookup;
use crate::Devices;
use crate::HardwareInventory;
use crate::IdentityDecisionOwed;
use crate::IdentityQuestionExpired;
use crate::IdentityQuestionRaised;
use crate::IdentityVerdict;
use crate::Presence;
use crate::RiggingRevision;
use crate::RoleKey;
use crate::binding::BindingError;
use crate::binding::EndpointOwner;

/// Whether the operator has looked at one standing question yet.
///
/// Stored rather than derived from the absence of an answer: a dialog that shows one question at a
/// time cannot otherwise tell "nobody has seen this" from "the operator said later", and it
/// re-opens the same question on every frame until the hardware goes away. There is no answered
/// variant, because answering removes the entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum IdentityQuestionState {
    /// The question has been raised and no operator action has touched it.
    #[default]
    Unseen,
    /// The operator saw the question and postponed it. What postponement means for the interface —
    /// a dismissed modal, a row moved to the bottom of a table — is the application's to decide;
    /// the kernel only keeps the entry standing and stops nothing on account of it.
    Deferred,
}

/// One unanswered question about whether a live unit replaces the one a role's saved key names.
///
/// The three keys travel together because none of them is recoverable from the others once the pass
/// that produced the question is over: `saved` is what the binding still points at, `candidate` is
/// what actually arrived, and `role` is the binding whose endpoint an adoption rewrites.
#[derive(Debug, Reflect)]
pub struct IdentityQuestion {
    /// Application role whose binding still addresses `saved`.
    pub role:      RoleKey,
    /// Durable key the role was bound against, which no live unit matched on the pass that raised
    /// this question.
    pub saved:     DeviceKey,
    /// Durable key of the unit that arrived into the attachment `saved` left, and the one an
    /// adoption rewrites the binding onto.
    pub candidate: DeviceKey,
    /// Reconcile pass the question arose on, so an interface can order questions that arrived
    /// while an earlier dialog sat open and report how long one has waited.
    pub arose:     RiggingRevision,
    /// Whether the operator has already postponed this question.
    pub state:     IdentityQuestionState,
    /// Which role owned the endpoint an adoption would move this role onto, as of the last
    /// reconcile pass.
    ///
    /// Cached rather than read at answer time because `IdentityDecisions::answer` is called from
    /// application code that holds nothing but this resource, and an adoption that quietly took an
    /// endpoint from another role is the one outcome the register must never produce.
    owner:         EndpointOwner,
}

/// What the operator decided about the candidate unit.
///
/// Both answers are recorded against the candidate key rather than clearing a flag, because the
/// kernel re-derives the mismatch on every pass: a cleared flag re-arms the same question one frame
/// later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum IdentityAnswer {
    /// The candidate is the unit the role should address from now on, so the saved key becomes the
    /// candidate key and the binding endpoint moves with it.
    Adopt,
    /// The candidate is not the saved unit. The binding keeps pointing at `IdentityQuestion::saved`
    /// and this candidate never raises the question again for this role; a later third unit
    /// arriving into the same attachment does.
    Reject,
}

/// Result of reading the one question standing for a role.
///
/// A named result rather than an optional reference so a caller learns at the lookup that a role
/// simply has nothing outstanding, which is the ordinary case and not a missing value.
#[derive(Debug)]
pub enum IdentityQuestionLookup<'a> {
    /// This role has no outstanding identity question.
    NoQuestion,
    /// This role has one question waiting for an answer.
    Pending(&'a IdentityQuestion),
}

/// What answering one question did.
///
/// The name follows adoption because adoption is the branch that can fail: rejecting a candidate
/// touches no binding and always succeeds.
#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
pub enum AdoptionOutcome {
    /// The adoption was recorded. The kernel rewrites the binding endpoint and the inventory entry
    /// together during its next `crate::RiggingSystems::Reconcile`.
    Adopted,
    /// The rejection was recorded. The entry is gone and this candidate raises nothing further for
    /// this role.
    Refused,
    /// Another role already owns the endpoint this adoption would move the role onto, so nothing
    /// was recorded and the question still stands. Adoption never takes an endpoint from another
    /// role; resolving the conflict — retiring the other role, or rejecting this candidate — is the
    /// application's decision.
    CandidateEndpointOwned {
        /// Role that holds the candidate endpoint.
        by: RoleKey,
    },
    /// No question stands for this role and candidate. The usual cause is expiry: the device
    /// departed or the role was retired between the moment the operator read the question and the
    /// moment they answered it. Nothing was recorded and no binding changed.
    NoSuchQuestion,
}

/// Result of checking every identity question one device-wide adoption would answer.
///
/// Preparation reads the complete device role set supplied by an integration and changes no
/// question or recorded answer. The prepared value can therefore travel with the integration's
/// own table preflight before `IdentityDecisions::answer_prepared_adoption` records the answers.
#[derive(Debug)]
pub enum IdentityAdoptionPreparation {
    /// Every matching standing question can be answered together.
    Prepared(PreparedIdentityAdoption),
    /// At least one matching question cannot be adopted, so no answer was recorded.
    Refused(AdoptionOutcome),
}

/// One checked device-wide set of identity questions ready to be adopted atomically.
///
/// The role set belongs to the integration because only it knows which independently routed roles
/// share one physical device. The kernel keeps this type device- and identity-specific: it matches
/// standing questions by role, saved key, and candidate key without naming an integration kind.
#[derive(Debug)]
pub struct PreparedIdentityAdoption {
    role:           RoleKey,
    saved:          DeviceKey,
    candidate:      DeviceKey,
    device_roles:   Vec<RoleKey>,
    question_roles: Vec<RoleKey>,
}

/// One answer waiting for the kernel to act on it.
struct RecordedAnswer {
    role:      RoleKey,
    saved:     DeviceKey,
    candidate: DeviceKey,
    answer:    IdentityAnswer,
    state:     IdentityQuestionState,
    arose:     RiggingRevision,
}

/// One candidate an answer has already settled for one role, kept so nothing re-arms.
///
/// The saved key and the answer travel with it because a refusal outlives the unit it refused: when
/// a later unit displaces the refused one, the register has to recover which role asked and which
/// saved key it was still addressing.
struct SettledCandidate {
    role:      RoleKey,
    saved:     DeviceKey,
    candidate: DeviceKey,
    answer:    IdentityAnswer,
    discharge: IdentityDebtDischarge,
}

/// Positions of every standing question one prepared device adoption will answer.
struct IdentityAdoptionQuestionSet {
    positions: Vec<usize>,
    roles:     Vec<RoleKey>,
}

/// Whether `Devices::discharge_identity_decision` has already run for a settled candidate.
///
/// Stored rather than derived from the entry's existence because a refusal's entry outlives its
/// discharge: it is what stops the same candidate raising the question again. Without this the
/// register would hand the same refusal back on every frame and `crate::Devices` would be marked
/// changed for the life of the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdentityDebtDischarge {
    /// The candidate still carries the `crate::IdentityDecisionOwed::HumanDecision` the answer
    /// settled.
    Owed,
    /// The debt has been cleared, so later passes conclude the unit's verdict from its own key.
    Done,
}

/// Every identity question a human still owes an answer to, in the order the questions arose.
///
/// Read the list, answer whichever entries the interface has room for, and leave the rest standing:
/// several units can go ambiguous at once and more can arrive while an operator is working through
/// an earlier one. Nothing here is mirrored onto an entity — the register is a queue of questions,
/// not a state axis of one device — so `crate::IdentityQuestionRaised` and
/// `crate::IdentityQuestionExpired` are what a consumer watches to keep a dialog in step with it.
///
/// Persistence is the application's. Adopting a candidate tells the application which durable key
/// its saved configuration should be rewritten to; the kernel does no file I/O and will ask the
/// same question again after a relaunch if the application did not record the answer.
#[derive(Default, Resource, Reflect)]
#[reflect(Resource)]
pub struct IdentityDecisions {
    questions: Vec<IdentityQuestion>,
    #[reflect(ignore, default = "Vec::new")]
    recorded:  Vec<RecordedAnswer>,
    #[reflect(ignore, default = "Vec::new")]
    settled:   Vec<SettledCandidate>,
}

impl IdentityDecisions {
    /// Read every standing question in the order the questions arose.
    ///
    /// A shared slice rather than a mutable entry handle: the only writes the register accepts are
    /// `Self::answer` and `Self::defer`, both of which name the question they act on, so a dialog
    /// that outlived the entry it was opened for cannot edit a different one that took its place.
    #[must_use]
    pub fn questions(&self) -> &[IdentityQuestion] { &self.questions }

    /// Read the one question standing for one role.
    #[must_use]
    pub fn question(&self, role: &RoleKey) -> IdentityQuestionLookup<'_> {
        self.questions
            .iter()
            .find(|question| &question.role == role)
            .map_or(IdentityQuestionLookup::NoQuestion, |question| {
                IdentityQuestionLookup::Pending(question)
            })
    }

    /// Record what the operator decided about one candidate.
    ///
    /// The candidate key is part of the question's name, so an answer written against a question
    /// that has since expired and been replaced by a different one for the same role answers
    /// `AdoptionOutcome::NoSuchQuestion` instead of settling the wrong unit.
    ///
    /// The answer is recorded, not applied: the kernel rewrites bindings and inventory during its
    /// next `crate::RiggingSystems::Reconcile`, so a caller reading `crate::Bindings` in the same
    /// frame still sees the old endpoint.
    pub fn answer(
        &mut self,
        role: &RoleKey,
        candidate: &DeviceKey,
        identity_answer: IdentityAnswer,
    ) -> AdoptionOutcome {
        if identity_answer == IdentityAnswer::Adopt {
            return match self.prepare_device_adoption(role, candidate, std::iter::once(role)) {
                IdentityAdoptionPreparation::Prepared(prepared) => {
                    self.answer_prepared_adoption(prepared)
                },
                IdentityAdoptionPreparation::Refused(outcome) => outcome,
            };
        }
        let Some(position) = self
            .questions
            .iter()
            .position(|question| &question.role == role && &question.candidate == candidate)
        else {
            return AdoptionOutcome::NoSuchQuestion;
        };

        let question = self.questions.remove(position);
        self.recorded.push(RecordedAnswer {
            role:      question.role,
            saved:     question.saved,
            candidate: question.candidate,
            answer:    identity_answer,
            state:     question.state,
            arose:     question.arose,
        });

        AdoptionOutcome::Refused
    }

    /// Check every standing question a device-wide adoption would answer.
    ///
    /// `device_roles` is the integration's complete set of independently routed roles for the
    /// physical device named by the selected question's saved key. Preparation includes every
    /// standing question whose role belongs to that set and whose saved and candidate keys exactly
    /// match the selected question. Questions outside the role set or with either different key
    /// remain outside the prepared adoption.
    ///
    /// The selected role must itself belong to `device_roles`. A missing, replaced, or out-of-scope
    /// selected question produces `IdentityAdoptionPreparation::Refused` with
    /// `AdoptionOutcome::NoSuchQuestion`.
    pub fn prepare_device_adoption<'a>(
        &self,
        role: &RoleKey,
        candidate: &DeviceKey,
        device_roles: impl IntoIterator<Item = &'a RoleKey>,
    ) -> IdentityAdoptionPreparation {
        let mut device_role_set = Vec::new();
        for device_role in device_roles {
            if !device_role_set.contains(device_role) {
                device_role_set.push(device_role.clone());
            }
        }
        if !device_role_set.contains(role) {
            return IdentityAdoptionPreparation::Refused(AdoptionOutcome::NoSuchQuestion);
        }
        let Some(selected_question) = self
            .questions
            .iter()
            .find(|question| &question.role == role && &question.candidate == candidate)
        else {
            return IdentityAdoptionPreparation::Refused(AdoptionOutcome::NoSuchQuestion);
        };
        let saved = selected_question.saved.clone();
        let candidate = selected_question.candidate.clone();
        let question_set =
            match self.adoption_question_set(role, &saved, &candidate, &device_role_set) {
                Ok(question_set) => question_set,
                Err(outcome) => return IdentityAdoptionPreparation::Refused(outcome),
            };
        IdentityAdoptionPreparation::Prepared(PreparedIdentityAdoption {
            role: role.clone(),
            saved,
            candidate,
            device_roles: device_role_set,
            question_roles: question_set.roles,
        })
    }

    /// Record every answer in one prepared device adoption after one complete recheck.
    ///
    /// No question is removed until every matching entry still exists and every cached candidate
    /// endpoint remains unowned. If any entry changed after preparation, the returned refusal
    /// leaves the complete register unchanged.
    pub fn answer_prepared_adoption(
        &mut self,
        prepared: PreparedIdentityAdoption,
    ) -> AdoptionOutcome {
        let question_set = match self.adoption_question_set(
            &prepared.role,
            &prepared.saved,
            &prepared.candidate,
            &prepared.device_roles,
        ) {
            Ok(question_set) => question_set,
            Err(outcome) => return outcome,
        };
        if question_set.roles != prepared.question_roles {
            return AdoptionOutcome::NoSuchQuestion;
        }
        let mut answered = Vec::with_capacity(question_set.positions.len());
        for position in question_set.positions.into_iter().rev() {
            answered.push(self.questions.remove(position));
        }
        for question in answered.into_iter().rev() {
            self.recorded.push(RecordedAnswer {
                role:      question.role,
                saved:     question.saved,
                candidate: question.candidate,
                answer:    IdentityAnswer::Adopt,
                state:     question.state,
                arose:     question.arose,
            });
        }

        AdoptionOutcome::Adopted
    }

    /// Mark one standing question as postponed and read it back.
    ///
    /// The entry stays in the register: postponing is not answering, and a question nobody answered
    /// still describes a unit that cannot be used. The read-back reports
    /// `IdentityQuestionLookup::NoQuestion` when the entry expired before the postponement reached
    /// it, which is the same asynchronous case `Self::answer` reports as
    /// `AdoptionOutcome::NoSuchQuestion`.
    pub fn defer(&mut self, role: &RoleKey, candidate: &DeviceKey) -> IdentityQuestionLookup<'_> {
        let Some(question) = self
            .questions
            .iter_mut()
            .find(|question| &question.role == role && &question.candidate == candidate)
        else {
            return IdentityQuestionLookup::NoQuestion;
        };
        question.state = IdentityQuestionState::Deferred;

        IdentityQuestionLookup::Pending(question)
    }

    /// Report whether an answer has already settled this candidate for this role.
    fn is_settled(&self, role: &RoleKey, candidate: &DeviceKey) -> bool {
        self.settled
            .iter()
            .any(|settled| &settled.role == role && &settled.candidate == candidate)
    }

    /// Report whether a question about this candidate still stands for any role.
    ///
    /// The identity debt on a device is one value shared by every role that named it, so it is only
    /// discharged once nothing is still waiting on that unit.
    fn any_question_about(&self, candidate: &DeviceKey) -> bool {
        self.questions
            .iter()
            .any(|question| &question.candidate == candidate)
    }

    /// Report whether an answer for this role and candidate is still waiting to be applied.
    ///
    /// A recorded answer holds no entry in `Self::questions`, so without this the next pass would
    /// re-derive the same mismatch and announce `crate::IdentityQuestionRaised` a second time for a
    /// question the operator has already answered.
    fn is_recorded(&self, role: &RoleKey, candidate: &DeviceKey) -> bool {
        self.recorded
            .iter()
            .any(|recorded| &recorded.role == role && &recorded.candidate == candidate)
    }

    /// Put a recorded adoption back on the register because the endpoint went to another role
    /// between the answer and this pass.
    ///
    /// Inserted at the position `IdentityQuestion::arose` puts it in rather than appended, because
    /// `Self::questions` is documented as being in the order the questions arose and a dialog that
    /// drains it one at a time would otherwise show the re-contested question last.
    fn reinstate(&mut self, recorded_answer: RecordedAnswer, owner: EndpointOwner) {
        let reinstated = IdentityQuestion {
            role: recorded_answer.role,
            saved: recorded_answer.saved,
            candidate: recorded_answer.candidate,
            arose: recorded_answer.arose,
            state: recorded_answer.state,
            owner,
        };
        let standing = self
            .questions
            .iter()
            .position(|question| question.arose > reinstated.arose)
            .unwrap_or(self.questions.len());
        self.questions.insert(standing, reinstated);
    }

    /// Forget every adoption whose debt this pass cleared, and mark every refusal cleared.
    ///
    /// The two answers retain differently because their entries are held open for different
    /// reasons: an adoption's entry exists only to carry the debt as far as its discharge, while a
    /// refusal's entry is what stops the same candidate raising the question again and has to
    /// outlive the unit it refused.
    fn forget_discharged(&mut self, discharged: &[DeviceKey]) {
        self.settled.retain_mut(|settled| {
            if settled.discharge == IdentityDebtDischarge::Done
                || !discharged.contains(&settled.candidate)
            {
                return true;
            }
            settled.discharge = IdentityDebtDischarge::Done;

            settled.answer == IdentityAnswer::Reject
        });
    }

    /// Recheck and locate the complete exact-match set for one prepared device adoption.
    fn adoption_question_set(
        &self,
        role: &RoleKey,
        saved: &DeviceKey,
        candidate: &DeviceKey,
        device_roles: &[RoleKey],
    ) -> Result<IdentityAdoptionQuestionSet, AdoptionOutcome> {
        let selected_still_stands = self.questions.iter().any(|question| {
            &question.role == role && &question.saved == saved && &question.candidate == candidate
        });
        if !selected_still_stands {
            return Err(AdoptionOutcome::NoSuchQuestion);
        }
        let mut positions = Vec::new();
        let mut roles = Vec::new();
        for (position, question) in self.questions.iter().enumerate() {
            if &question.saved != saved
                || &question.candidate != candidate
                || !device_roles.contains(&question.role)
            {
                continue;
            }
            if let EndpointOwner::OwnedBy(owner) = &question.owner {
                return Err(AdoptionOutcome::CandidateEndpointOwned { by: owner.clone() });
            }
            positions.push(position);
            roles.push(question.role.clone());
        }

        Ok(IdentityAdoptionQuestionSet { positions, roles })
    }
}

/// Apply recorded answers, expire questions whose hardware or role is gone, and raise new ones.
///
/// Runs inside `crate::RiggingSystems::Reconcile` after the device entities are projected, which is
/// what makes "the kernel applies it during its next reconcile" true for an answer written from any
/// system in the previous frame. The three steps run in this order because each depends on the one
/// before it: an answer settles a candidate, expiry removes what the settled pass made unreachable,
/// and only what survives both can raise.
///
/// Each step reaches for mutable access only once it has work — the expiry pass and the
/// endpoint-owner refresh, both derived as lists read out of the standing questions before anything
/// is written, and the settled discharges, which are taken once per answer rather than re-offered
/// on every later pass. A frame that applies no answer, expires no question, refreshes no owner and
/// raises nothing therefore leaves `crate::IdentityDecisions`, `crate::Bindings`, `crate::Devices`,
/// and `crate::HardwareInventory` unchanged — which includes every frame on which a question merely
/// stands, the state the register holds for as long as a dialog waits on a human — the same way
/// `crate::binding::drain_binding_transitions` leaves an empty queue alone.
pub(crate) fn adjudicate_identity_questions(
    mut commands: Commands,
    mut identity_decisions: ResMut<IdentityDecisions>,
    mut bindings: ResMut<Bindings>,
    mut devices: ResMut<Devices>,
    mut hardware_inventory: ResMut<HardwareInventory>,
    rigging_revision: Res<RiggingRevision>,
) {
    if !identity_decisions.recorded.is_empty() {
        apply_recorded_answers(
            &mut commands,
            &mut identity_decisions,
            &mut bindings,
            &mut hardware_inventory,
        );
    }
    let unanswerable = unanswerable_questions(&identity_decisions, &bindings, &devices);
    if !unanswerable.is_empty() {
        expire_questions(&mut commands, &mut identity_decisions, &unanswerable);
    }

    let raise_pass =
        raise_new_questions(&identity_decisions, &bindings, &devices, *rigging_revision);
    for question in raise_pass.questions {
        commands.trigger(IdentityQuestionRaised {
            role:      question.role.clone(),
            candidate: question.candidate.clone(),
        });
        identity_decisions.questions.push(question);
    }

    for endpoint_owner_refresh in stale_endpoint_owners(&identity_decisions, &bindings) {
        if let Some(question) = identity_decisions
            .questions
            .get_mut(endpoint_owner_refresh.standing)
        {
            question.owner = endpoint_owner_refresh.owner;
        }
    }

    let settled_discharges = settled_discharges(&identity_decisions);
    if !settled_discharges.is_empty() {
        for candidate in &settled_discharges {
            devices.discharge_identity_decision(candidate);
        }
        identity_decisions.forget_discharged(&settled_discharges);
    }
    for candidate in raise_pass.unanswerable {
        devices.discharge_identity_decision(&candidate);
    }
}

/// Rewrite the binding endpoint and the inventory entry of every adoption, and settle every
/// rejection.
///
/// Both answers mark the candidate settled for that role, which is what stops the next pass from
/// deriving the same mismatch and raising the question again.
fn apply_recorded_answers(
    commands: &mut Commands,
    identity_decisions: &mut IdentityDecisions,
    bindings: &mut Bindings,
    hardware_inventory: &mut HardwareInventory,
) {
    let mut still_waiting: Vec<RecordedAnswer> = Vec::new();
    for recorded_answer in std::mem::take(&mut identity_decisions.recorded) {
        match recorded_answer.answer {
            IdentityAnswer::Adopt => {
                match bindings.readdress(&recorded_answer.role, recorded_answer.candidate.clone()) {
                    Ok(()) => {
                        hardware_inventory
                            .readdress(&recorded_answer.saved, recorded_answer.candidate.clone());
                        identity_decisions.settled.push(SettledCandidate {
                            role:      recorded_answer.role,
                            saved:     recorded_answer.saved,
                            candidate: recorded_answer.candidate,
                            answer:    IdentityAnswer::Adopt,
                            discharge: IdentityDebtDischarge::Owed,
                        });
                    },
                    Err(BindingError::EndpointAlreadyOwned { owner, .. }) => {
                        commands.trigger(IdentityQuestionRaised {
                            role:      recorded_answer.role.clone(),
                            candidate: recorded_answer.candidate.clone(),
                        });
                        identity_decisions
                            .reinstate(recorded_answer, EndpointOwner::OwnedBy(owner));
                    },
                    Err(BindingError::PendingTransitionCapacityReached) => {
                        // The handoff drains every frame, so this refusal is about this pass and
                        // not about the adoption. Keeping the answer waiting is what makes the
                        // `AdoptionOutcome::Adopted` the caller already received true, and keeping
                        // it out of `IdentityDecisions::questions` is what stops
                        // `raise_new_questions` announcing the same question a second time.
                        still_waiting.push(recorded_answer);
                    },
                    Err(_) => {
                        // `BindingError::RoleNotBound` — the role was retired between the answer
                        // and this pass — or `BindingError::TransitionSequenceExhausted`, which
                        // never recovers. Either way no binding can move onto the candidate, so
                        // the question the answer was written against is gone.
                        commands.trigger(IdentityQuestionExpired {
                            role:      recorded_answer.role,
                            candidate: recorded_answer.candidate,
                        });
                    },
                }
            },
            IdentityAnswer::Reject => {
                identity_decisions.settled.push(SettledCandidate {
                    role:      recorded_answer.role,
                    saved:     recorded_answer.saved,
                    candidate: recorded_answer.candidate,
                    answer:    IdentityAnswer::Reject,
                    discharge: IdentityDebtDischarge::Owed,
                });
            },
        }
    }
    identity_decisions.recorded = still_waiting;
}

/// One standing question this pass can no longer answer, named by where it sits rather than lifted
/// out of the register.
///
/// Carried out of the read for the reason `EndpointOwnerRefresh` gives: reaching for
/// `bevy::ecs::system::ResMut<IdentityDecisions>` mutably is itself what marks the register
/// changed, so a pass on which every question is still answerable must not reach for it at all.
/// Rewriting `IdentityDecisions::questions` with the same entries it already held marked the
/// register changed on every frame a question merely stood, and a consumer watching it through
/// change detection saw a change that had not happened.
struct UnanswerableQuestion {
    /// Position in `IdentityDecisions::questions` of the entry that can no longer be answered.
    standing: usize,
}

/// Find every standing question whose role was retired or whose candidate unit is gone.
///
/// Positions come out in ascending order, which is what `expire_questions` depends on to remove
/// last position first: a caller that reorders them addresses the wrong entries.
fn unanswerable_questions(
    identity_decisions: &IdentityDecisions,
    bindings: &Bindings,
    devices: &Devices,
) -> Vec<UnanswerableQuestion> {
    identity_decisions
        .questions
        .iter()
        .enumerate()
        .filter(|(_, question)| {
            let role_retired = bindings.binding(&question.role).is_err();
            let candidate_hardware = candidate_hardware(devices, &question.candidate);

            role_retired || candidate_hardware == CandidateHardware::Departed
        })
        .map(|(standing, _)| UnanswerableQuestion { standing })
        .collect()
}

/// Drop the questions this pass can no longer answer, and announce each one.
///
/// Without this the register accumulates questions about hardware that left, and a dialog opened
/// against one of them has no signal that the entry vanished underneath it.
///
/// Entries are removed last position first, which requires `unanswerable` to be in the ascending
/// order `unanswerable_questions` produces: only then do the earlier positions still address the
/// entries they were derived from. They are announced in the order they arose, which is the order
/// `IdentityDecisions::questions` is documented to hold.
fn expire_questions(
    commands: &mut Commands,
    identity_decisions: &mut IdentityDecisions,
    unanswerable: &[UnanswerableQuestion],
) {
    let mut expired = Vec::with_capacity(unanswerable.len());
    for unanswerable_question in unanswerable.iter().rev() {
        expired.push(
            identity_decisions
                .questions
                .remove(unanswerable_question.standing),
        );
    }
    for question in expired.into_iter().rev() {
        commands.trigger(IdentityQuestionExpired {
            role:      question.role,
            candidate: question.candidate,
        });
    }
}

/// Whether the unit a standing question names is still there to be answered about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CandidateHardware {
    /// The key resolves to a retained device the contributors still report `Presence::Present`, so
    /// an adoption would move the binding onto hardware that is actually there.
    Present,
    /// The unit is gone, by either of the two departures `crate::DeviceDeparture` distinguishes.
    Departed,
}

/// Decide whether one candidate key still names usable hardware.
///
/// `Devices::resolve` alone cannot answer this: it reports only whether the key is in the
/// reconciled identity map, and a unit a reporter still enumerates while no longer reporting it
/// present keeps its handle, its state, and its entity. That is exactly
/// `crate::DeviceDeparture::RetainedButNotPresent`, the departure `crate::DeviceDeparted` fires
/// for, so a question about it has to expire the same way one about an unplugged unit does.
fn candidate_hardware(devices: &Devices, candidate: &DeviceKey) -> CandidateHardware {
    let DeviceResolution::Resolved(device_id) = devices.resolve(candidate) else {
        return CandidateHardware::Departed;
    };
    let DeviceStateLookup::Retained(reconciled_device_state) = devices.state(device_id) else {
        return CandidateHardware::Departed;
    };
    if reconciled_device_state.presence == Presence::Present {
        CandidateHardware::Present
    } else {
        CandidateHardware::Departed
    }
}

/// What one pass over the reconciled device states found to raise.
///
/// The two lists travel together because one pass over `crate::Devices` produces both: a device
/// whose debt no human can answer is the same read that would otherwise have raised a question
/// about it.
struct RaisePass {
    /// Questions to add to the register, in the order the pass derived them.
    questions:    Vec<IdentityQuestion>,
    /// Candidates no human can be asked about, so their identity debt is discharged instead:
    /// either the saved key claims no stable identity, or no role was ever bound to it.
    unanswerable: Vec<DeviceKey>,
}

/// Derive one question per role that still addresses a saved key a live unit displaced.
///
/// The debt is read from `crate::ReconciledDeviceState::decision_owed` rather than recomputed,
/// because the join between an arriving unit and the slot a saved one left exists only on the pass
/// the unit arrived.
///
/// Deriving rather than registering is what lets a pass that finds nothing leave
/// `IdentityDecisions` untouched: `adjudicate_identity_questions` pushes and announces each
/// question in `RaisePass::questions`.
fn raise_new_questions(
    identity_decisions: &IdentityDecisions,
    bindings: &Bindings,
    devices: &Devices,
    rigging_revision: RiggingRevision,
) -> RaisePass {
    let mut raised: Vec<IdentityQuestion> = Vec::new();
    let mut unanswerable: Vec<DeviceKey> = Vec::new();
    for reconciled_device_state in devices.states() {
        let IdentityDecisionOwed::HumanDecision(outstanding_verdict) =
            &reconciled_device_state.decision_owed
        else {
            continue;
        };
        if reconciled_device_state.presence != Presence::Present {
            // The debt outlives the unit's presence, so without this the pass would re-raise the
            // question `expire_questions` retired in this same frame. It is raised
            // again if the unit comes back, which is the only state in which it can be answered.
            continue;
        }
        let Some(saved) = saved_key(outstanding_verdict) else {
            continue;
        };
        if !answerable(saved) {
            unanswerable.push(reconciled_device_state.key.clone());
            continue;
        }
        let candidate = &reconciled_device_state.key;
        let askable_roles = askable_roles(identity_decisions, bindings, saved);
        if askable_roles.is_empty() {
            unanswerable.push(candidate.clone());
            continue;
        }
        for askable_role in askable_roles {
            let role = &askable_role.role;
            if identity_decisions.is_settled(role, candidate)
                || identity_decisions.is_recorded(role, candidate)
                || identity_decisions
                    .questions
                    .iter()
                    .chain(raised.iter())
                    .any(|question| &question.role == role && &question.candidate == candidate)
            {
                continue;
            }
            raised.push(IdentityQuestion {
                role:      askable_role.role.clone(),
                saved:     askable_role.saved,
                candidate: candidate.clone(),
                arose:     rigging_revision,
                state:     IdentityQuestionState::Unseen,
                owner:     EndpointOwner::Unowned,
            });
        }
    }

    RaisePass {
        questions: raised,
        unanswerable,
    }
}

/// One role a displacement can be put to a human about, and the key that role is still addressing.
///
/// A named pair rather than a tuple because both members are keys and only one of them is the
/// *saved* side of the join: reading them positionally is how the candidate ends up recorded as the
/// key a binding was authored against.
#[derive(PartialEq, Eq)]
struct AskableRole {
    /// Application role whose binding still addresses `Self::saved`.
    role:  RoleKey,
    /// Durable key that role was bound against, and the one an adoption rewrites away from.
    saved: DeviceKey,
}

/// Which roles a unit displacing this key should be asked about, and the saved key each of them is
/// still addressing.
///
/// Usually one entry per role bound to the displaced key. A refused candidate adds the role that
/// refused it: refusing one unit settles nothing about the next unit to take the same place, and
/// that role is still addressing the saved key it was addressing when it refused. Without this a
/// refusal would be the last question the slot ever raises, and the role would sit unresolvable.
///
/// An empty result is the case no human can be asked about at all — nobody ever bound a role to the
/// saved key — and `raise_new_questions` discharges the debt rather than leaving it standing
/// forever with no question to answer it.
fn askable_roles(
    identity_decisions: &IdentityDecisions,
    bindings: &Bindings,
    saved: &DeviceKey,
) -> Vec<AskableRole> {
    let mut askable: Vec<AskableRole> = bindings
        .roles_for(saved)
        .map(|role| AskableRole {
            role:  role.clone(),
            saved: saved.clone(),
        })
        .collect();

    for settled in &identity_decisions.settled {
        if settled.answer != IdentityAnswer::Reject || &settled.candidate != saved {
            continue;
        }
        if !bindings
            .roles_for(&settled.saved)
            .any(|role| role == &settled.role)
        {
            continue;
        }
        let inherited = AskableRole {
            role:  settled.role.clone(),
            saved: settled.saved.clone(),
        };
        if !askable.contains(&inherited) {
            askable.push(inherited);
        }
    }

    askable
}

/// Report whether a saved key's own source makes the mismatch worth asking a human about.
///
/// Only a claim of stability can be violated. `crate::DeviceIdSource::Reported` is a reporter
/// asserting the unit itself supplies this value, and `crate::DeviceIdSource::Authored` is a human
/// asserting the assignment, so a live unit contradicting either is a question.
/// `crate::DeviceIdSource::Synthesized` asserts nothing of the sort — a reporter that synthesizes a
/// key from location evidence has already decided what is stable about that hardware — so a
/// same-kind unit arriving where a synthesized key left is simply a different unit, and asking
/// about it would leave a rotating-serial device unusable while nobody could answer.
///
/// This is the only place that exemption is applied. The departed-slot join in
/// `crate::reconcile::verdict_for` reads the saved key's source only to tell `WrongUnit` from
/// `Displaced`, so it owes a human decision on a synthesized saved key exactly as it does on a
/// reported one; the caller answers that debt by discharging it, which restores the scanned
/// verdict. A synthesized replacement therefore ends the pass indistinguishable from a unit that
/// displaced nothing, and `tests/scripted.rs` pins the pair.
const fn answerable(saved: &DeviceKey) -> bool {
    match saved.id {
        DeviceIdSource::Reported { .. } | DeviceIdSource::Authored { .. } => true,
        DeviceIdSource::Synthesized { .. } => false,
    }
}

/// Read the saved key out of the verdict a human still owes an answer to.
///
/// Only the two join verdicts carry one. A duplicate key in one scan is an observation of that
/// scan, not a claim that some other unit replaced this one, so it raises no question and has no
/// saved side to name.
const fn saved_key(identity_verdict: &IdentityVerdict) -> Option<&DeviceKey> {
    match identity_verdict {
        IdentityVerdict::Displaced { saved } => Some(saved),
        IdentityVerdict::WrongUnit { authored } => Some(authored),
        _ => None,
    }
}

/// One standing question whose cached endpoint owner no longer says what `Bindings` says.
///
/// Carried out of the read as a list of changes rather than written in place, because reaching for
/// `bevy::ecs::system::ResMut<IdentityDecisions>` mutably is itself what marks the register
/// changed: a pass that re-derives the same owner for every standing question must write nothing.
struct EndpointOwnerRefresh {
    /// Position in `IdentityDecisions::questions` of the entry whose cached owner is stale.
    standing: usize,
    /// Which role holds the candidate endpoint as of this pass.
    owner:    EndpointOwner,
}

/// Find every standing question whose cached candidate-endpoint owner this pass changed.
///
/// Ownership moves while a question waits — another role can be registered against the same unit —
/// and the cached value is what `IdentityDecisions::answer` checks without access to `Bindings`.
fn stale_endpoint_owners(
    identity_decisions: &IdentityDecisions,
    bindings: &Bindings,
) -> Vec<EndpointOwnerRefresh> {
    identity_decisions
        .questions
        .iter()
        .enumerate()
        .filter_map(|(standing, question)| {
            let owner = bindings.candidate_endpoint_owner(&question.role, &question.candidate);
            (owner != question.owner).then_some(EndpointOwnerRefresh { standing, owner })
        })
        .collect()
}

/// Every settled candidate whose debt is still owed and whose question no standing entry holds
/// open.
///
/// Clearing that debt is what makes `crate::Devices` conclude the unit's verdict from its own key
/// instead of reporting the displacement forever, on this frame rather than the next scan's. A
/// candidate waits until nothing is outstanding
/// because one unit's debt is shared by every role that named the key it displaced, and an entry
/// already `IdentityDebtDischarge::Done` is skipped so a refusal that outlives its discharge does
/// not rewrite `Devices` on every later frame.
fn settled_discharges(identity_decisions: &IdentityDecisions) -> Vec<DeviceKey> {
    identity_decisions
        .settled
        .iter()
        .filter(|settled| settled.discharge == IdentityDebtDischarge::Owed)
        .map(|settled| settled.candidate.clone())
        .filter(|candidate| !identity_decisions.any_question_about(candidate))
        .collect()
}
