//! A reader token's outstanding-ticket registrations and owed open remain only while that exact
//! participation exists. Leaving prunes the token's entries; an owner promotion first redirects
//! them to the promoted token.

use std::hash::Hash;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use bevy::platform::collections::HashMap;

use crate::DeviceKey;
use crate::RoleKey;

/// The payload and dispatch identity a driver uses for one physical device open.
///
/// The payload stays driver-defined because rigging owns sharing policy without knowing whether
/// the device produces pictures, sound, controls, or some future streamed value. The ticket keeps
/// an asynchronous open tied to the reader tokens awaiting it without requiring the payload to
/// exist yet.
pub trait DeviceOpens: 'static {
    /// What one physical open of a device produces and every participating role shares.
    type Open: 'static;
    /// How the driver names an open it dispatched but has not received yet.
    type Ticket: Clone + Eq + Hash + 'static;
}

/// Membership in one shared physical open, counted independently from a role's identity.
///
/// Every call to [`SharedDeviceOpens::join`] issues a distinct token, and the open closes only when
/// its last token leaves. A role whose attempt and session records both hold tokens is therefore
/// two readers: releasing a predecessor cannot retire the successor's reading of the same open.
/// The fields and constructor stay private so only the record that tracks the membership can issue
/// one. Checked record and reader counters keep a cloned departed token distinct from every later
/// membership.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DeviceReaderToken {
    record:   RecordId,
    sequence: u64,
}

/// Per-record identity that makes foreign-record tokens unequal without retaining allocation
/// identities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct RecordId(u64);

static NEXT_RECORD_ID: AtomicU64 = AtomicU64::new(0);

/// What joining a device did and which token now owns that participation lifetime.
///
/// The split makes the only caller allowed to create hardware explicit: a subscriber receives the
/// existing owner's identity and cannot reach the open closure.
#[must_use = "the token is the only handle on this membership; dropping it leaks the reader"]
pub enum JoinedDeviceOpen<'a, Open> {
    /// This role created the device's one physical open and owns it initially.
    OpenedForThisRole {
        /// Membership token the caller must retain until this participation ends.
        token: DeviceReaderToken,
        /// Newly stored payload, available for driver-specific initialization in place.
        open:  &'a mut Open,
    },
    /// This role joined the physical open already owned by another participation.
    ReadsAnothersOpen {
        /// Membership token the caller must retain until this participation ends.
        token: DeviceReaderToken,
        /// Role whose participation currently owns the shared open.
        owner: RoleKey,
    },
}

/// How one participation token reads a shared physical open without being able to mutate it.
///
/// Ownership remains visible because driver retirement differs for the owner and a subscriber,
/// while `ReadsNothing` states absence rather than making callers infer it from an option.
pub enum SharedOpenReading<'a, Open> {
    /// The queried token's role currently owns this open.
    OwnedHere(&'a Open),
    /// The queried token's role reads an open owned by another role.
    ReadFrom {
        /// Current owner of the shared open.
        owner: &'a RoleKey,
        /// Payload shared by the owner and all subscribers.
        open:  &'a Open,
    },
    /// The queried token already left or was issued by another record.
    ReadsNothing,
}

/// How one participation token reaches a shared physical open for driver-specific mutation.
///
/// This is distinct from [`SharedOpenReading`] because Rust cannot abstract over reference
/// mutability, and a named absence arm keeps callers from treating a missing open as success.
pub enum SharedOpenReadingMut<'a, Open> {
    /// The queried token's role currently owns this open.
    OwnedHere(&'a mut Open),
    /// The queried token's role reads an open owned by another role.
    ReadFrom {
        /// Current owner of the shared open.
        owner: &'a RoleKey,
        /// Payload shared by the owner and all subscribers.
        open:  &'a mut Open,
    },
    /// The queried token already left or was issued by another record.
    ReadsNothing,
}

/// Whether a device has a physical open and which role currently owns it.
///
/// The named absence keeps a driver from inferring ownership through a separate reader lookup.
pub enum DeviceOpenOwner<'a> {
    /// The role whose participation currently owns the device open.
    Role(&'a RoleKey),
    /// No physical open is recorded for this device.
    NoOpen,
}

/// The role attached to one exact reader token.
///
/// Drivers claim asynchronous work by token, so this lookup recovers the role without letting a
/// newer participation under the same role receive an older participation's result.
pub enum TokenRole<'a> {
    /// Role whose exact participation the token names.
    Role(&'a RoleKey),
    /// The token already left or was issued by another record.
    NotAReader,
}

