//! The mailbox loop of the coordinator: the same calls as the simulation
//! thread of one process, answered by the cluster.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, sync_channel};
use std::time::{Duration, Instant};

use tokio::sync::broadcast;
use unsup_kernel::{World, ZoneTiming};
use unsup_protocol::{AdminCommand, AdminReply, FrameOutput, SimError, ZoneId};

use super::super::call::ClusterCall;
use super::super::config::ClusterConfig;
use super::super::error::ClusterError;
use super::super::handle::{ClusterHandle, ClusterJoin};
use super::super::link::{Alarm, Link};
use super::super::proto::{FromWorker, SealMark, ToWorker};
use super::core::Core;
use super::frame::FrameTimes;
use crate::external::PostError;
use crate::handle::SimHandle;
use crate::journal::{Pacer, RollbackError};
use crate::mailbox::{Command, Counters};
use crate::sim::ZONE_ROLLBACK_REFUSED;
use crate::stats::{FrameWindow, RateMeter, micros};
use crate::waiters::Waiters;

/// The coordinator while the cluster runs.
pub(crate) struct Server {
    pub core: Core,
    pub frames: broadcast::Sender<Arc<FrameOutput>>,
    pub counters: Arc<Counters>,
    pub alarm: Alarm,
    pub pacer: Box<dyn Pacer>,
    pub tick_interval: Duration,
    pub mailbox_capacity: usize,
    pub waiters: Waiters,
    pub max_waiters: usize,
    pub max_waiters_per_resident: usize,
    pub frame_window: FrameWindow,
    pub input_rate: RateMeter,
    pub meters: Meters,
}

/// Counters and times of the coordinator.
#[derive(Default)]
pub(crate) struct Meters {
    /// Tick time of each zone in the last frame.
    pub(super) timings: Vec<ZoneTiming>,
    pub(super) inputs_applied: u64,
    pub(super) refused: u64,
    pub(super) waiters_full: u64,
    pub(super) frames_run: u64,
    pub(super) zone_ticks_skipped: u64,
    pub(super) frame_total: Duration,
    pub(super) frame_worst: Duration,
    pub(super) frame_last: Duration,
    pub(super) last: FrameTimes,
    pub(super) zones_total: Duration,
    pub(super) global_total: Duration,
    pub(super) exchange_total: Duration,
}

/// Tells every worker that the cluster stops, and why.
pub(crate) fn stop_all(links: &mut [Link], error: &ClusterError) {
    let stop = ToWorker::Stop { reason: error.to_string() };
    for link in links {
        // A worker that is gone cannot be told.
        let _ = link.send(&stop);
    }
}

/// The answer of a world, or the refusal of a journal.
fn refused<T>(answer: &Result<T, SimError>) -> bool {
    matches!(answer, Err(SimError::Overloaded))
}

impl Server {
    /// Starts the coordinator thread for a cluster that is ready, and
    /// returns the handle and the owner of the thread.
    pub(crate) fn spawn(
        core: Core,
        config: ClusterConfig,
        alarm: Alarm,
    ) -> Result<(ClusterHandle, ClusterJoin), ClusterError> {
        let mailbox_capacity = config.mailbox.max(1);
        let (tx, rx) = sync_channel(mailbox_capacity);
        let (frames, _) = broadcast::channel(config.frame_buffer.max(1));
        let counters = Arc::new(Counters::default());
        // From now on a reader thread wakes the coordinator when its
        // connection ends.
        alarm.arm(tx.clone(), Arc::clone(&counters));
        let handle = ClusterHandle {
            sim: SimHandle {
                tx: tx.clone(),
                frames: frames.downgrade(),
                catalogue: Arc::new(core.global.world.catalogue()),
                counters: Arc::clone(&counters),
            },
        };
        let server = Server {
            core,
            frames,
            counters: Arc::clone(&counters),
            alarm,
            pacer: config.pacer,
            tick_interval: config.tick_interval,
            mailbox_capacity,
            waiters: Waiters::default(),
            max_waiters: config.max_waiters,
            max_waiters_per_resident: config.max_waiters_per_resident,
            frame_window: FrameWindow::new(config.stats_window),
            input_rate: RateMeter::new(config.rate_window, Instant::now()),
            meters: Meters::default(),
        };
        let thread = std::thread::Builder::new()
            .name("unsup-cluster".into())
            .spawn(move || server.run(&rx))
            .map_err(|e| ClusterError::Io(format!("the coordinator thread did not start: {e}")))?;
        Ok((handle, ClusterJoin { tx, counters, thread: Some(thread) }))
    }

