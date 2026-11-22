//! The timer behind study sessions and study rooms.
//!
//! A timer is a position on a timeline. A stopwatch's timeline is one endless
//! work phase; a pomodoro's repeats `rounds` work phases separated by breaks,
//! the last break of each cycle being the long one. While the timer runs, the
//! position at time `t` is `offset + (t - anchor)`; pausing freezes it, and
//! skipping a phase jumps it to the start of the next one. Everything else —
//! the phase to display, when it ends, which stretches of real time count as
//! study — is computed from that, so the server never has to tick.
use chrono::Duration;
use tokuly_core::prelude::*;
use tokuly_proto::school::{PomodoroConfig, StudyMode, TimerPhase, TimerSnapshot, TimerState};

/// Lengths of a pomodoro cycle in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cycle {
    pub work_ms: i64,
    pub break_ms: i64,
    pub long_break_ms: i64,
    /// Work phases per cycle.
    pub rounds: i64,
}

/// The phase a timeline is in at some position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PhaseAt {
    pub phase: TimerPhase,
    /// 1-based number of the work phase in progress, or coming up after a break.
    pub round: i64,
    pub start_ms: i64,
    /// `i64::MAX` for a stopwatch.
    pub end_ms: i64,
}

impl Cycle {
    /// Builds a cycle from request lengths in minutes; zero means "default".
    pub fn from_config(config: &PomodoroConfig, defaults: Cycle) -> AppResult<Self> {
        let pick = |minutes: i32, default_ms: i64, max: i32, what: &str| -> AppResult<i64> {
            if minutes == 0 {
                return Ok(default_ms);
            }
            ensure!((1..=max).contains(&minutes), invalid, "POMODORO_INVALID", "{what} must be 1-{max} minutes");
            Ok(minutes as i64 * 60_000)
        };
        let rounds = match config.rounds {
            0 => defaults.rounds,
            rounds => {
                ensure!((1..=12).contains(&rounds), invalid, "POMODORO_INVALID", "rounds must be 1-12");
                rounds as i64
            }
        };
        Ok(Self {
            work_ms: pick(config.work_minutes, defaults.work_ms, 180, "work_minutes")?,
            break_ms: pick(config.break_minutes, defaults.break_ms, 60, "break_minutes")?,
            long_break_ms: pick(config.long_break_minutes, defaults.long_break_ms, 120, "long_break_minutes")?,
            rounds,
        })
    }

    /// From the `*_seconds` / `rounds` columns.
    pub fn from_seconds(work: i32, short_break: i32, long_break: i32, rounds: i32) -> Self {
        Self {
            work_ms: (work as i64).max(1) * 1000,
            break_ms: (short_break as i64).max(1) * 1000,
            long_break_ms: (long_break as i64).max(1) * 1000,
            rounds: (rounds as i64).max(1),
        }
    }

    pub fn to_config(self) -> PomodoroConfig {
        PomodoroConfig {
            work_minutes: (self.work_ms / 60_000) as i32,
            break_minutes: (self.break_ms / 60_000) as i32,
            long_break_minutes: (self.long_break_ms / 60_000) as i32,
            rounds: self.rounds as i32,
            ..Default::default()
        }
    }

    fn length_ms(&self) -> i64 {
        self.rounds * self.work_ms + (self.rounds - 1) * self.break_ms + self.long_break_ms
    }

    fn phase_at(&self, position_ms: i64) -> PhaseAt {
        let position_ms = position_ms.max(0);
        let cycle = position_ms / self.length_ms();
        let cycle_start = cycle * self.length_ms();
        let within = position_ms - cycle_start;
        let step = self.work_ms + self.break_ms;
        // The last round of a cycle is followed by the long break instead of a short one.
        let round = (within / step).min(self.rounds - 1);
        let work_start = cycle_start + round * step;
        let work_end = work_start + self.work_ms;
        let first_round = cycle * self.rounds + 1;
        if position_ms < work_end {
            return PhaseAt {
                phase: TimerPhase::TIMER_PHASE_WORK,
                round: first_round + round,
                start_ms: work_start,
                end_ms: work_end,
            };
        }
        let (phase, length) = if round == self.rounds - 1 {
            (TimerPhase::TIMER_PHASE_LONG_BREAK, self.long_break_ms)
        } else {
            (TimerPhase::TIMER_PHASE_BREAK, self.break_ms)
        };
        PhaseAt { phase, round: first_round + round + 1, start_ms: work_end, end_ms: work_end + length }
    }
}

