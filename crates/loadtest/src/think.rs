//! Think time: how long a resident or an agent waits between two actions.
//!
//! The commands that run in one process count think time in ticks
//! ([`TickThink`], part of a behaviour profile). The swarm counts it in
//! seconds of real time ([`ThinkTime`]), to imitate agents that a language
//! model drives.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use unsup_kernel::Rng;

use crate::random;

/// Think time in ticks, for the commands that run in one process.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TickThink {
    /// Always the same number of ticks.
    Constant {
        /// The number of ticks.
        ticks: u32,
    },
    /// A uniform number of ticks from `min` to `max`, both included.
    Uniform {
        /// Least number of ticks.
        min: u32,
        /// Greatest number of ticks.
        max: u32,
    },
    /// An exponential number of ticks with the given mean, at most `max`.
    Exponential {
        /// Mean number of ticks before the cut at `max`.
        mean: f64,
        /// Greatest number of ticks.
        max: u32,
    },
}

impl TickThink {
    /// Checks the numbers.
    pub fn validate(&self) -> Result<(), String> {
        match *self {
            TickThink::Constant { .. } => Ok(()),
            TickThink::Uniform { min, max } if min > max => {
                Err(format!("think: min ({min}) is greater than max ({max})"))
            }
            TickThink::Uniform { .. } => Ok(()),
            TickThink::Exponential { mean, .. } if !(mean.is_finite() && mean >= 0.0) => {
                Err(format!("think: mean ({mean}) must be a number that is not negative"))
            }
            TickThink::Exponential { .. } => Ok(()),
        }
    }

    /// Returns one think time in ticks.
    pub fn sample(&self, rng: &mut Rng) -> u32 {
        match *self {
            TickThink::Constant { ticks } => ticks,
            TickThink::Uniform { min, max } => {
                let ticks = random::int_between(rng, i64::from(min), i64::from(max));
                u32::try_from(ticks).unwrap_or(min)
            }
            TickThink::Exponential { mean, max } => {
                let ticks = random::exponential(rng, mean).round();
                // The value is finite and not negative; the cast saturates.
                (ticks as u32).min(max)
            }
        }
    }
}

/// The distributions of think time in seconds that the swarm knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkKind {
    /// Always the mean.
    Constant,
    /// The exponential distribution: many short times, few long ones.
    Exponential,
    /// The log-normal distribution: like the answer times of a language
    /// model, with a long tail.
    Lognormal,
}

impl std::str::FromStr for ThinkKind {
    type Err = String;

    fn from_str(text: &str) -> Result<ThinkKind, String> {
        match text.to_ascii_lowercase().replace('-', "").as_str() {
            "constant" => Ok(ThinkKind::Constant),
            "exponential" => Ok(ThinkKind::Exponential),
            "lognormal" => Ok(ThinkKind::Lognormal),
            _ => Err(format!(
                "unknown think time distribution `{text}`; use constant, exponential or lognormal"
            )),
        }
    }
}

impl fmt::Display for ThinkKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ThinkKind::Constant => "constant",
            ThinkKind::Exponential => "exponential",
            ThinkKind::Lognormal => "lognormal",
        })
    }
}

/// Think time in seconds of real time, for the swarm.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThinkTime {
    /// The distribution.
    pub kind: ThinkKind,
    /// Mean in seconds.
    pub mean_secs: f64,
    /// Shape of the log-normal distribution: the standard deviation of the
    /// logarithm. The other distributions ignore it.
    pub sigma: f64,
}

impl ThinkTime {
    /// Makes a think time and checks the numbers.
    pub fn new(kind: ThinkKind, mean_secs: f64, sigma: f64) -> Result<ThinkTime, String> {
        if !(mean_secs.is_finite() && mean_secs >= 0.0) {
            return Err(format!("the mean think time ({mean_secs}) must not be negative"));
        }
        if !(sigma.is_finite() && sigma >= 0.0) {
            return Err(format!("the think time sigma ({sigma}) must not be negative"));
        }
        Ok(ThinkTime { kind, mean_secs, sigma })
    }

    /// Returns one think time in seconds.
    pub fn sample_secs(&self, rng: &mut Rng) -> f64 {
        if self.mean_secs <= 0.0 {
            return 0.0;
        }
        match self.kind {
            ThinkKind::Constant => self.mean_secs,
            ThinkKind::Exponential => random::exponential(rng, self.mean_secs),
            ThinkKind::Lognormal => {
                // The mean of exp(mu + sigma * Z) is exp(mu + sigma^2 / 2).
                let mu = self.mean_secs.ln() - self.sigma * self.sigma / 2.0;
                (mu + self.sigma * random::standard_normal(rng)).exp()
            }
        }
    }

    /// Returns one think time.
    pub fn sample(&self, rng: &mut Rng) -> Duration {
        let secs = self.sample_secs(rng);
        if secs.is_finite() && secs > 0.0 {
            Duration::try_from_secs_f64(secs).unwrap_or(Duration::MAX)
        } else {
            Duration::ZERO
        }
    }
}

impl fmt::Display for ThinkTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ThinkKind::Lognormal => {
                write!(f, "lognormal, mean {} s, sigma {}", self.mean_secs, self.sigma)
            }
            kind => write!(f, "{kind}, mean {} s", self.mean_secs),
        }
    }
}