    /// Serves the mailbox until the cluster stops. Returns the world of
    /// the global realm after a stop in order.
    fn run(mut self, mailbox: &Receiver<Command>) -> Result<World, ClusterError> {
        let served = self.serve(mailbox);
        // The reader threads must not keep the mailbox open.
        self.alarm.disarm();
        match served {
            Ok(()) => self.stop_in_order(),
            Err(error) => {
                tracing::error!(%error, "the cluster stops");
                stop_all(&mut self.core.links, &error);
                Err(error)
            }
        }
    }

    /// Ends the cluster in order: one more seal when inputs came after the
    /// last one, then every process writes its final snapshots.
    fn stop_in_order(mut self) -> Result<World, ClusterError> {
        // A worker that is known to be dead makes this a failure, before
        // any process writes the seal of a stop in order.
        let alive = self.core.links.iter_mut().try_for_each(Link::check);
        if let Err(error) = alive {
            stop_all(&mut self.core.links, &error);
            return Err(error);
        }
        let seal =
            self.core.dirty.then(|| SealMark { epoch: self.core.take_epoch(), frame_done: false });
        let order = ToWorker::Shutdown { seal };
        for link in &mut self.core.links {
            link.send(&order)?;
        }
        for link in &mut self.core.links {
            match link.recv(self.core.reply_timeout)? {
                FromWorker::Bye => {}
                other => {
                    return Err(ClusterError::Protocol(format!(
                        "worker {} answered `{}` to the stop",
                        link.slot,
                        other.name()
                    )));
                }
            }
        }
        if let Some(mark) = seal {
            self.core.global.seal(mark.epoch, false).map_err(|e| Core::global_failed(&e))?;
        }
        Ok(self.core.global.finish())
    }

    fn serve(&mut self, mailbox: &Receiver<Command>) -> Result<(), ClusterError> {
        let real_time = !self.tick_interval.is_zero();
        let mut next_frame = Instant::now() + self.tick_interval;
        loop {
            // Sleep until a call comes, a frame is due or a wait runs out.
            let wake_at = match (real_time.then_some(next_frame), self.waiters.next_deadline()) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let command = match wake_at {
                Some(at) => {
                    match mailbox.recv_timeout(at.saturating_duration_since(Instant::now())) {
                        Ok(command) => Some(command),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => return Ok(()),
                    }
                }
                None => match mailbox.recv() {
                    Ok(command) => Some(command),
                    Err(_) => return Ok(()),
                },
            };
            if let Some(command) = command {
                self.counters.depth.fetch_sub(1, Ordering::Relaxed);
                // No call is answered by a cluster that lost a worker. The
                // reader threads know of a dead connection before any
                // question goes out.
                self.core.links.iter_mut().try_for_each(Link::check)?;
                if !self.handle(command)? {
                    return Ok(());
                }
            }
            if real_time && Instant::now() >= next_frame {
                self.run_frame()?;
                next_frame += self.tick_interval;
                // A frame that took longer than the interval must not make
                // the world run a burst of frames to catch up.
                let now = Instant::now();
                if next_frame < now {
                    next_frame = now + self.tick_interval;
                }
            }
            let now = Instant::now();
            self.expire(now)?;
            self.input_rate.observe(now, self.meters.inputs_applied);
        }
    }

    /// Counts an input by its answer.
    fn count<T>(&mut self, answer: &Result<T, SimError>) {
        if refused(answer) {
            self.meters.refused += 1;
        } else {
            self.meters.inputs_applied += 1;
        }
    }

