//! Event module.

use crate::battle::{Battle, BattleRules, BattleState, Version};
use crate::error::{WeaselError, WeaselResult};
use crate::player::PlayerId;
use crate::team::TeamId;
use crate::user::UserEventId;
use log::error;
#[cfg(feature = "serialization")]
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::marker::PhantomData;
use std::ops::{Deref, Range};

/// Type for the id of events.
pub type EventId = u32;

/// Enum to represent all different kinds of events.
// Internal note: remember to update the event debug and serialization tests in tests/event.rs
// each time a new event is added to weasel.
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum EventKind {
    /// Dummy event doing nothing.
    DummyEvent,
    /// Start a new round.
    StartRound,
    /// End the current round.
    EndRound,
    /// Create a new team.
    CreateTeam,
    /// Create a new creature.
    CreateCreature,
    /// Activate an actor's ability.
    ActivateAbility,
    /// Reset the entropy model.
    ResetEntropy,
    /// Move an entity from one position to another.
    MoveEntity,
    /// Apply the consequences of an impact on the world.
    ApplyImpact,
    /// Modify the statistics of a character.
    AlterStatistics,
    /// Modify the abilities of an actor.
    AlterAbilities,
    /// Set new relations between teams.
    SetRelations,
    /// Convert a creature from one team to another.
    ConvertCreature,
    /// End the battle.
    EndBattle,
    /// An event to set a team's objectives outcome.
    ConcludeObjectives,
    /// Reset the objectives of a team.
    ResetObjectives,
    /// Reset the rounds model.
    ResetRounds,
    /// Reset the space model.
    ResetSpace,
    /// Removes a creature from the battle.
    RemoveCreature,
    /// A user defined event with an unique id.
    UserEvent(UserEventId),
}

/// Types of access rights that might be required in order to fire an event.
pub enum EventRights<'a, R: BattleRules> {
    /// Everyone can fire the event.
    None,
    /// Only the server can fire the event.
    Server,
    /// Only the server or a player with rights to this team can fire the event.
    Team(&'a TeamId<R>),
}

/// An event is the only mean to apply a change to the world.
pub trait Event<R: BattleRules>: std::fmt::Debug {
    /// Verifies if this event can be applied to the world.
    fn verify(&self, battle: &Battle<R>) -> WeaselResult<(), R>;

    /// Applies this event to the world. This method is called only if `verify` succeeded.
    ///
    /// If there's a failure inside this method, it immediately panic because we can't guarantee
    /// any consistency in the state of the world.
    ///
    /// Events generated by this event are stored into `queue`, if there's one.
    /// Noe that they will keep a link with the original event.
    fn apply(&self, battle: &mut Battle<R>, queue: &mut Option<EventQueue<R>>);

    /// Returns the kind of this event.
    fn kind(&self) -> EventKind;

    /// Clones this event as a trait object.
    fn box_clone(&self) -> Box<dyn Event<R>>;

    /// Returns an `Any` reference this event.
    fn as_any(&self) -> &dyn Any;

    /// Returns the access rights required by this event.
    ///
    /// The provided implementation returns `EventRights::Server`.
    fn rights<'a>(&'a self, _battle: &'a Battle<R>) -> EventRights<'a, R> {
        EventRights::Server
    }
}

impl<R: BattleRules> Clone for Box<dyn Event<R>> {
    fn clone(&self) -> Box<dyn Event<R>> {
        self.box_clone()
    }
}

impl<R: BattleRules> PartialEq<Box<dyn Event<R>>> for Box<dyn Event<R>> {
    fn eq(&self, other: &Box<dyn Event<R>>) -> bool {
        self.kind() == other.kind()
    }
}

/// A wrapper to decorate verified events with additional data.
pub struct EventWrapper<R: BattleRules> {
    /// Event Id is assigned only after events has been verified for consistency.
    pub(crate) id: EventId,
    /// Id of the event that generated this one.
    pub(crate) origin: Option<EventId>,
    /// The actual event wrapped inside this struct.
    pub(crate) event: Box<dyn Event<R>>,
}