/// One token-and-role view yielded from a device's shared-open membership.
///
/// Returning both identities lets callers address an exact participation while still presenting
/// the role that owns the corresponding kernel slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceReader<'a> {
    /// Role participating through this reader token.
    pub role:  &'a RoleKey,
    /// Exact participation lifetime for this reader.
    pub token: &'a DeviceReaderToken,
}

/// What a checked durable-device-key change did to a physical open.
///
/// Refusing both missing sources and occupied destinations keeps the open, readers, and payload
/// under one unambiguous device identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReaddressedDeviceOpen {
    /// The whole open and every relationship attached through its readers moved to the new key.
    Moved,
    /// The source key had no open to move.
    NoOpenAt,
    /// The destination already had an open, so neither entry changed.
    AlreadyOpenAt,
}

/// Which reader tokens owned an asynchronous-open ticket when its result was claimed.
///
/// Claiming consumes the registration so a second delivery cannot be applied to the same
/// participation lifetimes.
pub enum AwaitedOpenClaim {
    /// Reader tokens awaiting this result, in their registration order.
    For(Vec<DeviceReaderToken>),
    /// This ticket has no outstanding registration in the record.
    Unclaimed,
}

/// Whether an asynchronous-open ticket was attached to a live reader token.
///
/// Rejecting departed and foreign tokens prevents a late result from being correlated through a
/// role name to a different participation lifetime.
#[must_use = "a departed or foreign token leaves the ticket unregistered"]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AwaitRegistration {
    /// The live token now awaits the ticket.
    Registered,
    /// The token already left or was issued by another record.
    NotAReader,
}

/// Whether owner promotion carried an asynchronous physical open still in flight.
///
/// The ticket lets the driver correlate the eventual result with the promoted token instead of
/// opening the device again while the inherited request is pending.
pub enum AwaitedOpen<Ticket> {
    /// The promoted token inherited this dispatched open, the earliest still outstanding when the
    /// retired owner had registrations under more than one ticket for this device.
    Awaiting(Ticket),
    /// The retired owner had no dispatched open for the promoted role to inherit.
    NoOpenInFlight,
}

/// Whether owner promotion carried a within-frame obligation to start a fresh physical open.
///
/// This state moves with the promoted token so the old participation cannot be restarted after
/// retirement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwedOpen {
    /// The promoted token now owes the fresh open.
    Owed,
    /// The retired owner had no fresh open queued.
    NotOwed,
}

/// Whether fresh-open debt was attached to a live reader token.
///
/// Refusing departed and foreign tokens keeps a cancelled participation from creating work for a
/// newer token that happens to carry the same role.
#[must_use = "a departed or foreign token leaves no fresh-open debt"]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OweRegistration {
    /// The live token now owes a fresh open for its device.
    Owed,
    /// The token already left or was issued by another record.
    NotAReader,
}

/// One device-correlated obligation to start a fresh physical open for a reader token.
///
/// Carrying both identities prevents a newer participation under the same role from paying a
/// cancelled participation's debt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwedOpenEntry {
    /// Device for which the driver must start a fresh physical open.
    pub device: DeviceKey,
    /// Exact participation that must receive the open.
    pub token:  DeviceReaderToken,
}

/// What consuming one reader token did to the shared physical open.
///
/// `ClosedLastReader` is the only answer that hands the non-cloneable payload back to the driver.
/// The result is `must_use` because dropping that arm would leak the physical open that only the
/// driver holds the handle to close.
#[must_use]
pub enum DeviceOpenRetirement<Open, Ticket> {
    /// The owner's last membership ended while subscribers remained, so the oldest remaining
    /// participation became owner without moving the payload out of the record.
    PromotedSubscriber {
        /// Role owning the longest-standing remaining reader token.
        promoted: RoleKey,
        /// Clone of the promoted reader's token, identifying the exact inherited lifetime.
        token:    DeviceReaderToken,
        /// Outstanding open request redirected from the retired owner, when one existed.
        awaited:  AwaitedOpen<Ticket>,
        /// Fresh-open debt redirected from the retired owner, when it existed.
        owed:     OwedOpen,
    },
    /// The final membership ended, handing the payload back for driver-specific closure.
    ClosedLastReader {
        /// The physical-open payload the driver must retire.
        open: Open,
    },
    /// One membership ended while the existing owner still had a token or a subscriber left.
    ReaderLeft,
    /// This record did not hold the consumed token, including a token that already left.
    NotAReader,
}