/// What a timer counts through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Plan {
    /// A stopwatch: everything is work.
    Open,
    Pomodoro(Cycle),
}

impl Plan {
    /// From the `mode` and length columns of a session or room.
    pub fn from_columns(mode: i16, work: i32, short_break: i32, long_break: i32, rounds: i32) -> Self {
        if mode == StudyMode::STUDY_MODE_POMODORO as i16 {
            Self::Pomodoro(Cycle::from_seconds(work, short_break, long_break, rounds))
        } else {
            Self::Open
        }
    }

    pub fn mode(&self) -> StudyMode {
        match self {
            Self::Open => StudyMode::STUDY_MODE_STOPWATCH,
            Self::Pomodoro(_) => StudyMode::STUDY_MODE_POMODORO,
        }
    }

    /// `(work, break, long break)` in seconds and the rounds, for the columns.
    pub fn columns(&self) -> (i32, i32, i32, i32) {
        match self {
            Self::Open => (0, 0, 0, 0),
            Self::Pomodoro(c) => (
                (c.work_ms / 1000) as i32,
                (c.break_ms / 1000) as i32,
                (c.long_break_ms / 1000) as i32,
                c.rounds as i32,
            ),
        }
    }

    pub fn phase_at(&self, position_ms: i64) -> PhaseAt {
        match self {
            Self::Open => PhaseAt { phase: TimerPhase::TIMER_PHASE_WORK, round: 1, start_ms: 0, end_ms: i64::MAX },
            Self::Pomodoro(cycle) => cycle.phase_at(position_ms),
        }
    }

