//! Public behavior coverage for sharing one physical device open across role lifetimes.
//!
//! `ToneOpen` has no device-specific media type and does not implement `Clone`. The tests therefore
//! exercise ownership, shared reading, promotion, and retirement through the record's public
//! tokens and outcome enums.

use std::cell::Cell;
use std::error::Error;
use std::io::Error as IoError;

use hana_rigging::prelude::AwaitRegistration;
use hana_rigging::prelude::AwaitedOpen;
use hana_rigging::prelude::AwaitedOpenClaim;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::DeviceKind;
use hana_rigging::prelude::DeviceOpenOwner;
use hana_rigging::prelude::DeviceOpenRetirement;
use hana_rigging::prelude::DeviceOpens;
use hana_rigging::prelude::DeviceReader;
use hana_rigging::prelude::DeviceReaderToken;
use hana_rigging::prelude::JoinedDeviceOpen;
use hana_rigging::prelude::OweRegistration;
use hana_rigging::prelude::OwedOpen;
use hana_rigging::prelude::OwedOpenEntry;
use hana_rigging::prelude::ReaddressedDeviceOpen;
use hana_rigging::prelude::ReportedId;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::SchemeName;
use hana_rigging::prelude::SharedDeviceOpens;
use hana_rigging::prelude::SharedOpenReading;
use hana_rigging::prelude::SharedOpenReadingMut;
use hana_rigging::prelude::TokenRole;

const TONE_SCHEME: &str = "tone-device";
const OWNER_HZ: u32 = 440;
const OWNER_SAMPLES: &[i16] = &[1, -2, 3];
const OTHER_HZ: u32 = 880;
const OTHER_SAMPLES: &[i16] = &[5, -8];
const PROMOTED_SAMPLE: i16 = 13;
const WALK_SAMPLE: i16 = 21;
const PROMOTION_TICKET: u32 = 34;
const SHARED_TICKET: u32 = 55;
const READDRESS_TICKET: u32 = 89;
const CROSS_DEVICE_TICKET: u32 = 144;
const ABANDONED_TICKET: u32 = 233;
const RETAINED_TICKET: u32 = 377;
const CLOSED_TICKET: u32 = 610;
const DUPLICATE_PROMOTION_TICKET: u32 = 987;

#[derive(Debug, PartialEq, Eq)]
struct ToneOpen {
    hz:      u32,
    samples: Vec<i16>,
}

struct ToneOpens;