impl<R: BattleRules> Clone for EventWrapper<R> {
    fn clone(&self) -> EventWrapper<R> {
        EventWrapper::new(self.id, self.origin, self.event.clone())
    }
}

impl<R: BattleRules> EventWrapper<R> {
    /// Creates a new EventWrapper.
    pub(crate) fn new(
        id: EventId,
        origin: Option<EventId>,
        event: Box<dyn Event<R>>,
    ) -> EventWrapper<R> {
        EventWrapper { id, origin, event }
    }

    /// Returns this event's id.
    pub fn id(&self) -> EventId {
        self.id
    }

    /// Returns the id of the event that caused this one.
    pub fn origin(&self) -> Option<EventId> {
        self.origin
    }

    /// Returns the event.
    #[allow(clippy::borrowed_box)]
    pub fn event(&self) -> &Box<dyn Event<R>> {
        &self.event
    }

    /// Consume this event wrapper and returns a versioned instance of it.
    pub fn version(self, version: Version<R>) -> VersionedEventWrapper<R> {
        VersionedEventWrapper::new(self, version)
    }
}

impl<R: BattleRules> Deref for EventWrapper<R> {
    type Target = Box<dyn Event<R>>;

    fn deref(&self) -> &Self::Target {
        &self.event
    }
}

/// Decorates an `EventWrapper` with the battle rules version.
pub struct VersionedEventWrapper<R: BattleRules> {
    pub(crate) wrapper: EventWrapper<R>,
    pub(crate) version: Version<R>,
}

impl<R: BattleRules> Clone for VersionedEventWrapper<R> {
    fn clone(&self) -> VersionedEventWrapper<R> {
        VersionedEventWrapper::new(self.wrapper.clone(), self.version.clone())
    }
}

impl<R: BattleRules> VersionedEventWrapper<R> {
    /// Creates a new VersionedEventWrapper.
    pub(crate) fn new(wrapper: EventWrapper<R>, version: Version<R>) -> VersionedEventWrapper<R> {
        VersionedEventWrapper { wrapper, version }
    }

    /// Returns the `EventWrapper` contained in this object.
    pub fn wrapper(&self) -> &EventWrapper<R> {
        &self.wrapper
    }

    /// Returns the `BattleRules`' version of the event.
    pub fn version(&self) -> &Version<R> {
        &self.version
    }
}

impl<R: BattleRules> Deref for VersionedEventWrapper<R> {
    type Target = EventWrapper<R>;

    fn deref(&self) -> &Self::Target {
        &self.wrapper
    }
}

/// Function that tells if an event prototype met its additional conditions
/// in order to be applied.
pub type Condition<R> = std::rc::Rc<dyn Fn(&BattleState<R>) -> bool>;

/// A prototype for tentative events that are not yet verified.
pub struct EventPrototype<R: BattleRules> {
    /// Id of the event that generated this one.
    pub(crate) origin: Option<EventId>,
    /// The actual event wrapped inside this struct.
    pub(crate) event: Box<dyn Event<R>>,
    /// Condition that must be satisfied for this prototype to be valid.
    pub(crate) condition: Option<Condition<R>>,
}

impl<R: BattleRules> EventPrototype<R> {
    /// Creates a new EventPrototype.
    pub(crate) fn new(event: Box<dyn Event<R>>) -> EventPrototype<R> {
        EventPrototype {
            origin: None,
            event,
            condition: None,
        }
    }

    pub(crate) fn promote(self, id: EventId) -> EventWrapper<R> {
        EventWrapper::new(id, self.origin, self.event)
    }

    /// Returns the id of the event that caused this one.
    pub fn origin(&self) -> Option<EventId> {
        self.origin
    }

    /// Returns the event.
    #[allow(clippy::borrowed_box)]
    pub fn event(&self) -> &Box<dyn Event<R>> {
        &self.event
    }