    /// The work stretches between two positions.
    pub fn work_spans(&self, from_ms: i64, to_ms: i64) -> Vec<(i64, i64)> {
        let mut spans = Vec::new();
        let mut cursor = from_ms.max(0);
        while cursor < to_ms {
            let phase = self.phase_at(cursor);
            let until = phase.end_ms.min(to_ms);
            if phase.phase == TimerPhase::TIMER_PHASE_WORK {
                spans.push((cursor, until));
            }
            cursor = until;
        }
        spans
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Run {
    /// Never started (study rooms wait for their host).
    Idle,
    Running,
    Paused,
}

impl Run {
    pub fn from_column(state: i16) -> Self {
        match state {
            2 => Self::Running,
            3 => Self::Paused,
            _ => Self::Idle,
        }
    }

    pub fn column(self) -> i16 {
        self.proto() as i16
    }

    pub fn proto(self) -> TimerState {
        match self {
            Self::Idle => TimerState::TIMER_STATE_IDLE,
            Self::Running => TimerState::TIMER_STATE_RUNNING,
            Self::Paused => TimerState::TIMER_STATE_PAUSED,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Timer {
    pub plan: Plan,
    pub run: Run,
    /// While running: the moment the position was `offset_ms`.
    pub anchor: Option<Time>,
    pub offset_ms: i64,
}

impl Timer {
    /// A timer that has not started.
    pub fn idle(plan: Plan) -> Self {
        Self { plan, run: Run::Idle, anchor: None, offset_ms: 0 }
    }

    pub fn is_running(&self) -> bool {
        self.run == Run::Running
    }

    /// Where on the timeline the timer is at `at`.
    pub fn position_ms(&self, at: Time) -> i64 {
        match (self.run, self.anchor) {
            (Run::Running, Some(anchor)) => self.offset_ms + (at - anchor).num_milliseconds().max(0),
            _ => self.offset_ms,
        }
    }

    pub fn phase_at(&self, at: Time) -> PhaseAt {
        self.plan.phase_at(self.position_ms(at))
    }

    /// When the phase in progress at `at` ends by itself: running pomodoros only.
    pub fn phase_ends_at(&self, at: Time) -> Option<Time> {
        if !self.is_running() || self.plan == Plan::Open {
            return None;
        }
        let position = self.position_ms(at);
        Some(at + Duration::milliseconds(self.plan.phase_at(position).end_ms - position))
    }

    /// The stretches of real time between `from` and `to` during which the
    /// timer was in a work phase. Nothing while it is not running.
    pub fn work_intervals(&self, from: Time, to: Time) -> Vec<(Time, Time)> {
        let (Run::Running, Some(anchor)) = (self.run, self.anchor) else { return Vec::new() };
        let from = from.max(anchor);
        if to <= from {
            return Vec::new();
        }
        let to_time = |position_ms: i64| anchor + Duration::milliseconds(position_ms - self.offset_ms);
        self.plan
            .work_spans(self.position_ms(from), self.position_ms(to))
            .into_iter()
            .map(|(start, end)| (to_time(start), to_time(end)))
            .collect()
    }

    /// Starts an idle timer or resumes a paused one.
    pub fn run_from(&mut self, at: Time) {
        if self.run != Run::Running {
            self.run = Run::Running;
            self.anchor = Some(at);
        }
    }

    pub fn pause(&mut self, at: Time) {
        if self.run == Run::Running {
            self.offset_ms = self.position_ms(at);
            self.run = Run::Paused;
            self.anchor = None;
        }
    }

    /// Jumps to the start of the next phase. Stopwatches have none to skip.
    pub fn skip(&mut self, at: Time) {
        if self.plan == Plan::Open {
            return;
        }
        self.offset_ms = self.plan.phase_at(self.position_ms(at)).end_ms;
        if self.run == Run::Running {
            self.anchor = Some(at);
        }
    }

    /// Back to the beginning, not started.
    pub fn reset(&mut self) {
        *self = Self::idle(self.plan);
    }

    /// The same state with other lengths, back at the first work phase.
    pub fn replan(&mut self, plan: Plan, at: Time) {
        self.plan = plan;
        self.offset_ms = 0;
        if self.run == Run::Running {
            self.anchor = Some(at);
        }
    }

    /// The timer as clients render it, at `at`.
    pub fn snapshot(&self, at: Time) -> TimerSnapshot {
        let position = self.position_ms(at);
        let phase = self.plan.phase_at(position);
        let mut snapshot = TimerSnapshot {
            mode: self.plan.mode().into(),
            state: self.run.proto().into(),
            phase: phase.phase.into(),
            round: phase.round as i32,
            elapsed_seconds: (position / 1000) as i32,
            server_time: ts(at),
            ..Default::default()
        };
        if let Plan::Pomodoro(cycle) = self.plan {
            snapshot.pomodoro = cycle.to_config().into();
            let remaining = phase.end_ms - position;
            // Round up: a phase with 0.4 s left still shows "1".
            snapshot.phase_remaining_seconds = ((remaining + 999) / 1000) as i32;
            if self.is_running() {
                snapshot.phase_started_at = ts(at - Duration::milliseconds(position - phase.start_ms));
                snapshot.phase_ends_at = ts(at + Duration::milliseconds(remaining));
            }
        }
        snapshot
    }
}

/// Total length of some intervals in milliseconds.
pub(crate) fn total_ms(intervals: &[(Time, Time)]) -> i64 {
    intervals.iter().map(|(start, end)| (*end - *start).num_milliseconds()).sum()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    const MIN: i64 = 60_000;

    /// The classic pomodoro: 25 minutes of work, 5 of break, 15 after four rounds.
    fn classic() -> Cycle {
        Cycle { work_ms: 25 * MIN, break_ms: 5 * MIN, long_break_ms: 15 * MIN, rounds: 4 }
    }

    fn started(plan: Plan, at: Time) -> Timer {
        let mut timer = Timer::idle(plan);
        timer.run_from(at);
        timer
    }

    fn t(minutes: i64) -> Time {
        chrono::Utc.with_ymd_and_hms(2026, 4, 8, 9, 0, 0).unwrap() + Duration::minutes(minutes)
    }

    #[test]
    fn a_classic_cycle_is_four_rounds_with_a_long_break() {
        let cycle = classic();
        assert_eq!(cycle.length_ms(), (4 * 25 + 3 * 5 + 15) * MIN);
        let phases: Vec<(TimerPhase, i64, i64, i64)> = [0, 24, 25, 29, 30, 55, 60, 85, 90, 115, 129, 130, 155]
            .into_iter()
            .map(|minute| {
                let p = cycle.phase_at(minute * MIN);
                (p.phase, p.round, p.start_ms / MIN, p.end_ms / MIN)
            })
            .collect();
        use TimerPhase::*;
        assert_eq!(
            phases,
            vec![
                (TIMER_PHASE_WORK, 1, 0, 25),
                (TIMER_PHASE_WORK, 1, 0, 25),
                (TIMER_PHASE_BREAK, 2, 25, 30),
                (TIMER_PHASE_BREAK, 2, 25, 30),
                (TIMER_PHASE_WORK, 2, 30, 55),
                (TIMER_PHASE_BREAK, 3, 55, 60),
                (TIMER_PHASE_WORK, 3, 60, 85),
                (TIMER_PHASE_BREAK, 4, 85, 90),
                (TIMER_PHASE_WORK, 4, 90, 115),
                (TIMER_PHASE_LONG_BREAK, 5, 115, 130),
                (TIMER_PHASE_LONG_BREAK, 5, 115, 130),
                // The second cycle starts with round 5.
                (TIMER_PHASE_WORK, 5, 130, 155),
                (TIMER_PHASE_BREAK, 6, 155, 160),
            ]
        );
    }

    #[test]
    fn a_single_round_cycle_alternates_work_and_long_break() {
        let cycle = Cycle { work_ms: 50 * MIN, break_ms: 10 * MIN, long_break_ms: 10 * MIN, rounds: 1 };
        assert_eq!(cycle.phase_at(49 * MIN).phase, TimerPhase::TIMER_PHASE_WORK);
        assert_eq!(cycle.phase_at(50 * MIN).phase, TimerPhase::TIMER_PHASE_LONG_BREAK);
        let second = cycle.phase_at(60 * MIN);
        assert_eq!((second.phase, second.round), (TimerPhase::TIMER_PHASE_WORK, 2));
    }

    #[test]
    fn only_work_counts() {
        let plan = Plan::Pomodoro(classic());
        // 10 minutes into the first round until 5 minutes into the third.
        let spans = plan.work_spans(10 * MIN, 65 * MIN);
        assert_eq!(spans, vec![(10 * MIN, 25 * MIN), (30 * MIN, 55 * MIN), (60 * MIN, 65 * MIN)]);
        assert_eq!(plan.work_spans(26 * MIN, 29 * MIN), vec![], "a stretch inside a break counts for nothing");
        assert_eq!(Plan::Open.work_spans(3 * MIN, 500 * MIN), vec![(3 * MIN, 500 * MIN)]);
        assert_eq!(Plan::Open.work_spans(5 * MIN, 5 * MIN), vec![]);
    }

    #[test]
    fn a_running_timer_maps_work_to_real_time() {
        let timer = started(Plan::Pomodoro(classic()), t(0));
        assert_eq!(timer.work_intervals(t(0), t(40)), vec![(t(0), t(25)), (t(30), t(40))]);
        assert_eq!(total_ms(&timer.work_intervals(t(20), t(32))), 7 * MIN);
        // Time before the timer started does not count.
        assert_eq!(timer.work_intervals(t(-10), t(5)), vec![(t(0), t(5))]);
        assert_eq!(timer.work_intervals(t(5), t(5)), vec![]);
    }

    #[test]
    fn pausing_freezes_the_position_and_resuming_shifts_the_phases() {
        let mut timer = started(Plan::Pomodoro(classic()), t(0));
        timer.pause(t(10));
        assert_eq!(timer.run, Run::Paused);
        assert_eq!(timer.position_ms(t(60)), 10 * MIN, "no progress while paused");
        assert_eq!(timer.work_intervals(t(10), t(60)), vec![]);
        assert_eq!(timer.phase_ends_at(t(30)), None);

        timer.run_from(t(60));
        assert_eq!(timer.position_ms(t(70)), 20 * MIN);
        // 15 minutes of the first round were left: it now ends at minute 75.
        assert_eq!(timer.phase_ends_at(t(61)), Some(t(75)));
        assert_eq!(timer.work_intervals(t(60), t(82)), vec![(t(60), t(75)), (t(80), t(82))]);
        // Resuming twice changes nothing.
        let before = timer;
        timer.run_from(t(65));
        assert_eq!(timer, before);
    }

    #[test]
    fn skipping_jumps_to_the_next_phase() {
        let mut timer = started(Plan::Pomodoro(classic()), t(0));
        timer.skip(t(10)); // skip the rest of the first round: the break starts now
        let phase = timer.phase_at(t(10));
        assert_eq!((phase.phase, phase.round), (TimerPhase::TIMER_PHASE_BREAK, 2));
        assert_eq!(timer.phase_ends_at(t(10)), Some(t(15)));
        timer.skip(t(11)); // skip the break
        assert_eq!(timer.phase_at(t(11)).phase, TimerPhase::TIMER_PHASE_WORK);
        assert_eq!(
            timer.work_intervals(t(0), t(20)),
            vec![(t(11), t(20))],
            "work intervals are those since the last jump"
        );

        // Skipping while paused stays paused, at the start of the next phase.
        let mut paused = started(Plan::Pomodoro(classic()), t(0));
        paused.pause(t(3));
        paused.skip(t(50));
        assert_eq!((paused.run, paused.offset_ms), (Run::Paused, 25 * MIN));

        // A stopwatch has nothing to skip.
        let mut open = started(Plan::Open, t(0));
        open.skip(t(5));
        assert_eq!(open.position_ms(t(5)), 5 * MIN);
    }

    #[test]
    fn snapshots_describe_the_phase_with_server_timestamps() {
        let mut timer = started(Plan::Pomodoro(classic()), t(0));
        let running = timer.snapshot(t(27));
        assert_eq!(running.state, TimerState::TIMER_STATE_RUNNING);
        assert_eq!(running.phase, TimerPhase::TIMER_PHASE_BREAK);
        assert_eq!(running.round, 2);
        assert_eq!(from_ts(running.phase_started_at.as_option().unwrap()), t(25));
        assert_eq!(from_ts(running.phase_ends_at.as_option().unwrap()), t(30));
        assert_eq!(running.phase_remaining_seconds, 180);
        assert_eq!(running.elapsed_seconds, 27 * 60);
        assert_eq!(from_ts(running.server_time.as_option().unwrap()), t(27));
        assert_eq!(running.pomodoro.as_option().unwrap().work_minutes, 25);

        timer.pause(t(27));
        let paused = timer.snapshot(t(40));
        assert_eq!(paused.state, TimerState::TIMER_STATE_PAUSED);
        assert!(paused.phase_ends_at.as_option().is_none(), "a paused phase has no end time");
        assert_eq!(paused.phase_remaining_seconds, 180);

        let idle = Timer::idle(Plan::Pomodoro(classic())).snapshot(t(0));
        assert_eq!(idle.state, TimerState::TIMER_STATE_IDLE);
        assert_eq!(
            (idle.phase, idle.round, idle.phase_remaining_seconds),
            (TimerPhase::TIMER_PHASE_WORK.into(), 1, 1500)
        );

        let open = started(Plan::Open, t(0)).snapshot(t(90));
        assert_eq!(open.mode, StudyMode::STUDY_MODE_STOPWATCH);
        assert_eq!(
            (open.phase, open.elapsed_seconds, open.phase_remaining_seconds),
            (TimerPhase::TIMER_PHASE_WORK.into(), 5400, 0)
        );
        assert!(open.pomodoro.as_option().is_none());
    }

    #[test]
    fn remaining_seconds_round_up() {
        let timer = started(Plan::Pomodoro(classic()), t(0));
        let almost = t(25) - Duration::milliseconds(400);
        assert_eq!(timer.snapshot(almost).phase_remaining_seconds, 1);
    }

    #[test]
    fn configs_fall_back_to_defaults_and_are_range_checked() {
        let defaults = Cycle { work_ms: 50 * MIN, break_ms: 10 * MIN, long_break_ms: 30 * MIN, rounds: 2 };
        assert_eq!(Cycle::from_config(&PomodoroConfig::default(), defaults).unwrap(), defaults);
        let custom =
            Cycle::from_config(&PomodoroConfig { work_minutes: 45, rounds: 3, ..Default::default() }, defaults)
                .unwrap();
        assert_eq!(custom, Cycle { work_ms: 45 * MIN, rounds: 3, ..defaults });
        for bad in [
            PomodoroConfig { work_minutes: 181, ..Default::default() },
            PomodoroConfig { work_minutes: -1, ..Default::default() },
            PomodoroConfig { break_minutes: 61, ..Default::default() },
            PomodoroConfig { long_break_minutes: 121, ..Default::default() },
            PomodoroConfig { rounds: 13, ..Default::default() },
        ] {
            assert_eq!(Cycle::from_config(&bad, defaults).unwrap_err().reason, "POMODORO_INVALID");
        }
        assert_eq!(custom.to_config().work_minutes, 45);
        let plan = Plan::Pomodoro(custom);
        let (work, short_break, long_break, rounds) = plan.columns();
        assert_eq!(Plan::from_columns(2, work, short_break, long_break, rounds), plan);
        assert_eq!(Plan::from_columns(1, 0, 0, 0, 0), Plan::Open);
    }
}
