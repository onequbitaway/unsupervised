//! The simulation thread: the mailbox loop that owns the world.
//!
//! The loop takes calls from the mailbox in batches. For each batch it
//!
//! 1. gives every input of the batch to the journal
//!    ([`Journal::record`](crate::Journal::record));
//! 2. lets the journal make them durable
//!    ([`Journal::commit`](crate::Journal::commit));
//! 3. applies the inputs and answers the calls, in the order of arrival.
//!
//! So an input is in the log before the world has it, and one write to the
//! disk serves a whole batch. A frame is a batch of its own, because the
//! pacer decides its `skip` list only when the frame before it has run.

use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, oneshot};
use unsup_kernel::{Input, Output, REPORT_KIND, TickClock, World};
use unsup_protocol::{
    AdminCommand, AdminReply, FrameOutput, Observation, ObserveOpts, RealmId, ResidentId, SimError,
    ViewQuery, WaitFilter, WorldView,
};

use crate::journal::{Journal, JournalError, Pacer, Rollback, RollbackError};
use crate::mailbox::{Answer, Command, Counters, InputReply, Reply};
use crate::replier::Replier;
use crate::stats::{FrameWindow, RateMeter, micros};
use crate::waiters::{Waiter, Waiters};
use crate::{RollbackReport, RuntimeConfig, SimStats};

/// The answer to `AdminCommand::RollbackZone`.
const ZONE_ROLLBACK_REFUSED: &str = "A single zone cannot be rolled back. Money and goods cross \
    zone borders, so the totals of the world would no longer balance. Roll back the whole world \
    to a snapshot instead.";

/// A call that does not change the world.
enum Read {
    Observe {
        resident: ResidentId,
        opts: ObserveOpts,
        reply: Reply<Observation>,
    },
    Wait {
        resident: ResidentId,
        filter: WaitFilter,
        since: Option<u64>,
        timeout: Duration,
        reply: Reply<Observation>,
    },
    View {
        query: ViewQuery,
        reply: Reply<WorldView>,
    },
    Stats {
        reply: Reply<SimStats>,
    },
}

/// A call of the current batch.
enum Pending {
    /// An input that the journal recorded. It is applied after the commit.
    Input { input: Input, reply: InputReply },
    /// A read. It is answered in its place between the inputs.
    Read(Read),
}

/// A call that cannot share a batch with other calls.
enum Alone {
    Step { frames: u32, reply: Reply<u64> },
    Rollback { frame: u64, reply: oneshot::Sender<Result<RollbackReport, RollbackError>> },
    Shutdown,
}

/// Everything that the simulation thread owns.
struct Sim<'a> {
    world: World,
    journal: Box<dyn Journal>,
    pacer: Box<dyn Pacer>,
    clock: TickClock,
    tick_interval: Duration,
    frames: &'a broadcast::Sender<Arc<FrameOutput>>,
    counters: &'a Counters,
    mailbox_capacity: usize,
    waiters: Waiters,
    max_waiters: usize,
    max_waiters_per_resident: usize,
    /// The calls of the current batch, in the order of arrival.
    batch: Vec<Pending>,
    /// The answers of the current batch that are ready.
    answers: Vec<Answer>,
    /// The thread that sends the answers of large batches. `None` when the
    /// operating system could not start it; the answers are then sent here.
    replier: Option<Replier>,
    /// A batch with this many answers or more goes to the reply thread.
    reply_handoff: usize,
    /// Inputs of the current batch that the journal recorded.
    recorded: usize,
    /// Inputs that the journal has and the world has not. After a panic it
    /// tells the journal what to take back.
    unapplied: usize,
    inputs_applied: u64,
    frames_run: u64,
    frame_time_total: Duration,
    frame_time_worst: Duration,
    frame_time_last: Duration,
    frame_window: FrameWindow,
    input_rate: RateMeter,
    journal_errors: u64,
    /// The journal refused the last inputs that it was given.
    journal_failing: bool,
    waiters_full: u64,
    zone_ticks_skipped: u64,
    rollbacks: u64,
    intake_time: Duration,
    commit_time: Duration,
    apply_time: Duration,
    reply_time: Duration,
}

/// Returns true when an observation holds news that the filter lets
/// through.
fn has_news(observation: &Observation, filter: &WaitFilter) -> bool {
    let reports = observation
        .sections
        .get("reports")
        .and_then(|value| value.as_array())
        .is_some_and(|list| !list.is_empty());
    match filter {
        WaitFilter::Any => reports || !observation.notices.is_empty(),
        WaitFilter::ActionDone => reports,
        WaitFilter::Kinds { kinds } => {
            observation.notices.iter().any(|notice| kinds.contains(&notice.kind))
                || (reports && kinds.iter().any(|kind| kind == REPORT_KIND))
        }
    }
}

