//! The simulation thread: the mailbox loop that owns the world.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, oneshot};
use unsup_kernel::{Input, Output, REPORT_KIND, World};
use unsup_protocol::{
    ActionAck, ActionRequest, AdminCommand, AdminReply, FrameOutput, Observation, ObserveOpts,
    ResidentId, SimError, SpawnSpec, ViewQuery, WaitFilter, WorldView,
};

use crate::journal::{Journal, Pacer};
use crate::{RuntimeConfig, SimStats};

type Reply<T> = oneshot::Sender<Result<T, SimError>>;

/// One message in the mailbox of the simulation thread.
pub(crate) enum Command {
    Spawn {
        spec: SpawnSpec,
        reply: Reply<ResidentId>,
    },
    Act {
        resident: ResidentId,
        request: ActionRequest,
        reply: Reply<ActionAck>,
    },
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
    Admin {
        command: AdminCommand,
        reply: Reply<AdminReply>,
    },
    Stats {
        reply: Reply<SimStats>,
    },
    Step {
        frames: u32,
        reply: Reply<u64>,
    },
    Shutdown,
}

/// A `wait` call that has not been answered yet.
struct Waiter {
    filter: WaitFilter,
    /// News with a sequence number at or above this value counts.
    since: u64,
    deadline: Instant,
    reply: Reply<Observation>,
}

/// Everything that the simulation thread owns.
struct Sim<'a> {
    world: World,
    journal: Box<dyn Journal>,
    pacer: Box<dyn Pacer>,
    tick_interval: Duration,
    frames: &'a broadcast::Sender<Arc<FrameOutput>>,
    /// Waiting calls by resident, in arrival order.
    waiters: BTreeMap<ResidentId, Vec<Waiter>>,
    waiting: u64,
    inputs_applied: u64,
    frames_run: u64,
    frame_time_total: Duration,
    frame_time_worst: Duration,
    frame_time_last: Duration,
    journal_errors: u64,
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
    /// Records an input and applies it. Returns `Overloaded` when the
    /// journal cannot record it; the world is then unchanged.
    fn apply(&mut self, input: Input) -> Result<Output, SimError> {
        if self.journal.record(&input).is_err() {
            self.journal_errors += 1;
            return Err(SimError::Overloaded);
        }
        self.inputs_applied += 1;
        Ok(self.world.apply(input))
    }

    /// Runs one frame, publishes it and wakes the calls that wait for its
    /// news. Returns false when the journal refused the frame.
    fn step_frame(&mut self) -> bool {
        let skip = self.pacer.skip(self.world.timings(), self.tick_interval);
        let start = Instant::now();
        let frame = match self.apply(Input::Step { skip }) {
            Ok(Output::Frame(frame)) => frame,
            _ => return false,
        };
        let elapsed = start.elapsed();
        self.frames_run += 1;
        self.frame_time_total += elapsed;
        self.frame_time_worst = self.frame_time_worst.max(elapsed);
        self.frame_time_last = elapsed;

        self.journal.frame_done(&self.world, &frame);
        for &resident in &frame.woke {
            self.wake(resident);
        }
        // A send fails only when nobody subscribed. That is fine.
        let _ = self.frames.send(Arc::new(frame));
        true
    }

    /// Builds the observation that a waiting call gets.
    fn observation(&self, resident: ResidentId, since: u64) -> Result<Observation, SimError> {
        self.world.observe(resident, &ObserveOpts { since_notice: since, ..ObserveOpts::default() })
    }

    /// Answers the waiting calls of a resident whose filter lets the new
    /// notices through. The others keep waiting.
    fn wake(&mut self, resident: ResidentId) {
        let Some(list) = self.waiters.remove(&resident) else {
            return;
        };
        let mut still = Vec::new();
        for waiter in list {
            match self.observation(resident, waiter.since) {
                Ok(observation) if !has_news(&observation, &waiter.filter) => still.push(waiter),
                answer => {
                    self.waiting -= 1;
                    // The caller may have gone away. That is fine.
                    let _ = waiter.reply.send(answer);
                }
            }
        }
        if !still.is_empty() {
            self.waiters.insert(resident, still);
        }
    }

    /// Answers every waiting call whose time is up, with an observation
    /// that holds whatever arrived since the call.
    fn expire(&mut self, now: Instant) {
        let due: Vec<ResidentId> = self
            .waiters
            .iter()
            .filter(|(_, list)| list.iter().any(|w| w.deadline <= now))
            .map(|(&resident, _)| resident)
            .collect();
        for resident in due {
            let Some(list) = self.waiters.remove(&resident) else {
                continue;
            };
            let mut still = Vec::new();
            for waiter in list {
                if waiter.deadline <= now {
                    self.waiting -= 1;
                    let _ = waiter.reply.send(self.observation(resident, waiter.since));
                } else {
                    still.push(waiter);
                }
            }
            if !still.is_empty() {
                self.waiters.insert(resident, still);
            }
        }
    }

    /// Returns the earliest time at which a waiting call runs out.
    fn next_deadline(&self) -> Option<Instant> {
        self.waiters.values().flatten().map(|w| w.deadline).min()
    }

    fn stats(&self) -> SimStats {
        let micros = |d: Duration| u64::try_from(d.as_micros()).unwrap_or(u64::MAX);
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
            waiting: self.waiting,
            journal_errors: self.journal_errors,
        }
    }

    /// Handles one message. Returns false when the thread must stop.
    fn handle(&mut self, command: Command) -> bool {
        // A reply fails only when the caller went away. That is fine.
        match command {
            Command::Spawn { spec, reply } => {
                let answer = self.apply(Input::Spawn(spec)).and_then(|output| match output {
                    Output::Spawned(id) => Ok(id),
                    Output::Error(e) => Err(e),
                    _ => Err(SimError::Invalid),
                });
                let _ = reply.send(answer);
            }
            Command::Act { resident, request, reply } => {
                let answer =
                    self.apply(Input::Act { resident, request }).and_then(|output| match output {
                        Output::Ack(ack) => Ok(ack),
                        Output::Error(e) => Err(e),
                        _ => Err(SimError::Invalid),
                    });
                let _ = reply.send(answer);
            }
            Command::Observe { resident, opts, reply } => {
                let _ = reply.send(self.world.observe(resident, &opts));
            }
            Command::Wait { resident, filter, since, timeout, reply } => {
                self.start_wait(resident, filter, since, timeout, reply);
            }
            Command::View { query, reply } => {
                let _ = reply.send(self.world.view(&query));
            }
            Command::Admin { command, reply } => {
                let answer = self.apply(Input::Admin(command)).and_then(|output| match output {
                    Output::Admin(admin_reply) => Ok(admin_reply),
                    Output::Error(e) => Err(e),
                    _ => Err(SimError::Invalid),
                });
                let _ = reply.send(answer);
            }
            Command::Stats { reply } => {
                let _ = reply.send(Ok(self.stats()));
            }
            Command::Step { frames, reply } => {
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
            Command::Shutdown => return false,
        }
        true
    }

    fn start_wait(
        &mut self,
        resident: ResidentId,
        filter: WaitFilter,
        since: Option<u64>,
        timeout: Duration,
        reply: Reply<Observation>,
    ) {
        let since = match since {
            Some(since) => since,
            None => match self.world.next_notice(resident) {
                Ok(next) => next,
                Err(e) => {
                    let _ = reply.send(Err(e));
                    return;
                }
            },
        };
        // With a cursor from the caller, news may be there already.
        match self.observation(resident, since) {
            Ok(observation) if !has_news(&observation, &filter) && !timeout.is_zero() => {}
            answer => {
                let _ = reply.send(answer);
                return;
            }
        }
        let deadline = Instant::now() + timeout;
        self.waiters.entry(resident).or_default().push(Waiter { filter, since, deadline, reply });
        self.waiting += 1;
    }
}