/// Owned role-to-token membership retained inside one shared-open record.
///
/// The record needs both values to resolve a token's role without making pending work role-keyed;
/// the public [`DeviceReader`] is only the borrowed view exposed while visiting this storage.
struct DeviceOpenMembership {
    token: DeviceReaderToken,
    role:  RoleKey,
}

struct DeviceOpenRecord<Open> {
    owner:   RoleKey,
    open:    Open,
    readers: Vec<DeviceOpenMembership>,
}

struct AwaitingDeviceOpen {
    device: DeviceKey,
    tokens: Vec<DeviceReaderToken>,
}

/// The driver-owned record that shares one physical open among every role reading a device.
///
/// It lives beside, rather than inside, the driver ledger: the ledger owns what the kernel issued
/// per role, while this record owns what the device has open across roles. Reader tokens keep those
/// independently keyed lifetimes from collapsing into one role entry, and no role entity is stored
/// because a kernel may replace that entity between driver calls.
pub struct SharedDeviceOpens<Opens: DeviceOpens> {
    record_id:         RecordId,
    next_reader:       u64,
    opens:             HashMap<DeviceKey, DeviceOpenRecord<Opens::Open>>,
    reader_devices:    HashMap<DeviceReaderToken, DeviceKey>,
    outstanding_opens: HashMap<Opens::Ticket, AwaitingDeviceOpen>,
    outstanding_order: Vec<Opens::Ticket>,
    owed_opens:        Vec<OwedOpenEntry>,
}