    /// Returns the prototype's acceptance condition.
    pub fn condition(&self) -> &Option<Condition<R>> {
        &self.condition
    }

    /// Consume this event prototype and returns a `ClientEventPrototype` instance of it.
    pub fn client_prototype(
        self,
        version: Version<R>,
        player: Option<PlayerId>,
    ) -> ClientEventPrototype<R> {
        ClientEventPrototype::new(self.origin, self.event, version, player)
    }
}

impl<R: BattleRules> Deref for EventPrototype<R> {
    type Target = Box<dyn Event<R>>;

    fn deref(&self) -> &Self::Target {
        &self.event
    }
}

impl<R: BattleRules> Clone for EventPrototype<R> {
    fn clone(&self) -> Self {
        EventPrototype {
            origin: self.origin,
            event: self.event.clone(),
            condition: self.condition.clone(),
        }
    }
}

/// An event prototype sent by a client to a server that must be first verified
/// before being processed.
pub struct ClientEventPrototype<R: BattleRules> {
    /// Id of the event that generated this one.
    origin: Option<EventId>,
    /// The actual event wrapped inside this struct.
    pub(crate) event: Box<dyn Event<R>>,
    /// Version of `BattleRules` that generated this event.
    pub(crate) version: Version<R>,
    /// Id of the player who fired this event.
    player: Option<PlayerId>,
}

impl<R: BattleRules> ClientEventPrototype<R> {
    /// Creates a new ClientEventPrototype.
    pub(crate) fn new(
        origin: Option<EventId>,
        event: Box<dyn Event<R>>,
        version: Version<R>,
        player: Option<PlayerId>,
    ) -> ClientEventPrototype<R> {
        ClientEventPrototype {
            origin,
            event,
            version,
            player,
        }
    }

    /// Returns the `BattleRules`'s version of the event.
    pub fn version(&self) -> &Version<R> {
        &self.version
    }

    /// Returns the id of the event that caused this one.
    pub fn origin(&self) -> Option<EventId> {
        self.origin
    }

    /// Returns the event.
    #[allow(clippy::borrowed_box)]
    pub fn event(&self) -> &Box<dyn Event<R>> {
        &self.event
    }

    /// Transforms this client event into an event prototype.
    pub(crate) fn prototype(self) -> EventPrototype<R> {
        EventPrototype {
            origin: self.origin,
            event: self.event,
            condition: None,
        }
    }

    /// Authenticate this event with the identity of `player`.
    ///
    /// **Note:** you can use this method to perform server-side authentication of events coming
    /// from a remote client.
    pub fn authenticate(&mut self, player: PlayerId) {
        self.player = Some(player);
    }

    /// Returns the player who fired this prototype.
    pub fn player(&self) -> Option<PlayerId> {
        self.player
    }
}

impl<R: BattleRules> Deref for ClientEventPrototype<R> {
    type Target = Box<dyn Event<R>>;

    fn deref(&self) -> &Self::Target {
        &self.event
    }
}

impl<R: BattleRules> Clone for ClientEventPrototype<R> {
    fn clone(&self) -> Self {
        ClientEventPrototype {
            origin: self.origin,
            event: self.event.clone(),
            version: self.version.clone(),
            player: self.player,
        }
    }
}

/// A trait to describe an output type from an event processor.
/// The requirement of this type is to be able to return an object for an ok state.
pub trait DefaultOutput {
    /// Error type for this `DefaultOutput`.
    type Error: Sized + PartialEq + std::fmt::Debug;
    /// Returns the `ok` result for this type.
    fn ok() -> Self;
    /// Convert this output to a Option.
    fn err(self) -> Option<Self::Error>;
}

/// A trait for objects that can process new local events.
pub trait EventProcessor<R: BattleRules> {
    /// Return type for this processor's `process()`.
    type ProcessOutput: DefaultOutput;

    /// Processes a local event prototype.
    fn process(&mut self, event: EventPrototype<R>) -> Self::ProcessOutput;
}