/// The body of the simulation thread. It returns the world when it stops.
pub(crate) fn run(
    mut world: World,
    config: RuntimeConfig,
    mailbox: &Receiver<Command>,
    frames: &broadcast::Sender<Arc<FrameOutput>>,
) -> World {
    let RuntimeConfig { tick_interval, batch, journal, pacer, .. } = config;
    // The kernel never reads the wall clock. The runtime lends it one to
    // measure zone ticks; the pacer reads the result.
    let epoch = Instant::now();
    world.set_clock(Some(Arc::new(move || {
        u64::try_from(epoch.elapsed().as_nanos()).unwrap_or(u64::MAX)
    })));

    let mut sim = Sim {
        world,
        journal,
        pacer,
        tick_interval,
        frames,
        waiters: BTreeMap::new(),
        waiting: 0,
        inputs_applied: 0,
        frames_run: 0,
        frame_time_total: Duration::ZERO,
        frame_time_worst: Duration::ZERO,
        frame_time_last: Duration::ZERO,
        journal_errors: 0,
    };
    let real_time = !tick_interval.is_zero();
    let mut next_frame = Instant::now() + tick_interval;

    'running: loop {
        // Sleep until a message comes, a frame is due or a wait runs out.
        let wake_at = match (real_time.then_some(next_frame), sim.next_deadline()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let first = match wake_at {
            Some(at) => match mailbox.recv_timeout(at.saturating_duration_since(Instant::now())) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break 'running,
            },
            None => match mailbox.recv() {
                Ok(command) => Some(command),
                Err(_) => break 'running,
            },
        };
        // Handle a small batch, so that a busy mailbox cannot starve the
        // frames.
        if let Some(command) = first {
            if !sim.handle(command) {
                break 'running;
            }
            for _ in 1..batch.max(1) {
                match mailbox.try_recv() {
                    Ok(command) => {
                        if !sim.handle(command) {
                            break 'running;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break 'running,
                }
            }
        }
        let now = Instant::now();
        if real_time && now >= next_frame {
            sim.step_frame();
            next_frame += tick_interval;
            // A frame that took longer than the interval must not make the
            // world run a burst of frames to catch up.
            let now = Instant::now();
            if next_frame < now {
                next_frame = now + tick_interval;
            }
        }
        sim.expire(Instant::now());
    }
    // Dropping the waiters drops their reply senders: the callers get
    // `SimError::Shutdown`.
    sim.world.set_clock(None);
    sim.world
}