impl Sim<'_> {
    // ---- Batches --------------------------------------------------------------

    /// Gives an input to the journal and puts it into the batch. When the
    /// journal refuses it, the caller gets `Overloaded` at once.
    fn record(&mut self, input: Input, reply: InputReply) {
        match self.journal.record(&input) {
            Ok(()) => {
                self.recorded += 1;
                self.batch.push(Pending::Input { input, reply });
            }
            Err(error) => {
                self.journal_refused(&error, 1);
                reply.refuse(SimError::Overloaded).send();
            }
        }
    }

    /// Counts inputs that the journal did not take. The reason goes to the
    /// log once, when the journal starts to fail: a disk that stays full
    /// must not fill the log of the server too.
    fn journal_refused(&mut self, error: &JournalError, inputs: u64) {
        self.journal_errors += inputs;
        if !self.journal_failing {
            self.journal_failing = true;
            tracing::error!(%error, "the journal refuses inputs; their callers get `overloaded`");
        }
    }

    /// Notes that the journal took inputs.
    fn journal_took(&mut self) {
        if self.journal_failing {
            self.journal_failing = false;
            tracing::info!("the journal takes inputs again");
        }
    }

    /// Takes one call into the current batch. A call that cannot share a
    /// batch (a frame, a rollback, the stop) comes back: the caller ends
    /// the batch and then gives it to [`Sim::alone`].
    fn intake(&mut self, command: Command) -> Option<Alone> {
        match command {
            Command::Spawn { spec, reply } => {
                self.record(Input::Spawn(spec), InputReply::Spawn(reply));
            }
            Command::Act { resident, request, reply } => {
                self.record(Input::Act { resident, request }, InputReply::Act(reply));
            }
            Command::Admin { command: AdminCommand::RollbackZone { .. }, reply } => {
                // Not an input: the world does not change and the log does
                // not need it.
                let _ = reply.send(Ok(AdminReply::Error { message: ZONE_ROLLBACK_REFUSED.into() }));
            }
            Command::Admin { command, reply } => {
                self.record(Input::Admin(command), InputReply::Admin(reply));
            }
            Command::Deliver { reply, .. } if self.world.realm(RealmId::Global).is_none() => {
                // Another process hosts the global realm. This runtime has
                // no way to send the messages there, so it does not take
                // them.
                let _ = reply.send(Err(SimError::NotHosted));
            }
            Command::Deliver { messages, reply } => {
                self.record(Input::Deliver { messages }, InputReply::Deliver(reply));
            }
            Command::Observe { resident, opts, reply } => {
                self.batch.push(Pending::Read(Read::Observe { resident, opts, reply }));
            }
            Command::Wait { resident, filter, since, timeout, reply } => {
                self.batch.push(Pending::Read(Read::Wait {
                    resident,
                    filter,
                    since,
                    timeout,
                    reply,
                }));
            }
            Command::View { query, reply } => {
                self.batch.push(Pending::Read(Read::View { query, reply }));
            }
            Command::Stats { reply } => self.batch.push(Pending::Read(Read::Stats { reply })),
            Command::Step { frames, reply } => return Some(Alone::Step { frames, reply }),
            Command::Rollback { frame, reply } => return Some(Alone::Rollback { frame, reply }),
            Command::Shutdown => return Some(Alone::Shutdown),
        }
        None
    }

    /// Ends the current batch: commits its inputs, applies them and answers
    /// every call.
    fn finish_batch(&mut self) {
        if self.batch.is_empty() {
            return;
        }
        let start = Instant::now();
        let committed = self.recorded == 0
            || match self.journal.commit() {
                Ok(()) => {
                    self.journal_took();
                    true
                }
                Err(error) => {
                    self.journal_refused(&error, self.recorded as u64);
                    false
                }
            };
        let commit_done = Instant::now();
        self.unapplied = if committed { self.recorded } else { 0 };
        self.recorded = 0;
        // The buffers keep their allocation from batch to batch.
        let mut batch = std::mem::take(&mut self.batch);
        let mut answers = std::mem::take(&mut self.answers);
        for pending in batch.drain(..) {
            match pending {
                Pending::Input { input, reply } if committed => {
                    let output = self.world.apply(input);
                    self.unapplied -= 1;
                    self.inputs_applied += 1;
                    answers.push(reply.answer(output));
                }
                Pending::Input { reply, .. } => answers.push(reply.refuse(SimError::Overloaded)),
                Pending::Read(read) => self.read(read, &mut answers),
            }
        }
        let apply_done = Instant::now();
        match &self.replier {
            // A large batch means load: another thread wakes the callers
            // while this one takes the next batch.
            Some(replier) if answers.len() >= self.reply_handoff => {
                replier.send(std::mem::take(&mut answers));
            }
            _ => {
                for answer in answers.drain(..) {
                    answer.send();
                }
            }
        }
        self.batch = batch;
        self.answers = answers;
        self.commit_time += commit_done - start;
        self.apply_time += apply_done - commit_done;
        self.reply_time += apply_done.elapsed();
    }

    /// Answers a call that does not change the world.
    fn read(&mut self, read: Read, answers: &mut Vec<Answer>) {
        match read {
            Read::Observe { resident, opts, reply } => {
                answers.push(Answer::Observation(reply, self.world.observe(resident, &opts)));
            }
            Read::Wait { resident, filter, since, timeout, reply } => {
                self.start_wait(resident, filter, since, timeout, reply);
            }
            Read::View { query, reply } => {
                answers.push(Answer::View(reply, self.world.view(&query)));
            }
            Read::Stats { reply } => answers.push(Answer::Stats(reply, Ok(self.stats()))),
        }
    }

    /// Handles a call that is a batch of its own. Returns false when the
    /// thread must stop.
    fn alone(&mut self, command: Alone) -> bool {
        match command {
            Alone::Step { frames, reply } => {
                let mut answer = Ok(self.world.frame());
                for _ in 0..frames {
                    if !self.step_frame() {
                        answer = Err(SimError::Overloaded);
                        break;
                    }
                    answer = Ok(self.world.frame());
                }
                let _ = reply.send(answer);
            }
            Alone::Rollback { frame, reply } => {
                let _ = reply.send(self.rollback(frame));
            }
            Alone::Shutdown => return false,
        }
        true
    }

    // ---- Frames ---------------------------------------------------------------

    /// Runs one frame, publishes it and wakes the calls that wait for its
    /// news. Returns false when the journal refused the frame.
    fn step_frame(&mut self) -> bool {
        let skip = self.pacer.skip(self.world.timings(), self.tick_interval);
        let skipped = skip.len() as u64;
        let input = Input::Step { skip };
        if let Err(error) = self.journal.record(&input).and_then(|()| self.journal.commit()) {
            self.journal_refused(&error, 1);
            return false;
        }
        self.journal_took();
        self.unapplied = 1;
        let start = Instant::now();
        let output = self.world.apply(input);
        let elapsed = start.elapsed();
        self.unapplied = 0;
        self.inputs_applied += 1;
        let Output::Frame(frame) = output else {
            return false;
        };
        self.frames_run += 1;
        self.frame_time_total += elapsed;
        self.frame_time_worst = self.frame_time_worst.max(elapsed);
        self.frame_time_last = elapsed;
        self.frame_window.push(micros(elapsed));
        self.zone_ticks_skipped += skipped;

        self.journal.frame_done(&self.world, &frame);
        for &resident in &frame.woke {
            self.wake(resident);
        }
        // A send fails only when nobody subscribed. That is fine.
        let _ = self.frames.send(Arc::new(frame));
        true
    }

    // ---- Rollback ---------------------------------------------------------------

    /// Replaces the world with the newest snapshot at or before `frame`.
    fn rollback(&mut self, frame: u64) -> Result<RollbackReport, RollbackError> {
        let Rollback { mut world, frame, dropped_inputs } = self.journal.rollback(frame)?;
        world.set_clock(Some(Arc::clone(&self.clock)));
        self.world = world;
        self.rollbacks += 1;
        // The notice cursors of the calls that wait belong to a history
        // that no longer exists. Every call gets an answer now and the
        // caller starts again from the world as it is.
        for (resident, waiter) in self.waiters.drain() {
            let _ = waiter.reply.send(self.observation(resident, waiter.since));
        }
        Ok(RollbackReport { frame, dropped_inputs })
    }

    // ---- Waiting calls ----------------------------------------------------------

    /// Builds the observation that a waiting call gets. A cursor that is
    /// ahead of the inbox (after a rollback) counts as "now".
    fn observation(&self, resident: ResidentId, since: u64) -> Result<Observation, SimError> {
        let since = since.min(self.world.next_notice(resident)?);
        self.world.observe(resident, &ObserveOpts { since_notice: since, ..ObserveOpts::default() })
    }

    /// Answers the waiting calls of a resident whose filter lets the new
    /// notices through. The others keep waiting.
    fn wake(&mut self, resident: ResidentId) {
        let list = self.waiters.take(resident);
        if list.is_empty() {
            return;
        }
        let mut still = Vec::new();
        for waiter in list {
            match self.observation(resident, waiter.since) {
                Ok(observation) if !has_news(&observation, &waiter.filter) => still.push(waiter),
                answer => {
                    // The caller may have gone away. That is fine.
                    let _ = waiter.reply.send(answer);
                }
            }
        }
        self.waiters.put_back(resident, still);
    }

    /// Answers every waiting call whose time is up, with an observation
    /// that holds whatever arrived since the call.
    fn expire(&mut self, now: Instant) {
        while let Some((resident, waiter)) = self.waiters.pop_expired(now) {
            let _ = waiter.reply.send(self.observation(resident, waiter.since));
        }
    }

    fn start_wait(
        &mut self,
        resident: ResidentId,
        filter: WaitFilter,
        since: Option<u64>,
        timeout: Duration,
        reply: Reply<Observation>,
    ) {
        let next = match self.world.next_notice(resident) {
            Ok(next) => next,
            Err(e) => {
                let _ = reply.send(Err(e));
                return;
            }
        };
        // Without a cursor only news from now on counts. A cursor that is
        // ahead of the inbox counts as "now" too.
        let since = since.map_or(next, |since| since.min(next));
        // With an older cursor from the caller, news may be there already.
        // A cursor at the end of the inbox has none, so the call can wait
        // without an observation.
        if since < next || timeout.is_zero() {
            match self.observation(resident, since) {
                Ok(observation) if !has_news(&observation, &filter) && !timeout.is_zero() => {}
                answer => {
                    let _ = reply.send(answer);
                    return;
                }
            }
        }
        // A resident has too many calls that wait: the oldest ones end now,
        // as if their time were up.
        for Waiter { since, reply, .. } in
            self.waiters.make_room(resident, self.max_waiters_per_resident)
        {
            let _ = reply.send(self.observation(resident, since));
        }
        if self.waiters.len() >= self.max_waiters {
            self.waiters_full += 1;
            let _ = reply.send(Err(SimError::Overloaded));
            return;
        }
        self.waiters.add(resident, filter, since, Instant::now() + timeout, reply);
    }

    // ---- Statistics ---------------------------------------------------------------

    fn stats(&self) -> SimStats {
        let journal = self.journal.stats();
        let [p50, p95, p99] = self.frame_window.percentiles([50, 95, 99]);
        SimStats {
            frame: self.world.frame(),
            residents: self.world.residents() as u64,
            inputs_applied: self.inputs_applied,
            frames_run: self.frames_run,
            mean_frame_micros: match u32::try_from(self.frames_run) {
                Ok(0) => 0,
                Ok(n) => micros(self.frame_time_total / n),
                Err(_) => micros(self.frame_time_total) / self.frames_run,
            },
            worst_frame_micros: micros(self.frame_time_worst),
            last_frame_micros: micros(self.frame_time_last),
            waiting: self.waiters.len() as u64,
            journal_errors: self.journal_errors,
            p50_frame_micros: p50,
            p95_frame_micros: p95,
            p99_frame_micros: p99,
            inputs_per_second: self.input_rate.per_second(),
            mailbox_depth: self.counters.depth.load(Ordering::Relaxed) as u64,
            mailbox_capacity: self.mailbox_capacity as u64,
            overloaded: self.counters.mailbox_full.load(Ordering::Relaxed) + self.waiters_full,
            dilated_zones: self.pacer.dilated_zones() as u64,
            zone_ticks_skipped: self.zone_ticks_skipped,
            journal_records: journal.records,
            journal_lag: journal.lag,
            snapshots: journal.snapshots,
            snapshot_errors: journal.snapshot_errors,
            last_snapshot_frame: journal.last_snapshot_frame,
            last_snapshot_micros: journal.last_snapshot_capture_micros,
            last_snapshot_write_micros: journal.last_snapshot_write_micros,
            last_snapshot_bytes: journal.last_snapshot_bytes,
            rollbacks: self.rollbacks,
            intake_micros: micros(self.intake_time),
            commit_micros: micros(self.commit_time),
            apply_micros: micros(self.apply_time),
            reply_micros: micros(self.reply_time),
        }
    }
}