/// A trait for objects that can verify and process new client events.
pub trait EventServer<R: BattleRules> {
    /// Processes a client event prototype.
    fn process_client(&mut self, event: ClientEventPrototype<R>) -> WeaselResult<(), R>;
}

/// A trait for objects that can receive verified events.
pub trait EventReceiver<R: BattleRules> {
    /// Processes a verified event.
    fn receive(&mut self, event: VersionedEventWrapper<R>) -> WeaselResult<(), R>;
}

/// Trait to unify the interface of all event builders.
pub trait EventTrigger<'a, R: BattleRules, P: 'a + EventProcessor<R>> {
    /// Returns the processor bound to this trigger.
    fn processor(&'a mut self) -> &'a mut P;

    /// Returns the event constructed by this builder.
    fn event(&self) -> Box<dyn Event<R>>;

    /// Fires the event constructed by this builder.
    fn fire(&'a mut self) -> P::ProcessOutput {
        let prototype = self.prototype();
        self.processor().process(prototype)
    }

    /// Returns the event constructed by this builder, wrapped in a prototype.
    fn prototype(&self) -> EventPrototype<R> {
        EventPrototype::new(self.event())
    }
}

/// Collection to queue events prototypes, in order of insertion.
pub type EventQueue<R> = Vec<EventPrototype<R>>;

// Implement `EventProcessor` for event queues, so that it can be possible to
// use the latter with event triggers.
impl<R: BattleRules> EventProcessor<R> for EventQueue<R> {
    type ProcessOutput = ();

    fn process(&mut self, event: EventPrototype<R>) -> Self::ProcessOutput {
        self.push(event);
    }
}

/// An event that does nothing.
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
pub struct DummyEvent<R> {
    #[cfg_attr(feature = "serialization", serde(skip))]
    _phantom: PhantomData<R>,
}

impl<R: BattleRules> DummyEvent<R> {
    /// Returns a trigger for this event.
    pub fn trigger<P: EventProcessor<R>>(processor: &mut P) -> DummyEventTrigger<R, P> {
        DummyEventTrigger {
            processor,
            _phantom: PhantomData,
        }
    }
}

impl<R> std::fmt::Debug for DummyEvent<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DummyEvent {{ }}")
    }
}

impl<R> Clone for DummyEvent<R> {
    fn clone(&self) -> Self {
        DummyEvent {
            _phantom: PhantomData,
        }
    }
}

impl<R: BattleRules + 'static> Event<R> for DummyEvent<R> {
    fn verify(&self, _: &Battle<R>) -> WeaselResult<(), R> {
        Ok(())
    }

    fn apply(&self, _: &mut Battle<R>, _: &mut Option<EventQueue<R>>) {}

    fn kind(&self) -> EventKind {
        EventKind::DummyEvent
    }

    fn box_clone(&self) -> Box<dyn Event<R>> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rights<'a>(&'a self, _battle: &'a Battle<R>) -> EventRights<'a, R> {
        EventRights::None
    }
}

/// Trigger to build and fire a `DummyEvent` event.
pub struct DummyEventTrigger<'a, R, P>
where
    R: BattleRules,
    P: EventProcessor<R>,
{
    processor: &'a mut P,
    _phantom: PhantomData<R>,
}