impl<Opens> SharedDeviceOpens<Opens>
where
    Opens: DeviceOpens,
{
    /// Create an empty record with no physical opens, readers, requests, or fresh-open debt.
    #[must_use]
    pub fn new() -> Self {
        Self {
            record_id:         issue_record_id(),
            next_reader:       0,
            opens:             HashMap::default(),
            reader_devices:    HashMap::default(),
            outstanding_opens: HashMap::default(),
            outstanding_order: Vec::new(),
            owed_opens:        Vec::new(),
        }
    }

    /// Join one role to a device's physical open, creating the payload only for the first reader.
    ///
    /// A role may join more than once because its attempt and session are separate participation
    /// lifetimes. Every call returns a new token for the caller to store in the corresponding
    /// ledger record.
    pub fn join(
        &mut self,
        device: &DeviceKey,
        role: RoleKey,
        open: impl FnOnce() -> Opens::Open,
    ) -> JoinedDeviceOpen<'_, Opens::Open> {
        let token = self.issue_reader_token();

        if let Some(record) = self.opens.get_mut(device) {
            let owner = record.owner.clone();
            record.readers.push(DeviceOpenMembership {
                token: token.clone(),
                role,
            });
            self.reader_devices.insert(token.clone(), device.clone());
            return JoinedDeviceOpen::ReadsAnothersOpen { token, owner };
        }

        let record = self
            .opens
            .entry(device.clone())
            .or_insert_with(|| DeviceOpenRecord {
                owner:   role.clone(),
                open:    open(),
                readers: vec![DeviceOpenMembership {
                    token: token.clone(),
                    role,
                }],
            });
        self.reader_devices.insert(token.clone(), device.clone());

        JoinedDeviceOpen::OpenedForThisRole {
            token,
            open: &mut record.open,
        }
    }

    /// Read the exact physical open named by a participation token and state its ownership.
    ///
    /// A role may hold tokens for different devices in its attempt and session slots, so the token
    /// rather than the role is the only unambiguous lookup key.
    #[must_use]
    pub fn open_of(&self, token: &DeviceReaderToken) -> SharedOpenReading<'_, Opens::Open> {
        let Some(device) = self.reader_devices.get(token) else {
            return SharedOpenReading::ReadsNothing;
        };
        let Some(record) = self.opens.get(device) else {
            return SharedOpenReading::ReadsNothing;
        };
        let Some(reader) = record.readers.iter().find(|reader| &reader.token == token) else {
            return SharedOpenReading::ReadsNothing;
        };

        if record.owner == reader.role {
            return SharedOpenReading::OwnedHere(&record.open);
        }

        SharedOpenReading::ReadFrom {
            owner: &record.owner,
            open:  &record.open,
        }
    }

    /// Mutably read the exact physical open named by a token and state its ownership.
    ///
    /// `ReadsNothing` rejects both a departed token and one issued by another record.
    #[must_use]
    pub fn open_of_mut(
        &mut self,
        token: &DeviceReaderToken,
    ) -> SharedOpenReadingMut<'_, Opens::Open> {
        let Some(device) = self.reader_devices.get(token) else {
            return SharedOpenReadingMut::ReadsNothing;
        };
        let Some(record) = self.opens.get_mut(device) else {
            return SharedOpenReadingMut::ReadsNothing;
        };
        let Some(reader) = record.readers.iter().find(|reader| &reader.token == token) else {
            return SharedOpenReadingMut::ReadsNothing;
        };

        if record.owner == reader.role {
            return SharedOpenReadingMut::OwnedHere(&mut record.open);
        }

        SharedOpenReadingMut::ReadFrom {
            owner: &record.owner,
            open:  &mut record.open,
        }
    }

    /// Name the role that currently owns a device's physical open.
    #[must_use]
    pub fn owner_of(&self, device: &DeviceKey) -> DeviceOpenOwner<'_> {
        match self.opens.get(device) {
            Some(record) => DeviceOpenOwner::Role(&record.owner),
            None => DeviceOpenOwner::NoOpen,
        }
    }

    /// Find the role attached to one exact participation token.
    #[must_use]
    pub fn role_of(&self, token: &DeviceReaderToken) -> TokenRole<'_> {
        let Some(device) = self.reader_devices.get(token) else {
            return TokenRole::NotAReader;
        };
        let Some(record) = self.opens.get(device) else {
            return TokenRole::NotAReader;
        };
        match record.readers.iter().find(|reader| &reader.token == token) {
            Some(reader) => TokenRole::Role(&reader.role),
            None => TokenRole::NotAReader,
        }
    }

    /// Visit every reader's role and exact token in join order, including repeated roles.
    pub fn readers_of(&self, device: &DeviceKey) -> impl Iterator<Item = DeviceReader<'_>> {
        let readers = match self.opens.get(device) {
            Some(record) => record.readers.as_slice(),
            None => &[],
        };
        readers.iter().map(|reader| DeviceReader {
            role:  &reader.role,
            token: &reader.token,
        })
    }

    /// Visit every physical open exactly once, independent of its reader count.
    pub fn opens(&self) -> impl Iterator<Item = (&DeviceKey, &Opens::Open)> {
        self.opens
            .iter()
            .map(|(device, record)| (device, &record.open))
    }

    /// Mutably visit every physical open exactly once, independent of its reader count.
    pub fn opens_mut(&mut self) -> impl Iterator<Item = (&DeviceKey, &mut Opens::Open)> {
        self.opens
            .iter_mut()
            .map(|(device, record)| (device, &mut record.open))
    }

    /// Move one device's complete shared-open identity to a learned durable key.
    ///
    /// Reader-token indexes, outstanding-ticket device tags, and fresh-open debt are rewritten
    /// with the key, so the relationships move without role inference or cloning the physical
    /// payload.
    pub fn readdress(&mut self, from: &DeviceKey, to: DeviceKey) -> ReaddressedDeviceOpen {
        if !self.opens.contains_key(from) {
            return ReaddressedDeviceOpen::NoOpenAt;
        }
        if self.opens.contains_key(&to) {
            return ReaddressedDeviceOpen::AlreadyOpenAt;
        }

        let Some(record) = self.opens.remove(from) else {
            return ReaddressedDeviceOpen::NoOpenAt;
        };
        for reader in &record.readers {
            self.reader_devices.insert(reader.token.clone(), to.clone());
        }
        for awaited in self.outstanding_opens.values_mut() {
            if &awaited.device == from {
                awaited.device.clone_from(&to);
            }
        }
        for owed in &mut self.owed_opens {
            if &owed.device == from {
                owed.device.clone_from(&to);
            }
        }
        self.remove_duplicate_owed_opens();
        self.opens.insert(to, record);

        ReaddressedDeviceOpen::Moved
    }

    /// Register one more reader token awaiting a driver's asynchronous physical-open ticket.
    ///
    /// The target device comes from the live token, so a role's older and newer participation
    /// lifetimes cannot receive one another's result.
    ///
    /// # Panics
    ///
    /// Panics when the same ticket is registered for a different device. One ticket names one
    /// dispatched physical open, so accepting two target devices would corrupt later claims.
    pub fn await_open(
        &mut self,
        ticket: Opens::Ticket,
        token: &DeviceReaderToken,
    ) -> AwaitRegistration {
        let Some(device) = self.reader_devices.get(token) else {
            return AwaitRegistration::NotAReader;
        };
        let Some(record) = self.opens.get(device) else {
            return AwaitRegistration::NotAReader;
        };
        if !record.readers.iter().any(|reader| &reader.token == token) {
            return AwaitRegistration::NotAReader;
        }
        let device = device.clone();

        if let Some(awaiting) = self.outstanding_opens.get_mut(&ticket) {
            assert_eq!(
                awaiting.device, device,
                "one physical-open ticket cannot target two devices"
            );
            awaiting.tokens.push(token.clone());
            return AwaitRegistration::Registered;
        }

        self.outstanding_order.push(ticket.clone());
        self.outstanding_opens.insert(
            ticket,
            AwaitingDeviceOpen {
                device,
                tokens: vec![token.clone()],
            },
        );
        AwaitRegistration::Registered
    }

    /// Consume an asynchronous-open ticket and return every awaiting token in registration order.
    #[must_use]
    pub fn claim_open(&mut self, ticket: &Opens::Ticket) -> AwaitedOpenClaim {
        match self.outstanding_opens.remove(ticket) {
            Some(awaiting) => {
                self.outstanding_order
                    .retain(|outstanding| outstanding != ticket);
                AwaitedOpenClaim::For(awaiting.tokens)
            },
            None => AwaitedOpenClaim::Unclaimed,
        }
    }

    /// Record that a reader token must start a fresh physical open before this frame ends.
    pub fn owe_open(&mut self, token: &DeviceReaderToken) -> OweRegistration {
        let Some(device) = self.reader_devices.get(token) else {
            return OweRegistration::NotAReader;
        };
        let Some(record) = self.opens.get(device) else {
            return OweRegistration::NotAReader;
        };
        if !record.readers.iter().any(|reader| &reader.token == token) {
            return OweRegistration::NotAReader;
        }

        if !self
            .owed_opens
            .iter()
            .any(|owed| &owed.device == device && &owed.token == token)
        {
            self.owed_opens.push(OwedOpenEntry {
                device: device.clone(),
                token:  token.clone(),
            });
        }
        OweRegistration::Owed
    }

    /// Visit the within-frame fresh-open queue without changing its owed order or draining it.
    pub fn owed_opens(&self) -> impl Iterator<Item = &OwedOpenEntry> { self.owed_opens.iter() }

    /// Drain the within-frame fresh-open queue in the order its device-token pairs became owed.
    #[must_use]
    pub fn take_owed_opens(&mut self) -> Vec<OwedOpenEntry> { std::mem::take(&mut self.owed_opens) }

    /// End one participation and either retain, promote, or hand back the physical open.
    ///
    /// Promotion selects the longest-standing remaining token. It changes ownership in place and
    /// redirects only the retiring token's pending-ticket registrations and fresh-open debt onto
    /// the promoted token, without ever moving the non-cloneable payload out of the record.
    pub fn leave(
        &mut self,
        token: DeviceReaderToken,
    ) -> DeviceOpenRetirement<Opens::Open, Opens::Ticket> {
        let Some(device) = self.reader_devices.remove(&token) else {
            return DeviceOpenRetirement::NotAReader;
        };
        let Some(record) = self.opens.get_mut(&device) else {
            return DeviceOpenRetirement::NotAReader;
        };
        let Some(reader_index) = record
            .readers
            .iter()
            .position(|reader| reader.token == token)
        else {
            return DeviceOpenRetirement::NotAReader;
        };

        let leaving_role = record.readers.remove(reader_index).role;
        let leaving_role_still_reads = record
            .readers
            .iter()
            .any(|reader| reader.role == leaving_role);
        if record.readers.is_empty() {
            self.abandon_token_work(&device, &token);
            let Some(record) = self.opens.remove(&device) else {
                return DeviceOpenRetirement::NotAReader;
            };
            return DeviceOpenRetirement::ClosedLastReader { open: record.open };
        }

        if leaving_role_still_reads {
            self.abandon_token_work(&device, &token);
            return DeviceOpenRetirement::ReaderLeft;
        }

        if record.owner != leaving_role {
            self.abandon_token_work(&device, &token);
            return DeviceOpenRetirement::ReaderLeft;
        }

        let promoted = record.readers[0].role.clone();
        let promoted_token = record.readers[0].token.clone();
        record.owner = promoted.clone();

        let awaited = self.redirect_awaited_opens(&device, &token, &promoted_token);
        let owed = self.redirect_owed_open(&device, &token, &promoted_token);
        self.abandon_token_work(&device, &token);

        DeviceOpenRetirement::PromotedSubscriber {
            promoted,
            token: promoted_token,
            awaited,
            owed,
        }
    }

    const fn issue_reader_token(&mut self) -> DeviceReaderToken {
        let sequence = self.next_reader;
        let next_reader = self.next_reader.checked_add(1);
        assert!(
            next_reader.is_some(),
            "device reader token sequence exhausted"
        );
        self.next_reader = match next_reader {
            Some(next_reader) => next_reader,
            None => self.next_reader,
        };
        DeviceReaderToken {
            record: self.record_id,
            sequence,
        }
    }

    fn redirect_awaited_opens(
        &mut self,
        device: &DeviceKey,
        from: &DeviceReaderToken,
        to: &DeviceReaderToken,
    ) -> AwaitedOpen<Opens::Ticket> {
        let mut inherited = AwaitedOpen::NoOpenInFlight;

        for ticket in &self.outstanding_order {
            let Some(awaiting) = self.outstanding_opens.get_mut(ticket) else {
                continue;
            };
            if &awaiting.device != device {
                continue;
            }
            let promoted_already_awaits = awaiting.tokens.iter().any(|token| token == to);
            let mut promoted_registered = promoted_already_awaits;
            let mut redirected_ticket = false;
            awaiting.tokens.retain_mut(|token| {
                if token != from {
                    return true;
                }
                redirected_ticket = true;
                if promoted_registered {
                    return false;
                }
                token.clone_from(to);
                promoted_registered = true;
                true
            });
            if redirected_ticket && matches!(inherited, AwaitedOpen::NoOpenInFlight) {
                inherited = AwaitedOpen::Awaiting(ticket.clone());
            }
        }

        inherited
    }

    fn redirect_owed_open(
        &mut self,
        device: &DeviceKey,
        from: &DeviceReaderToken,
        to: &DeviceReaderToken,
    ) -> OwedOpen {
        let mut redirected = OwedOpen::NotOwed;
        for owed in &mut self.owed_opens {
            if &owed.device == device && &owed.token == from {
                owed.token.clone_from(to);
                redirected = OwedOpen::Owed;
            }
        }
        if redirected == OwedOpen::Owed {
            let mut kept_promoted = false;
            self.owed_opens.retain(|owed| {
                if &owed.device != device || &owed.token != to {
                    return true;
                }
                if kept_promoted {
                    return false;
                }
                kept_promoted = true;
                true
            });
        }
        redirected
    }

    fn abandon_token_work(&mut self, device: &DeviceKey, token: &DeviceReaderToken) {
        for awaiting in self.outstanding_opens.values_mut() {
            if &awaiting.device == device {
                awaiting
                    .tokens
                    .retain(|awaiting_token| awaiting_token != token);
            }
        }
        self.outstanding_opens
            .retain(|_, awaiting| !awaiting.tokens.is_empty());
        let outstanding_opens = &self.outstanding_opens;
        self.outstanding_order
            .retain(|ticket| outstanding_opens.contains_key(ticket));
        self.owed_opens
            .retain(|owed| &owed.device != device || &owed.token != token);
    }

    fn remove_duplicate_owed_opens(&mut self) {
        let mut unique_owed_opens = Vec::with_capacity(self.owed_opens.len());
        for owed in std::mem::take(&mut self.owed_opens) {
            if !unique_owed_opens.contains(&owed) {
                unique_owed_opens.push(owed);
            }
        }
        self.owed_opens = unique_owed_opens;
    }
}

fn issue_record_id() -> RecordId {
    let mut record_id = NEXT_RECORD_ID.load(Ordering::Relaxed);
    loop {
        let next_record_id = record_id.checked_add(1);
        assert!(
            next_record_id.is_some(),
            "shared device-open record identity exhausted"
        );
        let next_record_id = next_record_id.unwrap_or(record_id);
        match NEXT_RECORD_ID.compare_exchange_weak(
            record_id,
            next_record_id,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return RecordId(record_id),
            Err(observed_record_id) => record_id = observed_record_id,
        }
    }
}

impl<Opens> Default for SharedDeviceOpens<Opens>
where
    Opens: DeviceOpens,
{
    fn default() -> Self { Self::new() }
}