impl DeviceOpens for ToneOpens {
    type Open = ToneOpen;
    type Ticket = u32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenClosureCall {
    NotCalled,
    Called,
}

fn tone_open(hz: u32, samples: &[i16]) -> ToneOpen {
    ToneOpen {
        hz,
        samples: samples.to_vec(),
    }
}

fn device(value: &str) -> Result<DeviceKey, Box<dyn Error>> {
    Ok(DeviceKey::reported(
        DeviceKind::AudioInterface,
        SchemeName::new(TONE_SCHEME)?,
        ReportedId::new(value)?,
    ))
}

fn wrong_outcome(expected: &'static str) -> Box<dyn Error> {
    IoError::other(format!("expected {expected}")).into()
}

const fn reader_identity(reader: DeviceReader<'_>) -> (&RoleKey, &DeviceReaderToken) {
    (reader.role, reader.token)
}

fn join_owner(
    device_opens: &mut SharedDeviceOpens<ToneOpens>,
    device: &DeviceKey,
    role: &RoleKey,
    hz: u32,
    samples: &[i16],
) -> Result<DeviceReaderToken, Box<dyn Error>> {
    let JoinedDeviceOpen::OpenedForThisRole { token, open } =
        device_opens.join(device, role.clone(), || tone_open(hz, samples))
    else {
        return Err(wrong_outcome("OpenedForThisRole"));
    };
    assert_eq!(open.hz, hz);
    assert_eq!(open.samples.as_slice(), samples);
    Ok(token)
}

fn join_reader(
    device_opens: &mut SharedDeviceOpens<ToneOpens>,
    device: &DeviceKey,
    role: &RoleKey,
    expected_owner: &RoleKey,
) -> Result<DeviceReaderToken, Box<dyn Error>> {
    let JoinedDeviceOpen::ReadsAnothersOpen { token, owner } =
        device_opens.join(device, role.clone(), || tone_open(OTHER_HZ, OTHER_SAMPLES))
    else {
        return Err(wrong_outcome("ReadsAnothersOpen"));
    };
    assert_eq!(&owner, expected_owner);
    Ok(token)
}

#[test]
fn a_second_role_joining_one_device_never_runs_the_open_closure_and_reads_the_owners_payload()
-> Result<(), Box<dyn Error>> {
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let device = device("shared-tone")?;
    let owner = RoleKey::new("tone/owner")?;
    let reader = RoleKey::new("tone/reader")?;
    let owner_token = join_owner(&mut device_opens, &device, &owner, OWNER_HZ, OWNER_SAMPLES)?;
    let closure_call = Cell::new(OpenClosureCall::NotCalled);

    let JoinedDeviceOpen::ReadsAnothersOpen {
        token: reader_token,
        owner: recorded_owner,
    } = device_opens.join(&device, reader.clone(), || {
        closure_call.set(OpenClosureCall::Called);
        tone_open(OTHER_HZ, OTHER_SAMPLES)
    })
    else {
        return Err(wrong_outcome("ReadsAnothersOpen"));
    };

    assert_eq!(closure_call.get(), OpenClosureCall::NotCalled);
    assert_eq!(recorded_owner, owner);
    assert_ne!(owner_token, reader_token);
    let SharedOpenReading::ReadFrom {
        owner: recorded_owner,
        open,
    } = device_opens.open_of(&reader_token)
    else {
        return Err(wrong_outcome("ReadFrom"));
    };
    assert_eq!(recorded_owner, &owner);
    assert_eq!(open, &tone_open(OWNER_HZ, OWNER_SAMPLES));
    let SharedOpenReadingMut::ReadFrom {
        owner: recorded_owner,
        open,
    } = device_opens.open_of_mut(&reader_token)
    else {
        return Err(wrong_outcome("ReadFrom"));
    };
    assert_eq!(recorded_owner, &owner);
    assert_eq!(open, &tone_open(OWNER_HZ, OWNER_SAMPLES));
    assert_eq!(
        device_opens
            .readers_of(&device)
            .map(reader_identity)
            .collect::<Vec<_>>(),
        vec![(&owner, &owner_token), (&reader, &reader_token)]
    );
    Ok(())
}

#[test]
fn a_role_holding_two_tokens_stays_a_reader_until_both_leave() -> Result<(), Box<dyn Error>> {
    let mut device_opens: SharedDeviceOpens<ToneOpens> = SharedDeviceOpens::default();
    let device = device("two-token-tone")?;
    let owner = RoleKey::new("tone/two-token-owner")?;
    let first_token = join_owner(&mut device_opens, &device, &owner, OWNER_HZ, OWNER_SAMPLES)?;
    let second_token = join_reader(&mut device_opens, &device, &owner, &owner)?;

    assert!(matches!(
        device_opens.leave(first_token),
        DeviceOpenRetirement::ReaderLeft
    ));
    let DeviceOpenOwner::Role(recorded_owner) = device_opens.owner_of(&device) else {
        return Err(wrong_outcome("Role"));
    };
    assert_eq!(recorded_owner, &owner);
    assert_eq!(
        device_opens
            .readers_of(&device)
            .map(reader_identity)
            .collect::<Vec<_>>(),
        vec![(&owner, &second_token)]
    );

    let DeviceOpenRetirement::ClosedLastReader { open } = device_opens.leave(second_token) else {
        return Err(wrong_outcome("ClosedLastReader"));
    };
    assert_eq!(open, tone_open(OWNER_HZ, OWNER_SAMPLES));
    assert!(matches!(
        device_opens.owner_of(&device),
        DeviceOpenOwner::NoOpen
    ));
    Ok(())
}

#[test]
fn the_owners_last_token_leaving_promotes_the_longest_standing_reader_and_keeps_the_open_in_the_record()
-> Result<(), Box<dyn Error>> {
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let device = device("promoted-tone")?;
    let owner = RoleKey::new("tone/retiring-owner")?;
    let first_reader = RoleKey::new("tone/first-reader")?;
    let second_reader = RoleKey::new("tone/second-reader")?;
    let owner_token = join_owner(&mut device_opens, &device, &owner, OWNER_HZ, OWNER_SAMPLES)?;
    let first_reader_token = join_reader(&mut device_opens, &device, &first_reader, &owner)?;
    let second_reader_token = join_reader(&mut device_opens, &device, &second_reader, &owner)?;
    assert_ne!(first_reader_token, second_reader_token);

    let DeviceOpenRetirement::PromotedSubscriber {
        promoted,
        token: promoted_token,
        awaited,
        owed,
    } = device_opens.leave(owner_token)
    else {
        return Err(wrong_outcome("PromotedSubscriber"));
    };
    assert_eq!(promoted, first_reader);
    assert_eq!(promoted_token, first_reader_token);
    assert!(matches!(awaited, AwaitedOpen::NoOpenInFlight));
    assert!(matches!(owed, OwedOpen::NotOwed));
    let DeviceOpenOwner::Role(recorded_owner) = device_opens.owner_of(&device) else {
        return Err(wrong_outcome("Role"));
    };
    assert_eq!(recorded_owner, &first_reader);

    let SharedOpenReadingMut::OwnedHere(open) = device_opens.open_of_mut(&promoted_token) else {
        return Err(wrong_outcome("OwnedHere"));
    };
    assert_eq!(open, &tone_open(OWNER_HZ, OWNER_SAMPLES));
    open.samples.push(PROMOTED_SAMPLE);

    let SharedOpenReading::ReadFrom {
        owner: recorded_owner,
        open,
    } = device_opens.open_of(&second_reader_token)
    else {
        return Err(wrong_outcome("ReadFrom"));
    };
    assert_eq!(recorded_owner, &first_reader);
    assert_eq!(open.samples.last(), Some(&PROMOTED_SAMPLE));
    Ok(())
}

#[test]
fn the_last_token_leaving_answers_closed_last_reader_with_the_payload() -> Result<(), Box<dyn Error>>
{
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let device = device("last-reader-tone")?;
    let owner = RoleKey::new("tone/last-reader")?;
    let token = join_owner(&mut device_opens, &device, &owner, OWNER_HZ, OWNER_SAMPLES)?;
    let departed_token = token.clone();

    let DeviceOpenRetirement::ClosedLastReader { open } = device_opens.leave(token) else {
        return Err(wrong_outcome("ClosedLastReader"));
    };
    assert_eq!(open, tone_open(OWNER_HZ, OWNER_SAMPLES));
    assert!(matches!(
        device_opens.open_of(&departed_token),
        SharedOpenReading::ReadsNothing
    ));
    assert!(matches!(
        device_opens.open_of_mut(&departed_token),
        SharedOpenReadingMut::ReadsNothing
    ));
    assert!(matches!(
        device_opens.owner_of(&device),
        DeviceOpenOwner::NoOpen
    ));
    Ok(())
}

#[test]
fn a_token_that_already_left_answers_not_a_reader() -> Result<(), Box<dyn Error>> {
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let shared_device = device("departed-reader-tone")?;
    let owner = RoleKey::new("tone/staying-owner")?;
    let reader = RoleKey::new("tone/departing-reader")?;
    let owner_token = join_owner(
        &mut device_opens,
        &shared_device,
        &owner,
        OWNER_HZ,
        OWNER_SAMPLES,
    )?;
    let reader_token = join_reader(&mut device_opens, &shared_device, &reader, &owner)?;
    let already_left = reader_token.clone();
    let departed_reader_token = reader_token.clone();

    assert!(matches!(
        device_opens.leave(reader_token),
        DeviceOpenRetirement::ReaderLeft
    ));
    assert!(matches!(
        device_opens.leave(already_left),
        DeviceOpenRetirement::NotAReader
    ));
    assert!(matches!(
        device_opens.open_of(&departed_reader_token),
        SharedOpenReading::ReadsNothing
    ));
    assert!(matches!(
        device_opens.await_open(ABANDONED_TICKET, &departed_reader_token),
        AwaitRegistration::NotAReader
    ));
    assert!(matches!(
        device_opens.owe_open(&departed_reader_token),
        OweRegistration::NotAReader
    ));

    let mut foreign_device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let foreign_device = device("foreign-token-tone")?;
    let foreign_owner = RoleKey::new("tone/foreign-owner")?;
    let foreign_token = join_owner(
        &mut foreign_device_opens,
        &foreign_device,
        &foreign_owner,
        OTHER_HZ,
        OTHER_SAMPLES,
    )?;
    let foreign_lookup_token = foreign_token.clone();
    assert!(matches!(
        device_opens.open_of(&foreign_lookup_token),
        SharedOpenReading::ReadsNothing
    ));
    assert!(matches!(
        device_opens.leave(foreign_token),
        DeviceOpenRetirement::NotAReader
    ));
    let DeviceOpenOwner::Role(recorded_owner) = device_opens.owner_of(&shared_device) else {
        return Err(wrong_outcome("Role"));
    };
    assert_eq!(recorded_owner, &owner);
    assert!(matches!(
        device_opens.leave(owner_token),
        DeviceOpenRetirement::ClosedLastReader { .. }
    ));
    Ok(())
}

#[test]
fn an_awaited_ticket_and_an_owed_open_follow_the_promotion() -> Result<(), Box<dyn Error>> {
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let device = device("pending-promotion-tone")?;
    let owner = RoleKey::new("tone/pending-owner")?;
    let subscriber = RoleKey::new("tone/promoted-subscriber")?;
    let owner_token = join_owner(&mut device_opens, &device, &owner, OWNER_HZ, OWNER_SAMPLES)?;
    let subscriber_token = join_reader(&mut device_opens, &device, &subscriber, &owner)?;
    assert!(matches!(
        device_opens.await_open(PROMOTION_TICKET, &owner_token),
        AwaitRegistration::Registered
    ));
    assert!(matches!(
        device_opens.owe_open(&owner_token),
        OweRegistration::Owed
    ));

    let DeviceOpenRetirement::PromotedSubscriber {
        promoted,
        token: promoted_token,
        awaited,
        owed,
    } = device_opens.leave(owner_token)
    else {
        return Err(wrong_outcome("PromotedSubscriber"));
    };
    assert_eq!(promoted, subscriber);
    assert_eq!(promoted_token, subscriber_token);
    let AwaitedOpen::Awaiting(ticket) = awaited else {
        return Err(wrong_outcome("Awaiting"));
    };
    assert_eq!(ticket, PROMOTION_TICKET);
    assert!(matches!(owed, OwedOpen::Owed));

    let AwaitedOpenClaim::For(awaiting_tokens) = device_opens.claim_open(&PROMOTION_TICKET) else {
        return Err(wrong_outcome("For"));
    };
    assert_eq!(awaiting_tokens, vec![subscriber_token.clone()]);
    let owed_opens = device_opens.take_owed_opens();
    let [
        OwedOpenEntry {
            device: owed_device,
            token: owed_token,
        },
    ] = owed_opens.as_slice()
    else {
        return Err(wrong_outcome("one OwedOpenEntry"));
    };
    assert_eq!(owed_device, &device);
    assert_eq!(owed_token, &subscriber_token);
    Ok(())
}

#[test]
fn claim_open_names_every_token_awaiting_one_ticket_and_then_answers_unclaimed()
-> Result<(), Box<dyn Error>> {
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let awaited_device = device("shared-ticket-tone")?;
    let first = RoleKey::new("tone/first-awaiting-role")?;
    let second = RoleKey::new("tone/second-awaiting-role")?;
    let third = RoleKey::new("tone/third-awaiting-role")?;
    let first_token = join_owner(
        &mut device_opens,
        &awaited_device,
        &first,
        OWNER_HZ,
        OWNER_SAMPLES,
    )?;
    let second_token = join_reader(&mut device_opens, &awaited_device, &second, &first)?;
    let third_token = join_reader(&mut device_opens, &awaited_device, &third, &first)?;
    assert!(matches!(
        device_opens.await_open(SHARED_TICKET, &first_token),
        AwaitRegistration::Registered
    ));
    assert!(matches!(
        device_opens.await_open(SHARED_TICKET, &second_token),
        AwaitRegistration::Registered
    ));
    assert!(matches!(
        device_opens.await_open(SHARED_TICKET, &third_token),
        AwaitRegistration::Registered
    ));

    let AwaitedOpenClaim::For(awaiting_tokens) = device_opens.claim_open(&SHARED_TICKET) else {
        return Err(wrong_outcome("For"));
    };
    assert_eq!(
        awaiting_tokens,
        vec![first_token, second_token, third_token]
    );
    assert!(matches!(
        device_opens.claim_open(&SHARED_TICKET),
        AwaitedOpenClaim::Unclaimed
    ));
    Ok(())
}

#[test]
fn a_token_leaving_abandons_its_ticket_and_debt_while_other_tokens_remain()
-> Result<(), Box<dyn Error>> {
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let shared_device = device("abandoned-pending-tone")?;
    let owner = RoleKey::new("tone/pending-owner-with-two-tokens")?;
    let subscriber = RoleKey::new("tone/departing-pending-subscriber")?;
    let first_owner_token = join_owner(
        &mut device_opens,
        &shared_device,
        &owner,
        OWNER_HZ,
        OWNER_SAMPLES,
    )?;
    let second_owner_token = join_reader(&mut device_opens, &shared_device, &owner, &owner)?;
    let subscriber_token = join_reader(&mut device_opens, &shared_device, &subscriber, &owner)?;
    assert!(matches!(
        device_opens.await_open(ABANDONED_TICKET, &subscriber_token),
        AwaitRegistration::Registered
    ));
    assert!(matches!(
        device_opens.owe_open(&subscriber_token),
        OweRegistration::Owed
    ));

    assert!(matches!(
        device_opens.leave(subscriber_token),
        DeviceOpenRetirement::ReaderLeft
    ));
    assert!(matches!(
        device_opens.claim_open(&ABANDONED_TICKET),
        AwaitedOpenClaim::Unclaimed
    ));
    assert!(device_opens.owed_opens().next().is_none());

    assert!(matches!(
        device_opens.await_open(RETAINED_TICKET, &first_owner_token),
        AwaitRegistration::Registered
    ));
    assert!(matches!(
        device_opens.owe_open(&first_owner_token),
        OweRegistration::Owed
    ));
    assert!(matches!(
        device_opens.leave(first_owner_token),
        DeviceOpenRetirement::ReaderLeft
    ));
    assert!(matches!(
        device_opens.claim_open(&RETAINED_TICKET),
        AwaitedOpenClaim::Unclaimed
    ));
    assert!(device_opens.owed_opens().next().is_none());

    assert!(matches!(
        device_opens.await_open(CLOSED_TICKET, &second_owner_token),
        AwaitRegistration::Registered
    ));
    assert!(matches!(
        device_opens.owe_open(&second_owner_token),
        OweRegistration::Owed
    ));
    let DeviceOpenRetirement::ClosedLastReader { open } = device_opens.leave(second_owner_token)
    else {
        return Err(wrong_outcome("ClosedLastReader"));
    };
    assert_eq!(open, tone_open(OWNER_HZ, OWNER_SAMPLES));
    assert!(matches!(
        device_opens.claim_open(&CLOSED_TICKET),
        AwaitedOpenClaim::Unclaimed
    ));
    assert!(device_opens.owed_opens().next().is_none());
    Ok(())
}

#[test]
fn owed_opens_reads_the_queue_without_draining_it() -> Result<(), Box<dyn Error>> {
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let first_device = device("first-owed-tone")?;
    let second_device = device("second-owed-tone")?;
    let first_role = RoleKey::new("tone/first-owed-role")?;
    let second_role = RoleKey::new("tone/second-owed-role")?;
    let first_token = join_owner(
        &mut device_opens,
        &first_device,
        &first_role,
        OWNER_HZ,
        OWNER_SAMPLES,
    )?;
    let second_token = join_owner(
        &mut device_opens,
        &second_device,
        &second_role,
        OTHER_HZ,
        OTHER_SAMPLES,
    )?;
    let expected = vec![
        OwedOpenEntry {
            device: first_device.clone(),
            token:  first_token.clone(),
        },
        OwedOpenEntry {
            device: second_device.clone(),
            token:  second_token.clone(),
        },
    ];
    assert!(matches!(
        device_opens.owe_open(&first_token),
        OweRegistration::Owed
    ));
    assert!(matches!(
        device_opens.owe_open(&second_token),
        OweRegistration::Owed
    ));

    let before_reads = device_opens.owed_opens().cloned().collect::<Vec<_>>();
    assert_eq!(before_reads, expected);
    let first_read = device_opens.owed_opens().cloned().collect::<Vec<_>>();
    let second_read = device_opens.owed_opens().cloned().collect::<Vec<_>>();
    assert_eq!(first_read, before_reads);
    assert_eq!(second_read, before_reads);
    let after_reads = device_opens.owed_opens().cloned().collect::<Vec<_>>();
    assert_eq!(after_reads, before_reads);
    assert_eq!(device_opens.take_owed_opens(), before_reads);
    assert!(device_opens.owed_opens().next().is_none());
    Ok(())
}

#[test]
fn a_promotion_never_registers_the_promoted_role_twice_on_one_ticket() -> Result<(), Box<dyn Error>>
{
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let shared_device = device("deduplicated-promotion-tone")?;
    let owner = RoleKey::new("tone/deduplicated-retiring-owner")?;
    let subscriber = RoleKey::new("tone/deduplicated-promoted-subscriber")?;
    let other_waiter = RoleKey::new("tone/deduplicated-other-waiter")?;
    let owner_token = join_owner(
        &mut device_opens,
        &shared_device,
        &owner,
        OWNER_HZ,
        OWNER_SAMPLES,
    )?;
    let subscriber_token = join_reader(&mut device_opens, &shared_device, &subscriber, &owner)?;
    let other_waiter_token = join_reader(&mut device_opens, &shared_device, &other_waiter, &owner)?;
    assert!(matches!(
        device_opens.await_open(DUPLICATE_PROMOTION_TICKET, &owner_token),
        AwaitRegistration::Registered
    ));
    assert!(matches!(
        device_opens.await_open(DUPLICATE_PROMOTION_TICKET, &other_waiter_token),
        AwaitRegistration::Registered
    ));
    assert!(matches!(
        device_opens.await_open(DUPLICATE_PROMOTION_TICKET, &subscriber_token),
        AwaitRegistration::Registered
    ));

    let DeviceOpenRetirement::PromotedSubscriber {
        promoted,
        token: promoted_token,
        awaited,
        owed,
    } = device_opens.leave(owner_token)
    else {
        return Err(wrong_outcome("PromotedSubscriber"));
    };
    assert_eq!(promoted, subscriber);
    assert_eq!(promoted_token, subscriber_token);
    let AwaitedOpen::Awaiting(ticket) = awaited else {
        return Err(wrong_outcome("Awaiting"));
    };
    assert_eq!(ticket, DUPLICATE_PROMOTION_TICKET);
    assert!(matches!(owed, OwedOpen::NotOwed));
    let AwaitedOpenClaim::For(awaiting_tokens) =
        device_opens.claim_open(&DUPLICATE_PROMOTION_TICKET)
    else {
        return Err(wrong_outcome("For"));
    };
    assert_eq!(awaiting_tokens, vec![other_waiter_token, subscriber_token]);
    Ok(())
}

#[test]
fn readdress_keeps_one_debt_per_device_and_token() -> Result<(), Box<dyn Error>> {
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let provisional_device = device("duplicate-debt-provisional-tone")?;
    let durable_device = device("duplicate-debt-durable-tone")?;
    let owner = RoleKey::new("tone/duplicate-debt-owner")?;
    let owner_token = join_owner(
        &mut device_opens,
        &provisional_device,
        &owner,
        OWNER_HZ,
        OWNER_SAMPLES,
    )?;
    assert!(matches!(
        device_opens.owe_open(&owner_token),
        OweRegistration::Owed
    ));
    assert!(matches!(
        device_opens.owe_open(&owner_token),
        OweRegistration::Owed
    ));

    assert_eq!(
        device_opens.owed_opens().cloned().collect::<Vec<_>>(),
        vec![OwedOpenEntry {
            device: provisional_device.clone(),
            token:  owner_token.clone(),
        }]
    );

    assert!(matches!(
        device_opens.readdress(&provisional_device, durable_device.clone()),
        ReaddressedDeviceOpen::Moved
    ));
    assert_eq!(
        device_opens.owed_opens().collect::<Vec<_>>(),
        vec![&OwedOpenEntry {
            device: durable_device,
            token:  owner_token,
        }]
    );
    Ok(())
}

#[test]
fn readdress_moves_the_payload_readers_tickets_and_owed_opens_together()
-> Result<(), Box<dyn Error>> {
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let provisional_device = device("provisional-tone")?;
    let durable_device = device("durable-tone")?;
    let owner = RoleKey::new("tone/readdressed-owner")?;
    let reader = RoleKey::new("tone/readdressed-reader")?;
    let owner_token = join_owner(
        &mut device_opens,
        &provisional_device,
        &owner,
        OWNER_HZ,
        OWNER_SAMPLES,
    )?;
    let reader_token = join_reader(&mut device_opens, &provisional_device, &reader, &owner)?;
    assert!(matches!(
        device_opens.await_open(READDRESS_TICKET, &owner_token),
        AwaitRegistration::Registered
    ));
    assert!(matches!(
        device_opens.await_open(READDRESS_TICKET, &reader_token),
        AwaitRegistration::Registered
    ));
    assert!(matches!(
        device_opens.owe_open(&owner_token),
        OweRegistration::Owed
    ));
    assert!(matches!(
        device_opens.owe_open(&reader_token),
        OweRegistration::Owed
    ));

    assert!(matches!(
        device_opens.readdress(&provisional_device, durable_device.clone()),
        ReaddressedDeviceOpen::Moved
    ));
    assert!(matches!(
        device_opens.owner_of(&provisional_device),
        DeviceOpenOwner::NoOpen
    ));
    let DeviceOpenOwner::Role(recorded_owner) = device_opens.owner_of(&durable_device) else {
        return Err(wrong_outcome("Role"));
    };
    assert_eq!(recorded_owner, &owner);
    assert_eq!(
        device_opens
            .readers_of(&durable_device)
            .map(reader_identity)
            .collect::<Vec<_>>(),
        vec![(&owner, &owner_token), (&reader, &reader_token)]
    );
    assert!(
        device_opens
            .readers_of(&provisional_device)
            .next()
            .is_none()
    );

    let SharedOpenReading::ReadFrom {
        owner: recorded_owner,
        open,
    } = device_opens.open_of(&reader_token)
    else {
        return Err(wrong_outcome("ReadFrom"));
    };
    assert_eq!(recorded_owner, &owner);
    assert_eq!(open, &tone_open(OWNER_HZ, OWNER_SAMPLES));

    let DeviceOpenRetirement::PromotedSubscriber {
        promoted,
        token: promoted_token,
        awaited,
        owed,
    } = device_opens.leave(owner_token)
    else {
        return Err(wrong_outcome("PromotedSubscriber"));
    };
    assert_eq!(promoted, reader);
    assert_eq!(promoted_token, reader_token);
    let AwaitedOpen::Awaiting(ticket) = awaited else {
        return Err(wrong_outcome("Awaiting"));
    };
    assert_eq!(ticket, READDRESS_TICKET);
    assert!(matches!(owed, OwedOpen::Owed));
    let SharedOpenReading::OwnedHere(open) = device_opens.open_of(&promoted_token) else {
        return Err(wrong_outcome("OwnedHere"));
    };
    assert_eq!(open, &tone_open(OWNER_HZ, OWNER_SAMPLES));

    let AwaitedOpenClaim::For(awaiting_tokens) = device_opens.claim_open(&READDRESS_TICKET) else {
        return Err(wrong_outcome("For"));
    };
    assert_eq!(awaiting_tokens, vec![reader_token.clone()]);
    let owed_opens = device_opens.take_owed_opens();
    let [
        OwedOpenEntry {
            device: owed_device,
            token: owed_token,
        },
    ] = owed_opens.as_slice()
    else {
        return Err(wrong_outcome("one OwedOpenEntry"));
    };
    assert_eq!(owed_device, &durable_device);
    assert_eq!(owed_token, &reader_token);
    Ok(())
}

#[test]
fn readdress_refuses_a_missing_source_and_an_occupied_destination() -> Result<(), Box<dyn Error>> {
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let missing_device = device("missing-tone")?;
    let unused_destination = device("unused-tone")?;
    let source_device = device("source-tone")?;
    let occupied_device = device("occupied-tone")?;
    let source_owner = RoleKey::new("tone/source-owner")?;
    let occupied_owner = RoleKey::new("tone/occupied-owner")?;
    let occupied_token = join_owner(
        &mut device_opens,
        &occupied_device,
        &occupied_owner,
        OTHER_HZ,
        OTHER_SAMPLES,
    )?;

    assert!(matches!(
        device_opens.readdress(&missing_device, unused_destination),
        ReaddressedDeviceOpen::NoOpenAt
    ));
    let source_token = join_owner(
        &mut device_opens,
        &source_device,
        &source_owner,
        OWNER_HZ,
        OWNER_SAMPLES,
    )?;
    assert!(matches!(
        device_opens.await_open(READDRESS_TICKET, &source_token),
        AwaitRegistration::Registered
    ));
    assert!(matches!(
        device_opens.owe_open(&source_token),
        OweRegistration::Owed
    ));

    assert!(matches!(
        device_opens.readdress(&source_device, occupied_device.clone()),
        ReaddressedDeviceOpen::AlreadyOpenAt
    ));
    let DeviceOpenOwner::Role(recorded_source_owner) = device_opens.owner_of(&source_device) else {
        return Err(wrong_outcome("Role"));
    };
    assert_eq!(recorded_source_owner, &source_owner);
    let DeviceOpenOwner::Role(recorded_occupied_owner) = device_opens.owner_of(&occupied_device)
    else {
        return Err(wrong_outcome("Role"));
    };
    assert_eq!(recorded_occupied_owner, &occupied_owner);
    let SharedOpenReading::OwnedHere(source_open) = device_opens.open_of(&source_token) else {
        return Err(wrong_outcome("OwnedHere"));
    };
    assert_eq!(source_open, &tone_open(OWNER_HZ, OWNER_SAMPLES));
    let SharedOpenReading::OwnedHere(occupied_open) = device_opens.open_of(&occupied_token) else {
        return Err(wrong_outcome("OwnedHere"));
    };
    assert_eq!(occupied_open, &tone_open(OTHER_HZ, OTHER_SAMPLES));
    let AwaitedOpenClaim::For(awaiting_tokens) = device_opens.claim_open(&READDRESS_TICKET) else {
        return Err(wrong_outcome("For"));
    };
    assert_eq!(awaiting_tokens, vec![source_token.clone()]);
    let owed_opens = device_opens.take_owed_opens();
    let [
        OwedOpenEntry {
            device: owed_device,
            token: owed_token,
        },
    ] = owed_opens.as_slice()
    else {
        return Err(wrong_outcome("one OwedOpenEntry"));
    };
    assert_eq!(owed_device, &source_device);
    assert_eq!(owed_token, &source_token);
    Ok(())
}

#[test]
fn one_role_on_two_devices_answers_each_token_with_its_own_open_and_promotes_only_that_device()
-> Result<(), Box<dyn Error>> {
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let first_device = device("two-device-first-tone")?;
    let second_device = device("two-device-second-tone")?;
    let shared_role = RoleKey::new("tone/two-device-role")?;
    let first_device_reader = RoleKey::new("tone/first-device-reader")?;
    let first_device_owner_token = join_owner(
        &mut device_opens,
        &first_device,
        &shared_role,
        OWNER_HZ,
        OWNER_SAMPLES,
    )?;
    let second_device_owner_token = join_owner(
        &mut device_opens,
        &second_device,
        &shared_role,
        OTHER_HZ,
        OTHER_SAMPLES,
    )?;
    let first_device_reader_token = join_reader(
        &mut device_opens,
        &first_device,
        &first_device_reader,
        &shared_role,
    )?;
    assert!(matches!(
        device_opens.await_open(CROSS_DEVICE_TICKET, &second_device_owner_token),
        AwaitRegistration::Registered
    ));
    assert!(matches!(
        device_opens.owe_open(&second_device_owner_token),
        OweRegistration::Owed
    ));

    let DeviceOpenRetirement::PromotedSubscriber {
        promoted,
        token: promoted_token,
        awaited,
        owed,
    } = device_opens.leave(first_device_owner_token)
    else {
        return Err(wrong_outcome("PromotedSubscriber"));
    };
    assert_eq!(promoted, first_device_reader);
    assert_eq!(promoted_token, first_device_reader_token);
    assert!(matches!(awaited, AwaitedOpen::NoOpenInFlight));
    assert!(matches!(owed, OwedOpen::NotOwed));

    let SharedOpenReading::OwnedHere(first_open) = device_opens.open_of(&promoted_token) else {
        return Err(wrong_outcome("OwnedHere"));
    };
    assert_eq!(first_open, &tone_open(OWNER_HZ, OWNER_SAMPLES));
    let SharedOpenReading::OwnedHere(second_open) =
        device_opens.open_of(&second_device_owner_token)
    else {
        return Err(wrong_outcome("OwnedHere"));
    };
    assert_eq!(second_open, &tone_open(OTHER_HZ, OTHER_SAMPLES));

    let AwaitedOpenClaim::For(awaiting_tokens) = device_opens.claim_open(&CROSS_DEVICE_TICKET)
    else {
        return Err(wrong_outcome("For"));
    };
    assert_eq!(awaiting_tokens, vec![second_device_owner_token.clone()]);
    let owed_opens = device_opens.take_owed_opens();
    let [
        OwedOpenEntry {
            device: owed_device,
            token: owed_token,
        },
    ] = owed_opens.as_slice()
    else {
        return Err(wrong_outcome("one OwedOpenEntry"));
    };
    assert_eq!(owed_device, &second_device);
    assert_eq!(owed_token, &second_device_owner_token);

    let DeviceOpenOwner::Role(first_owner) = device_opens.owner_of(&first_device) else {
        return Err(wrong_outcome("Role"));
    };
    assert_eq!(first_owner, &first_device_reader);
    let DeviceOpenOwner::Role(second_owner) = device_opens.owner_of(&second_device) else {
        return Err(wrong_outcome("Role"));
    };
    assert_eq!(second_owner, &shared_role);
    Ok(())
}

#[test]
fn a_cancelled_attempts_ticket_never_reaches_the_roles_newer_token() -> Result<(), Box<dyn Error>> {
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let shared_device = device("cancelled-attempt-tone")?;
    let owner = RoleKey::new("tone/cancelled-attempt-owner")?;
    let attempting_role = RoleKey::new("tone/cancelled-attempt-reader")?;
    let owner_token = join_owner(
        &mut device_opens,
        &shared_device,
        &owner,
        OWNER_HZ,
        OWNER_SAMPLES,
    )?;
    let cancelled_token = join_reader(&mut device_opens, &shared_device, &attempting_role, &owner)?;
    let newer_token = join_reader(&mut device_opens, &shared_device, &attempting_role, &owner)?;
    assert!(matches!(
        device_opens.await_open(ABANDONED_TICKET, &cancelled_token),
        AwaitRegistration::Registered
    ));
    assert!(matches!(
        device_opens.owe_open(&cancelled_token),
        OweRegistration::Owed
    ));

    assert!(matches!(
        device_opens.leave(cancelled_token),
        DeviceOpenRetirement::ReaderLeft
    ));
    assert!(matches!(
        device_opens.claim_open(&ABANDONED_TICKET),
        AwaitedOpenClaim::Unclaimed
    ));
    assert!(device_opens.owed_opens().next().is_none());
    assert!(matches!(
        device_opens.open_of(&newer_token),
        SharedOpenReading::ReadFrom { .. }
    ));
    assert_eq!(
        device_opens
            .readers_of(&shared_device)
            .map(reader_identity)
            .collect::<Vec<_>>(),
        vec![(&owner, &owner_token), (&attempting_role, &newer_token)]
    );
    Ok(())
}

#[test]
fn a_claim_names_tokens_in_registration_order_across_two_roles_and_two_tokens_of_one_role()
-> Result<(), Box<dyn Error>> {
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let shared_device = device("ordered-token-claim-tone")?;
    let repeated_role = RoleKey::new("tone/ordered-repeated-role")?;
    let other_role = RoleKey::new("tone/ordered-other-role")?;
    let repeated_first_token = join_owner(
        &mut device_opens,
        &shared_device,
        &repeated_role,
        OWNER_HZ,
        OWNER_SAMPLES,
    )?;
    let other_token = join_reader(
        &mut device_opens,
        &shared_device,
        &other_role,
        &repeated_role,
    )?;
    let repeated_second_token = join_reader(
        &mut device_opens,
        &shared_device,
        &repeated_role,
        &repeated_role,
    )?;
    assert!(matches!(
        device_opens.await_open(SHARED_TICKET, &other_token),
        AwaitRegistration::Registered
    ));
    assert!(matches!(
        device_opens.await_open(SHARED_TICKET, &repeated_second_token),
        AwaitRegistration::Registered
    ));
    assert!(matches!(
        device_opens.await_open(SHARED_TICKET, &repeated_first_token),
        AwaitRegistration::Registered
    ));

    let AwaitedOpenClaim::For(awaiting_tokens) = device_opens.claim_open(&SHARED_TICKET) else {
        return Err(wrong_outcome("For"));
    };
    assert_eq!(
        awaiting_tokens,
        vec![other_token, repeated_second_token, repeated_first_token]
    );
    Ok(())
}

fn one_role_owning_two_devices() -> Result<
    (
        SharedDeviceOpens<ToneOpens>,
        DeviceReaderToken,
        DeviceReaderToken,
    ),
    Box<dyn Error>,
> {
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let first_device = device("cross-device-ticket-first-tone")?;
    let second_device = device("cross-device-ticket-second-tone")?;
    let shared_role = RoleKey::new("tone/cross-device-ticket-role")?;
    let first_token = join_owner(
        &mut device_opens,
        &first_device,
        &shared_role,
        OWNER_HZ,
        OWNER_SAMPLES,
    )?;
    let second_token = join_owner(
        &mut device_opens,
        &second_device,
        &shared_role,
        OWNER_HZ,
        OWNER_SAMPLES,
    )?;
    Ok((device_opens, first_token, second_token))
}

#[test]
#[should_panic(expected = "one physical-open ticket cannot target two devices")]
fn one_ticket_registered_from_two_devices_panics() {
    let Ok((mut device_opens, first_token, second_token)) = one_role_owning_two_devices() else {
        return;
    };
    assert!(matches!(
        device_opens.await_open(CROSS_DEVICE_TICKET, &first_token),
        AwaitRegistration::Registered
    ));

    let _: AwaitRegistration = device_opens.await_open(CROSS_DEVICE_TICKET, &second_token);
}

#[test]
fn one_role_on_two_devices_registers_by_token_and_promotes_only_that_device()
-> Result<(), Box<dyn Error>> {
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let promoted_device = device("token-promotion-device-tone")?;
    let awaiting_device = device("token-awaiting-device-tone")?;
    let shared_role = RoleKey::new("tone/token-cross-device-role")?;
    let promoted_role = RoleKey::new("tone/token-cross-device-reader")?;
    let promoted_device_owner_token = join_owner(
        &mut device_opens,
        &promoted_device,
        &shared_role,
        OWNER_HZ,
        OWNER_SAMPLES,
    )?;
    let awaiting_device_owner_token = join_owner(
        &mut device_opens,
        &awaiting_device,
        &shared_role,
        OTHER_HZ,
        OTHER_SAMPLES,
    )?;
    let promoted_token = join_reader(
        &mut device_opens,
        &promoted_device,
        &promoted_role,
        &shared_role,
    )?;
    assert!(matches!(
        device_opens.await_open(CROSS_DEVICE_TICKET, &awaiting_device_owner_token),
        AwaitRegistration::Registered
    ));
    assert!(matches!(
        device_opens.owe_open(&awaiting_device_owner_token),
        OweRegistration::Owed
    ));

    let DeviceOpenRetirement::PromotedSubscriber {
        promoted,
        token,
        awaited,
        owed,
    } = device_opens.leave(promoted_device_owner_token)
    else {
        return Err(wrong_outcome("PromotedSubscriber"));
    };
    assert_eq!(promoted, promoted_role);
    assert_eq!(token, promoted_token);
    assert!(matches!(awaited, AwaitedOpen::NoOpenInFlight));
    assert!(matches!(owed, OwedOpen::NotOwed));

    let AwaitedOpenClaim::For(awaiting_tokens) = device_opens.claim_open(&CROSS_DEVICE_TICKET)
    else {
        return Err(wrong_outcome("For"));
    };
    assert_eq!(awaiting_tokens, vec![awaiting_device_owner_token.clone()]);
    assert_eq!(
        device_opens.take_owed_opens(),
        vec![OwedOpenEntry {
            device: awaiting_device.clone(),
            token:  awaiting_device_owner_token,
        }]
    );
    let DeviceOpenOwner::Role(promoted_device_owner) = device_opens.owner_of(&promoted_device)
    else {
        return Err(wrong_outcome("Role"));
    };
    assert_eq!(promoted_device_owner, &promoted_role);
    let DeviceOpenOwner::Role(awaiting_device_owner) = device_opens.owner_of(&awaiting_device)
    else {
        return Err(wrong_outcome("Role"));
    };
    assert_eq!(awaiting_device_owner, &shared_role);
    Ok(())
}

#[test]
fn the_owners_pending_work_moves_onto_the_promoted_token_once() -> Result<(), Box<dyn Error>> {
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let shared_device = device("single-promotion-registration-tone")?;
    let unrelated_device = device("unrelated-owed-registration-tone")?;
    let owner = RoleKey::new("tone/single-promotion-owner")?;
    let subscriber = RoleKey::new("tone/single-promotion-subscriber")?;
    let unrelated_owner = RoleKey::new("tone/unrelated-owed-owner")?;
    let owner_token = join_owner(
        &mut device_opens,
        &shared_device,
        &owner,
        OWNER_HZ,
        OWNER_SAMPLES,
    )?;
    let subscriber_token = join_reader(&mut device_opens, &shared_device, &subscriber, &owner)?;
    let unrelated_token = join_owner(
        &mut device_opens,
        &unrelated_device,
        &unrelated_owner,
        OTHER_HZ,
        OTHER_SAMPLES,
    )?;
    assert!(matches!(
        device_opens.await_open(DUPLICATE_PROMOTION_TICKET, &owner_token),
        AwaitRegistration::Registered
    ));
    assert!(matches!(
        device_opens.await_open(DUPLICATE_PROMOTION_TICKET, &subscriber_token),
        AwaitRegistration::Registered
    ));
    assert!(matches!(
        device_opens.owe_open(&owner_token),
        OweRegistration::Owed
    ));
    assert!(matches!(
        device_opens.owe_open(&unrelated_token),
        OweRegistration::Owed
    ));
    assert!(matches!(
        device_opens.owe_open(&subscriber_token),
        OweRegistration::Owed
    ));

    let DeviceOpenRetirement::PromotedSubscriber {
        promoted,
        token,
        awaited,
        owed,
    } = device_opens.leave(owner_token)
    else {
        return Err(wrong_outcome("PromotedSubscriber"));
    };
    assert_eq!(promoted, subscriber);
    assert_eq!(token, subscriber_token);
    let AwaitedOpen::Awaiting(ticket) = awaited else {
        return Err(wrong_outcome("Awaiting"));
    };
    assert_eq!(ticket, DUPLICATE_PROMOTION_TICKET);
    assert!(matches!(owed, OwedOpen::Owed));

    let AwaitedOpenClaim::For(awaiting_tokens) =
        device_opens.claim_open(&DUPLICATE_PROMOTION_TICKET)
    else {
        return Err(wrong_outcome("For"));
    };
    assert_eq!(awaiting_tokens, vec![subscriber_token.clone()]);
    assert_eq!(
        device_opens.take_owed_opens(),
        vec![
            OwedOpenEntry {
                device: shared_device,
                token:  subscriber_token,
            },
            OwedOpenEntry {
                device: unrelated_device,
                token:  unrelated_token,
            },
        ]
    );
    Ok(())
}

#[test]
fn role_of_answers_the_role_of_a_member_and_not_a_reader_after_it_leaves()
-> Result<(), Box<dyn Error>> {
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let shared_device = device("role-of-token-tone")?;
    let owner = RoleKey::new("tone/role-of-owner")?;
    let token = join_owner(
        &mut device_opens,
        &shared_device,
        &owner,
        OWNER_HZ,
        OWNER_SAMPLES,
    )?;
    let departed_token = token.clone();

    let TokenRole::Role(recorded_role) = device_opens.role_of(&token) else {
        return Err(wrong_outcome("Role"));
    };
    assert_eq!(recorded_role, &owner);
    assert!(matches!(
        device_opens.leave(token),
        DeviceOpenRetirement::ClosedLastReader { .. }
    ));
    assert!(matches!(
        device_opens.role_of(&departed_token),
        TokenRole::NotAReader
    ));
    Ok(())
}

#[test]
fn opens_visits_each_physical_open_once_whatever_the_number_of_readers()
-> Result<(), Box<dyn Error>> {
    let mut device_opens = SharedDeviceOpens::<ToneOpens>::new();
    let first_device = device("walked-first-tone")?;
    let second_device = device("walked-second-tone")?;
    let first_owner = RoleKey::new("tone/walked-first-owner")?;
    let first_reader = RoleKey::new("tone/walked-first-reader")?;
    let second_reader = RoleKey::new("tone/walked-second-reader")?;
    let second_owner = RoleKey::new("tone/walked-second-owner")?;
    join_owner(
        &mut device_opens,
        &first_device,
        &first_owner,
        OWNER_HZ,
        OWNER_SAMPLES,
    )?;
    join_reader(
        &mut device_opens,
        &first_device,
        &first_reader,
        &first_owner,
    )?;
    join_reader(
        &mut device_opens,
        &first_device,
        &second_reader,
        &first_owner,
    )?;
    join_owner(
        &mut device_opens,
        &second_device,
        &second_owner,
        OTHER_HZ,
        OTHER_SAMPLES,
    )?;

    for (_, open) in device_opens.opens_mut() {
        open.samples.push(WALK_SAMPLE);
    }
    let mut visited = device_opens
        .opens()
        .map(|(_, open)| (open.hz, open.samples.clone()))
        .collect::<Vec<_>>();
    visited.sort_unstable_by_key(|(hz, _)| *hz);

    let mut owner_samples = OWNER_SAMPLES.to_vec();
    owner_samples.push(WALK_SAMPLE);
    let mut other_samples = OTHER_SAMPLES.to_vec();
    other_samples.push(WALK_SAMPLE);
    assert_eq!(
        visited,
        vec![(OWNER_HZ, owner_samples), (OTHER_HZ, other_samples)]
    );
    Ok(())
}