impl<'a, R, P> EventTrigger<'a, R, P> for DummyEventTrigger<'a, R, P>
where
    R: BattleRules + 'static,
    P: EventProcessor<R>,
{
    fn processor(&'a mut self) -> &'a mut P {
        self.processor
    }

    /// Returns a `DummyEvent` event.
    fn event(&self) -> Box<dyn Event<R>> {
        Box::new(DummyEvent {
            _phantom: PhantomData,
        })
    }
}

// Implement `EventProcessor` for option, so that it would be possible to pass
// None or a real processor to event triggers.
impl<R, T> EventProcessor<R> for &mut Option<T>
where
    R: BattleRules,
    T: EventProcessor<R>,
{
    type ProcessOutput = T::ProcessOutput;

    fn process(&mut self, event: EventPrototype<R>) -> Self::ProcessOutput {
        if let Some(processor) = self {
            processor.process(event)
        } else {
            Self::ProcessOutput::ok()
        }
    }
}

impl<R, T> EventProcessor<R> for Option<T>
where
    R: BattleRules,
    T: EventProcessor<R>,
{
    type ProcessOutput = T::ProcessOutput;

    fn process(&mut self, event: EventPrototype<R>) -> Self::ProcessOutput {
        if let Some(processor) = self {
            processor.process(event)
        } else {
            Self::ProcessOutput::ok()
        }
    }
}

// Implement `EventProcessor` for (), doing nothing.
impl<R> EventProcessor<R> for ()
where
    R: BattleRules,
{
    type ProcessOutput = WeaselResult<(), R>;

    fn process(&mut self, _: EventPrototype<R>) -> Self::ProcessOutput {
        Err(WeaselError::EmptyEventProcessor)
    }
}

impl DefaultOutput for () {
    type Error = ();
    fn ok() -> Self {}
    fn err(self) -> Option<Self::Error> {
        None
    }
}

/// Decorator for `EventQueue` processor. It appends new events at the front of the queue, instead
/// of pushing them at the back.
pub struct Prioritized<'a, R: BattleRules> {
    event_queue: &'a mut EventQueue<R>,
}

impl<'a, R: BattleRules> Prioritized<'a, R> {
    /// Creates a new Prioritized decorator for the given `event_queue`.
    pub fn new(event_queue: &'a mut EventQueue<R>) -> Prioritized<R> {
        Prioritized { event_queue }
    }
}

impl<R> EventProcessor<R> for Prioritized<'_, R>
where
    R: BattleRules,
{
    type ProcessOutput = ();

    fn process(&mut self, event: EventPrototype<R>) -> Self::ProcessOutput {
        self.event_queue.insert(0, event);
    }
}

/// Decorator for event triggers to add a condition on the generated event prototype.
pub struct Conditional<'a, R, T, P>
where
    R: BattleRules,
    T: EventTrigger<'a, R, P>,
    P: 'a + EventProcessor<R>,
{
    trigger: T,
    condition: Condition<R>,
    _phantom: PhantomData<&'a P>,
}

impl<'a, R, T, P> Conditional<'a, R, T, P>
where
    R: BattleRules,
    T: EventTrigger<'a, R, P>,
    P: 'a + EventProcessor<R>,
    Condition<R>: Clone,
{
    /// Creates a new `Conditional` decorator for an `EventTrigger`.
    pub fn new(trigger: T, condition: Condition<R>) -> Conditional<'a, R, T, P> {
        Conditional {
            trigger,
            condition,
            _phantom: PhantomData,
        }
    }
}

