//! The thread that hands answers to callers under load.
//!
//! An answer wakes the task of its caller on another thread. That costs
//! about as much as applying a small input. When a batch is large, the
//! simulation thread gives all its answers to this thread in one step and
//! goes on with the next batch.

use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;

use crate::mailbox::Answer;

/// The handle to the reply thread.
pub(crate) struct Replier {
    tx: Option<SyncSender<Vec<Answer>>>,
    thread: Option<JoinHandle<()>>,
}

fn run(batches: &Receiver<Vec<Answer>>) {
    while let Ok(answers) = batches.recv() {
        for answer in answers {
            answer.send();
        }
    }
}

impl Replier {
    /// Starts the thread. It holds at most `queue` batches of answers that
    /// wait; then the simulation thread waits for it, which slows the
    /// intake down and fills the mailbox: the callers see `overloaded`.
    pub(crate) fn start(queue: usize) -> std::io::Result<Replier> {
        let (tx, rx) = sync_channel(queue.max(1));
        let thread =
            std::thread::Builder::new().name("unsup-reply".into()).spawn(move || run(&rx))?;
        Ok(Replier { tx: Some(tx), thread: Some(thread) })
    }

    /// Gives a batch of answers to the thread. When the thread is gone the
    /// answers are sent here.
    pub(crate) fn send(&self, answers: Vec<Answer>) {
        let Some(tx) = &self.tx else {
            return;
        };
        if let Err(returned) = tx.send(answers) {
            for answer in returned.0 {
                answer.send();
            }
        }
    }
}

impl Drop for Replier {
    fn drop(&mut self) {
        // Closing the channel ends the thread after the answers that it
        // already has. Every caller gets its answer before the runtime is
        // gone.
        self.tx = None;
        if let Some(thread) = self.thread.take() {
            // The thread has no result. A panic in it was already printed.
            let _ = thread.join();
        }
    }
}