/// The mailbox loop. It returns when the runtime must stop.
fn serve(sim: &mut Sim<'_>, mailbox: &Receiver<Command>, batch: usize) {
    let real_time = !sim.tick_interval.is_zero();
    let mut next_frame = Instant::now() + sim.tick_interval;
    loop {
        // Sleep until a message comes, a frame is due or a wait runs out.
        let wake_at = match (real_time.then_some(next_frame), sim.waiters.next_deadline()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let first = match wake_at {
            Some(at) => match mailbox.recv_timeout(at.saturating_duration_since(Instant::now())) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
            None => match mailbox.recv() {
                Ok(command) => Some(command),
                Err(_) => return,
            },
        };
        // Take a batch, so that a busy mailbox cannot starve the frames and
        // one commit of the journal serves many calls.
        if let Some(first) = first {
            let start = Instant::now();
            sim.counters.depth.fetch_sub(1, Ordering::Relaxed);
            let mut alone = sim.intake(first);
            let mut taken = 1;
            let mut disconnected = false;
            while alone.is_none() && taken < batch {
                match mailbox.try_recv() {
                    Ok(command) => {
                        sim.counters.depth.fetch_sub(1, Ordering::Relaxed);
                        taken += 1;
                        alone = sim.intake(command);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
            sim.intake_time += start.elapsed();
            sim.finish_batch();
            if alone.is_some_and(|command| !sim.alone(command)) || disconnected {
                return;
            }
        }
        if real_time && Instant::now() >= next_frame {
            sim.step_frame();
            next_frame += sim.tick_interval;
            // A frame that took longer than the interval must not make the
            // world run a burst of frames to catch up.
            let now = Instant::now();
            if next_frame < now {
                next_frame = now + sim.tick_interval;
            }
        }
        let now = Instant::now();
        sim.expire(now);
        sim.input_rate.observe(now, sim.inputs_applied);
    }
}

/// The body of the simulation thread. It returns the world when it stops.
pub(crate) fn run(
    mut world: World,
    config: RuntimeConfig,
    mailbox: &Receiver<Command>,
    frames: &broadcast::Sender<Arc<FrameOutput>>,
    counters: &Counters,
) -> World {
    // The kernel never reads the wall clock. The runtime lends it one to
    // measure zone ticks; the pacer reads the result.
    let epoch = Instant::now();
    let clock: TickClock =
        Arc::new(move || u64::try_from(epoch.elapsed().as_nanos()).unwrap_or(u64::MAX));
    world.set_clock(Some(Arc::clone(&clock)));

    let replier = match Replier::start(config.reply_queue) {
        Ok(replier) => Some(replier),
        Err(error) => {
            tracing::warn!(%error, "the reply thread did not start; the simulation thread answers itself");
            None
        }
    };
    let mut sim = Sim {
        world,
        journal: config.journal,
        pacer: config.pacer,
        clock,
        tick_interval: config.tick_interval,
        frames,
        counters,
        mailbox_capacity: config.mailbox.max(1),
        waiters: Waiters::default(),
        max_waiters: config.max_waiters,
        max_waiters_per_resident: config.max_waiters_per_resident,
        batch: Vec::new(),
        answers: Vec::new(),
        replier,
        reply_handoff: config.reply_handoff.max(1),
        recorded: 0,
        unapplied: 0,
        inputs_applied: 0,
        frames_run: 0,
        frame_time_total: Duration::ZERO,
        frame_time_worst: Duration::ZERO,
        frame_time_last: Duration::ZERO,
        frame_window: FrameWindow::new(config.stats_window),
        input_rate: RateMeter::new(config.rate_window, epoch),
        journal_errors: 0,
        journal_failing: false,
        waiters_full: 0,
        zone_ticks_skipped: 0,
        rollbacks: 0,
        intake_time: Duration::ZERO,
        commit_time: Duration::ZERO,
        apply_time: Duration::ZERO,
        reply_time: Duration::ZERO,
    };
    let batch = config.batch.max(1);
    // A panic of a plugin or of the kernel ends the thread. The journal
    // first takes back the inputs that the world never got, so that a
    // recovery does not meet the input that caused the panic. The world is
    // not touched again after a panic.
    match catch_unwind(AssertUnwindSafe(|| serve(&mut sim, mailbox, batch))) {
        Ok(()) => sim.journal.shutdown(&sim.world),
        Err(panic) => {
            sim.journal.discard(sim.unapplied);
            resume_unwind(panic);
        }
    }
    // Dropping the waiters drops their reply senders: the callers get
    // `SimError::Shutdown`.
    sim.world.set_clock(None);
    sim.world
}