impl<'a, R, T, P> EventTrigger<'a, R, P> for Conditional<'a, R, T, P>
where
    R: BattleRules,
    T: EventTrigger<'a, R, P>,
    P: 'a + EventProcessor<R>,
    Condition<R>: Clone,
{
    fn processor(&'a mut self) -> &'a mut P {
        self.trigger.processor()
    }

    fn event(&self) -> Box<dyn Event<R>> {
        self.trigger.event()
    }

    fn prototype(&self) -> EventPrototype<R> {
        let mut prototype = self.trigger.prototype();
        prototype.condition = Some(self.condition.clone());
        prototype
    }
}

/// Id of an event sink.
pub type EventSinkId = u16;

/// Basic trait for event sinks.
pub trait EventSink {
    /// Returns the Id associated to this sink.
    fn id(&self) -> EventSinkId;

    /// Invoked when this sink is forcedly disconnected.
    ///
    /// The provided implementation does nothing.
    fn on_disconnect(&mut self) {}
}

/// An output sink to dump versioned and verified events to a client.
pub trait ClientSink<R: BattleRules>: EventSink {
    /// Sends an already accepted event to a remote or local client.
    fn send(&mut self, event: &VersionedEventWrapper<R>) -> WeaselResult<(), R>;
}

/// An output sink to dump tentative events to a server.
pub trait ServerSink<R: BattleRules>: EventSink {
    /// Sends a client event prototype to a remote or local server.
    fn send(&mut self, event: &ClientEventPrototype<R>) -> WeaselResult<(), R>;
}

/// A data structure to contain multiple client sinks.
pub(crate) struct MultiClientSink<R: BattleRules> {
    sinks: Vec<Box<dyn ClientSink<R>>>,
}

impl<R: BattleRules> MultiClientSink<R> {
    pub(crate) fn new() -> MultiClientSink<R> {
        MultiClientSink { sinks: Vec::new() }
    }

    /// Adds a new sink.
    /// Returns an error if another sink with the same id already exists.
    pub(crate) fn add(&mut self, sink: Box<dyn ClientSink<R>>) -> WeaselResult<(), R> {
        if self.sinks.iter().any(|e| e.id() == sink.id()) {
            Err(WeaselError::DuplicatedEventSink(sink.id()))
        } else {
            self.sinks.push(sink);
            Ok(())
        }
    }

    /// Sends all `events` to an existing sink.
    /// Returns an error if sending the events failed or the sink doesn't exist.
    pub(crate) fn send<I>(&mut self, id: EventSinkId, events: I) -> WeaselResult<(), R>
    where
        I: Iterator<Item = VersionedEventWrapper<R>>,
    {
        let index = self.sinks.iter().position(|e| e.id() == id);
        if let Some(index) = index {
            // Send events.
            for event in events {
                let sink = &mut self.sinks[index];
                let result = sink.send(&event);
                if result.is_err() {
                    sink.on_disconnect();
                    self.sinks.remove(index);
                }
                result?;
            }
            Ok(())
        } else {
            Err(WeaselError::EventSinkNotFound(id))
        }
    }

    /// Removes the sink with the given `id`, if it exists.
    pub(crate) fn remove(&mut self, id: EventSinkId) {
        let index = self.sinks.iter().position(|e| e.id() == id);
        if let Some(index) = index {
            self.sinks.remove(index);
        }
    }

    /// Sends an event to all sinks.
    /// If a sink returns an error, its on_disconnect() fn will be invoked
    /// and the sink is disconnected from the server.
    pub(crate) fn send_all(&mut self, event: &VersionedEventWrapper<R>) {
        let mut failed_sinks_index = Vec::new();
        for (i, sink) in self.sinks.iter_mut().enumerate() {
            sink.send(event).unwrap_or_else(|err| {
                error!("{:?}", err);
                failed_sinks_index.push(i)
            });
        }
        for i in failed_sinks_index {
            self.sinks[i].on_disconnect();
            self.sinks.remove(i);
        }
    }

    pub(crate) fn sinks(&self) -> impl Iterator<Item = &Box<dyn ClientSink<R>>> {
        self.sinks.iter()
    }
}

/// A structure to access client sinks.
pub struct MultiClientSinkHandle<'a, R>
where
    R: BattleRules,
{
    sinks: &'a MultiClientSink<R>,
}

impl<'a, R> MultiClientSinkHandle<'a, R>
where
    R: BattleRules + 'static,
{
    pub(crate) fn new(sinks: &'a MultiClientSink<R>) -> MultiClientSinkHandle<'a, R> {
        MultiClientSinkHandle { sinks }
    }

    /// Returns an iterator over all sinks.
    pub fn sinks(&self) -> impl Iterator<Item = &Box<dyn ClientSink<R>>> {
        self.sinks.sinks()
    }
}

/// A structure to access and manipulate client sinks.
pub struct MultiClientSinkHandleMut<'a, R>
where
    R: BattleRules + 'static,
{
    sinks: &'a mut MultiClientSink<R>,
    battle: &'a Battle<R>,
}

impl<'a, R> MultiClientSinkHandleMut<'a, R>
where
    R: BattleRules + 'static,
{
    pub(crate) fn new(
        sinks: &'a mut MultiClientSink<R>,
        battle: &'a Battle<R>,
    ) -> MultiClientSinkHandleMut<'a, R> {
        MultiClientSinkHandleMut { sinks, battle }
    }

    /// Adds a new sink.
    ///
    /// Sinks must have unique ids.
    pub fn add_sink(&mut self, sink: Box<dyn ClientSink<R>>) -> WeaselResult<(), R> {
        self.sinks.add(sink)
    }

    /// Adds a new sink and shares the battle history with it,
    /// starting from the event having `event_id` up to the most recent event.
    ///
    /// Sinks must have unique ids.
    pub fn add_sink_from(
        &mut self,
        sink: Box<dyn ClientSink<R>>,
        event_id: EventId,
    ) -> WeaselResult<(), R> {
        self.add_sink_range(
            sink,
            Range {
                start: event_id,
                end: self.battle.history().len(),
            },
        )
    }

    /// Adds a new sink and shares a portion of the battle history with it.
    /// More precisely, only the events inside `range` will be sent to the sink.
    ///
    /// Sinks must have unique ids.
    pub fn add_sink_range(
        &mut self,
        sink: Box<dyn ClientSink<R>>,
        range: Range<EventId>,
    ) -> WeaselResult<(), R> {
        let range = normalize_range(range, self.battle.history().len())?;
        // Add the new sink.
        let sink_id = sink.id();
        self.sinks.add(sink)?;
        // Get all versioned events from history and send them.
        self.sinks
            .send(sink_id, self.battle.versioned_events(range))
    }

    /// Sends a range of events from the battle history to the sink with the given id.
    pub fn send_range(&mut self, id: EventSinkId, range: Range<EventId>) -> WeaselResult<(), R> {
        let range = normalize_range(range, self.battle.history().len())?;
        // Get all versioned events from history and send them.
        self.sinks.send(id, self.battle.versioned_events(range))
    }

    /// Removes the sink with the given id.
    pub fn remove_sink(&mut self, id: EventSinkId) {
        self.sinks.remove(id);
    }

    /// Returns an iterator over all sinks.
    pub fn sinks(&self) -> impl Iterator<Item = &Box<dyn ClientSink<R>>> {
        self.sinks.sinks()
    }
}

/// Converts a range of `EventId` into a range of `usize`.
fn normalize_range<R: BattleRules>(
    range: Range<EventId>,
    history_len: EventId,
) -> WeaselResult<Range<usize>, R> {
    if range.start > range.end || range.end > history_len {
        return Err(WeaselError::InvalidEventRange(range, history_len));
    }
    let range: Range<usize> = Range {
        start: range.start as usize,
        end: range.end as usize,
    };
    Ok(range)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entropy::ResetEntropy;
    use crate::{battle_rules, rules::empty::*};
    use std::iter::once;

    battle_rules! {}

    #[test]
    fn event_equality() {
        let dummy = DummyEvent::<CustomRules>::trigger(&mut ()).event();
        let dummy_copy = dummy.clone();
        let reset_entropy = ResetEntropy::<CustomRules>::trigger(&mut ()).event();
        assert_eq!(&dummy, &dummy_copy);
        assert_ne!(&dummy, &reset_entropy);
    }

    #[test]
    fn prioritized() {
        let mut queue = EventQueue::<CustomRules>::new();
        DummyEvent::<CustomRules>::trigger(&mut queue).fire();
        ResetEntropy::<CustomRules>::trigger(&mut Prioritized::new(&mut queue)).fire();
        assert_eq!(queue[0].kind(), EventKind::ResetEntropy);
        assert_eq!(queue[1].kind(), EventKind::DummyEvent);
    }

    #[test]
    fn multi_client_sink() {
        struct Sink {
            id: EventSinkId,
            ok: bool,
        }

        impl EventSink for Sink {
            fn id(&self) -> EventSinkId {
                self.id
            }
        }

        impl ClientSink<CustomRules> for Sink {
            fn send(
                &mut self,
                _: &VersionedEventWrapper<CustomRules>,
            ) -> WeaselResult<(), CustomRules> {
                if self.ok {
                    Ok(())
                } else {
                    Err(WeaselError::EventSinkError("broken".to_string()))
                }
            }
        }

        // Check add.
        let mut multi = MultiClientSink::new();
        assert_eq!(multi.add(Box::new(Sink { id: 0, ok: true })).err(), None);
        assert_eq!(multi.sinks.len(), 1);
        assert_eq!(
            multi.add(Box::new(Sink { id: 0, ok: true })).err(),
            Some(WeaselError::DuplicatedEventSink(0))
        );
        assert_eq!(multi.sinks.len(), 1);
        // Check remove.
        multi.remove(2);
        assert_eq!(multi.sinks.len(), 1);
        multi.remove(0);
        assert_eq!(multi.sinks.len(), 0);
        // Check send_all.
        assert_eq!(multi.add(Box::new(Sink { id: 0, ok: true })).err(), None);
        assert_eq!(multi.add(Box::new(Sink { id: 1, ok: false })).err(), None);
        assert_eq!(multi.sinks.len(), 2);
        let event = DummyEvent::<CustomRules>::trigger(&mut ())
            .prototype()
            .promote(0)
            .version(0);
        multi.send_all(&event);
        assert_eq!(multi.sinks.len(), 1);
        // Check send.
        assert_eq!(multi.send(0, once(event.clone())).err(), None);
        assert_eq!(
            multi.send(2, once(event.clone())).err(),
            Some(WeaselError::EventSinkNotFound(2))
        );
        assert_eq!(multi.add(Box::new(Sink { id: 1, ok: false })).err(), None);
        assert_eq!(multi.sinks.len(), 2);
        assert_eq!(
            multi.send(1, once(event.clone())).err(),
            Some(WeaselError::EventSinkError("broken".to_string()))
        );
        assert_eq!(multi.sinks.len(), 1);
    }

    /// Test decorator for event triggers.
    struct ChangeOrigin<'a, R, T, P>
    where
        R: BattleRules,
        T: EventTrigger<'a, R, P>,
        P: 'a + EventProcessor<R>,
    {
        trigger: T,
        origin: EventId,
        _phantom: PhantomData<&'a P>,
        _phantom_: PhantomData<R>,
    }

    impl<'a, R, T, P> ChangeOrigin<'a, R, T, P>
    where
        R: BattleRules,
        T: EventTrigger<'a, R, P>,
        P: 'a + EventProcessor<R>,
    {
        pub fn new(trigger: T, origin: EventId) -> ChangeOrigin<'a, R, T, P> {
            ChangeOrigin {
                trigger,
                origin,
                _phantom: PhantomData,
                _phantom_: PhantomData,
            }
        }
    }

    impl<'a, R, T, P> EventTrigger<'a, R, P> for ChangeOrigin<'a, R, T, P>
    where
        R: BattleRules,
        T: EventTrigger<'a, R, P>,
        P: 'a + EventProcessor<R>,
    {
        fn processor(&'a mut self) -> &'a mut P {
            self.trigger.processor()
        }

        fn event(&self) -> Box<dyn Event<R>> {
            self.trigger.event()
        }

        fn prototype(&self) -> EventPrototype<R> {
            let mut prototype = self.trigger.prototype();
            prototype.origin = Some(self.origin);
            prototype
        }
    }

    #[test]
    fn decorators_stack() {
        let mut processor = ();
        let event = Conditional::new(
            DummyEvent::trigger(&mut processor),
            std::rc::Rc::new(|_: &BattleState<CustomRules>| true),
        );
        let event = ChangeOrigin::new(event, 0);
        let prototype = event.prototype();
        assert!(prototype.condition.is_some());
        assert!(prototype.origin.is_some());
    }
}
