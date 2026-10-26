//! One thread that talks to the service on the window's behalf.
//!
//! A click must never wait on the network: the service can be busy (a batch of thousands being
//! added, a restart waiting for its transfer to stop) and a frame that blocks freezes the whole
//! window. So every command is a job for this thread. Jobs run one at a time, in the order they
//! were given — *Pause* then *Remove* happen in that order — and each may hand back a reply,
//! which the window runs on its own thread at the next frame (to show a toast, say).
//!
//! One thread rather than one per click: *Start all* on 5,000 paused downloads used to start
//! 5,000 threads. Work that can take minutes (a playlist lookup, a page scan) keeps a thread of
//! its own, so it doesn't hold up a *Pause* queued behind it.

use crate::api::Client;
use std::sync::mpsc::{Receiver, Sender};

/// What the window does with a job's result, on its own thread.
pub type Reply<T> = Box<dyn FnOnce(&mut T) + Send>;
type Job<T> = Box<dyn FnOnce(&Client) -> Option<Reply<T>> + Send>;

pub struct Worker<T> {
    jobs: Sender<Job<T>>,
    replies: Receiver<Reply<T>>,
}

impl<T: 'static> Worker<T> {
    /// `after` runs on the worker thread after each job: to wake the window, and the poller so
    /// the job's effect shows at once.
    pub fn start(client: Client, after: impl Fn() + Send + 'static) -> Worker<T> {
        let (jobs, rx) = std::sync::mpsc::channel::<Job<T>>();
        let (tx, replies) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("reidm-commands".into())
            .spawn(move || {
                for job in rx {
                    let reply = job(&client);
                    if let Some(r) = reply {
                        if tx.send(r).is_err() {
                            return;
                        }
                    }
                    after();
                }
            })
            .expect("cannot start the command thread");
        Worker { jobs, replies }
    }

    /// Queue a job.
    pub fn run(&self, job: impl FnOnce(&Client) -> Option<Reply<T>> + Send + 'static) {
        // Only fails once the thread has gone, when the window is closing anyway.
        let _ = self.jobs.send(Box::new(job));
    }

    /// Replies that have arrived, oldest first.
    pub fn replies(&self) -> Vec<Reply<T>> {
        self.replies.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn many_commands_share_one_thread_and_keep_their_order() {
        let w: Worker<Vec<(usize, std::thread::ThreadId)>> = Worker::start(Client::new(std::time::Duration::from_secs(1)), || {});
        for i in 0..5000 {
            w.run(move |_| {
                let t = std::thread::current().id();
                Some(Box::new(move |log: &mut Vec<(usize, std::thread::ThreadId)>| log.push((i, t))))
            });
        }
        let mut log = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while log.len() < 5000 && std::time::Instant::now() < deadline {
            for r in w.replies() {
                r(&mut log);
            }
            std::thread::yield_now();
        }
        assert_eq!(log.len(), 5000);
        assert!(log.windows(2).all(|p| p[0].0 < p[1].0), "in the order given");
        let threads: std::collections::HashSet<_> = log.iter().map(|(_, t)| *t).collect();
        assert_eq!(threads.len(), 1, "one thread for all of them");
        assert_ne!(log[0].1, std::thread::current().id(), "and not the window's");
    }
}