    /// Handles one call. Returns false when the cluster must stop in
    /// order. An error stops the cluster; the caller of the call then sees
    /// its reply go away, which is `SimError::Shutdown`.
    fn handle(&mut self, command: Command) -> Result<bool, ClusterError> {
        match command {
            Command::Spawn { spec, reply } => {
                let answer = self.core.spawn(spec)?;
                self.count(&answer);
                let _ = reply.send(answer);
            }
            Command::Act { resident, request, reply } => {
                let answer = self.core.act(resident, request)?;
                self.count(&answer);
                let _ = reply.send(answer);
            }
            Command::Admin { command: AdminCommand::RollbackZone { .. }, reply } => {
                // Not an input: no world changes and no log needs it.
                let _ = reply.send(Ok(AdminReply::Error { message: ZONE_ROLLBACK_REFUSED.into() }));
            }
            Command::Admin { command, reply } => {
                let answer = self.core.admin(command)?;
                self.count(&answer);
                let _ = reply.send(answer);
            }
            Command::Deliver { messages, reply } => {
                let answer = self.core.post(messages)?;
                match &answer {
                    Ok(()) => self.meters.inputs_applied += 1,
                    Err(PostError::Sim(SimError::Overloaded)) => self.meters.refused += 1,
                    // A message that nobody handles never was an input.
                    Err(_) => {}
                }
                let _ = reply.send(answer);
            }
            Command::Observe { resident, opts, reply } => {
                let _ = reply.send(self.core.observe(resident, opts)?);
            }
            Command::Wait { resident, filter, since, timeout, reply } => {
                self.start_wait(resident, filter, since, timeout, reply)?;
            }
            Command::View { query, reply } => {
                let _ = reply.send(self.core.view(&query)?);
            }
            Command::Stats { reply } => {
                let _ = reply.send(Ok(self.stats()));
            }
            Command::Step { frames, reply } => {
                for _ in 0..frames {
                    self.run_frame()?;
                }
                let _ = reply.send(Ok(self.core.global.world.frame()));
            }
            Command::Rollback { reply, .. } => {
                // A rollback of a cluster would need one snapshot of the
                // same seal in every process.
                let _ = reply.send(Err(RollbackError::Unsupported));
            }
            Command::Shutdown => return Ok(false),
            Command::Cluster(call) => self.cluster_call(call)?,
        }
        Ok(true)
    }

    fn cluster_call(&mut self, call: ClusterCall) -> Result<(), ClusterError> {
        match call {
            ClusterCall::Check => {
                for link in &mut self.core.links {
                    link.check()?;
                }
            }
            ClusterCall::Stats { reply } => {
                let _ = reply.send(Ok(self.cluster_stats()));
            }
            ClusterCall::Hashes { reply } => {
                let _ = reply.send(Ok(self.core.hashes()?));
            }
            ClusterCall::Snapshots { reply } => {
                let _ = reply.send(Ok(self.core.snapshots()?));
            }
            ClusterCall::Audit { reply } => {
                let _ = reply.send(self.core.audit()?);
            }
        }
        Ok(())
    }

    // ---- Frames ---------------------------------------------------------------

    /// Runs one frame, publishes it and wakes the calls that wait for its
    /// news.
    fn run_frame(&mut self) -> Result<(), ClusterError> {
        let skip = self.pacer.skip(&self.meters.timings, self.tick_interval);
        let skipped = skip.len() as u64;
        let start = Instant::now();
        let framed = self.core.frame(skip)?;
        let elapsed = start.elapsed();

        let m = &mut self.meters;
        // The frame is one input of every realm.
        m.inputs_applied += 1;
        m.frames_run += 1;
        m.zone_ticks_skipped += skipped;
        m.frame_total += elapsed;
        m.frame_worst = m.frame_worst.max(elapsed);
        m.frame_last = elapsed;
        m.zones_total += framed.times.zones;
        m.global_total += framed.times.global;
        m.exchange_total += framed.times.exchange;
        m.last = framed.times;
        m.timings = framed
            .elapsed
            .iter()
            .enumerate()
            .map(|(zone, &elapsed)| ZoneTiming { zone: ZoneId(zone as u32), elapsed })
            .collect();
        self.frame_window.push(micros(elapsed));

        self.wake(&framed.output.woke)?;
        // A send fails only when nobody subscribed. That is fine.
        let _ = self.frames.send(Arc::new(framed.output));
        Ok(())
    }
}
