//! What every way to control a resident has in common.
//!
//! The story of a newcomer is the same over HTTPS and over the agent
//! WebSocket. [`Link`] is the part of the two that the story needs.

use serde_json::Value;

use super::{World, id};

/// The news that one link received, in the order of its arrival. It
/// checks what the protocol promises: no notice and no report comes two
/// times, and each of the two comes in the order of its `seq`.
#[derive(Debug, Default)]
pub struct Inbox {
    /// Every action report that arrived.
    pub reports: Vec<Value>,
    /// Every notice that arrived.
    pub notices: Vec<Value>,
}

impl Inbox {
    /// Takes a report.
    ///
    /// # Panics
    ///
    /// Panics when the report repeats a sequence number or comes out of
    /// order.
    pub fn report(&mut self, report: Value) {
        let seq = id(&report["seq"], "the `seq` of a report");
        self.check_new(seq);
        if let Some(last) = self.reports.last() {
            assert!(seq > id(&last["seq"], "seq"), "reports come in the order of `seq`: {report}");
        }
        self.reports.push(report);
    }

    /// Takes a notice.
    ///
    /// # Panics
    ///
    /// Panics when the notice repeats a sequence number or comes out of
    /// order.
    pub fn notice(&mut self, notice: Value) {
        let seq = id(&notice["seq"], "the `seq` of a notice");
        self.check_new(seq);
        if let Some(last) = self.notices.last() {
            assert!(seq > id(&last["seq"], "seq"), "notices come in the order of `seq`: {notice}");
        }
        self.notices.push(notice);
    }

    /// Takes the news of an observation: the section `reports` and the
    /// notices.
    pub fn observation(&mut self, observation: &Value) {
        for report in observation["sections"]["reports"].as_array().into_iter().flatten() {
            self.report(report.clone());
        }
        for notice in observation["notices"].as_array().expect("an observation has notices") {
            self.notice(notice.clone());
        }
    }

    fn check_new(&self, seq: u64) {
        let seen = self.reports.iter().chain(&self.notices).any(|news| news["seq"] == seq);
        assert!(!seen, "the sequence number {seq} came two times");
    }

    /// Returns the report of an action, when it arrived.
    pub fn report_of(&self, action: u64) -> Option<&Value> {
        self.reports.iter().find(|report| report["action_id"] == action)
    }

    /// Returns the notices of one kind.
    pub fn notices_of(&self, kind: &str) -> Vec<&Value> {
        self.notices.iter().filter(|notice| notice["kind"] == kind).collect()
    }

    /// Returns every sequence number that arrived, in ascending order.
    pub fn seqs(&self) -> Vec<u64> {
        let mut seqs: Vec<u64> =
            self.reports.iter().chain(&self.notices).map(|news| id(&news["seq"], "seq")).collect();
        seqs.sort_unstable();
        seqs
    }
}

/// One way to control a resident: HTTPS or the agent WebSocket.
pub trait Link {
    /// Sends one action request. Returns its acknowledgement.
    async fn act(&mut self, verb: &str, args: Value) -> Value;

    /// Sends a plan: a list of action requests. Returns one
    /// acknowledgement for each request that the server sent to the
    /// simulation.
    async fn plan(&mut self, requests: Vec<Value>) -> Vec<Value>;

    /// Returns an observation. `detail` is `brief` or `full`. An empty
    /// list of sections asks for all of them.
    async fn observe(&mut self, detail: &str, sections: &[&str]) -> Value;

    /// Waits until the resident has news that this link did not see yet,
    /// and puts it into the inbox. The world runs while the call waits.
    async fn news(&mut self, world: &World);

    /// Returns the news that arrived so far.
    fn inbox(&self) -> &Inbox;
}

/// Reads the state of an acknowledgement.
pub fn state(ack: &Value) -> &str {
    ack["status"]["state"].as_str().unwrap_or_else(|| panic!("an acknowledgement, not {ack}"))
}

/// Sends one action that the simulation must accept. Returns the
/// acknowledgement.
///
/// # Panics
///
/// Panics when the simulation rejects the action.
pub async fn accepted<L: Link>(link: &mut L, verb: &str, args: Value) -> Value {
    let ack = link.act(verb, args).await;
    assert_ne!(state(&ack), "rejected", "{verb}: {ack}");
    assert_eq!(ack["verb"], verb, "{ack}");
    ack
}

/// Waits for the report of an action. The world runs while the call
/// waits.
pub async fn report<L: Link>(link: &mut L, world: &World, action: u64) -> Value {
    loop {
        if let Some(report) = link.inbox().report_of(action) {
            return report.clone();
        }
        link.news(world).await;
    }
}

/// Sends one action that takes game time and waits for its report. The
/// report must say that the action went well. Returns the result of the
/// report.
///
/// # Panics
///
/// Panics when the action is rejected, ends at once or fails.
pub async fn done<L: Link>(link: &mut L, world: &World, verb: &str, args: Value) -> Value {
    let ack = accepted(link, verb, args).await;
    assert_ne!(state(&ack), "done", "`{verb}` ends later, with a report: {ack}");
    let report = report(link, world, id(&ack["action_id"], "action_id")).await;
    assert_eq!(report["ok"], true, "`{verb}` went well: {report}");
    assert_eq!(report["verb"], verb, "{report}");
    report["result"].clone()
}

/// Returns the section `self` of a fresh observation.
pub async fn myself<L: Link>(link: &mut L) -> Value {
    let observation = link.observe("brief", &["self"]).await;
    observation["sections"]["self"].clone()
}
