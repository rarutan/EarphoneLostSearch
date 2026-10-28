/// Resolved Frame1 origin window. `end_tick` is exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Frame1CandidateRange {
    pub(super) start_tick: i64,
    pub(super) end_tick: i64,
}

impl Frame1CandidateRange {
    pub(super) fn new(start_tick: i64, end_tick: i64) -> Self {
        Self {
            start_tick: start_tick.max(0),
            end_tick: end_tick.max(0),
        }
    }

    pub(super) fn is_empty(self) -> bool {
        self.start_tick >= self.end_tick
    }

    pub(super) fn contains(self, tick: i64) -> bool {
        (self.start_tick..self.end_tick).contains(&tick)
    }
}
