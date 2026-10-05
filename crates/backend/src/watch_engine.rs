//! Pure show-timeline math (port of HQTube's `events/engine.ts`). No IO, no
//! clocks — everything takes `now` so a restart can recompute exactly where
//! a show is, from the schedule or from the persisted cursor.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EngineItem {
    pub duration_ms: Option<i64>,
    pub intermission_after_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorPhase {
    Preshow,
    Playing,
    Paused,
    Intermission,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cursor {
    pub item_index: i64,
    pub phase: CursorPhase,
    pub base_pos_ms: i64,
    /// Wall-clock anchor for `base_pos_ms`; during intermission: its end.
    pub base_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    Playing { item_index: usize, position_ms: i64 },
    Paused { item_index: usize, position_ms: i64 },
    Intermission { item_index: usize, ends_at_ms: i64 },
    Ended,
}

/// Rolls forward through the timeline: item `start_index` begins at wall
/// time `t0`; durations and intermissions elapse back-to-back until `now`
/// lands inside an item or a break. An unknown duration never auto-elapses.
pub fn roll_forward(items: &[EngineItem], start_index: usize, t0: i64, now: i64) -> Recovery {
    let mut t = t0;
    for (i, item) in items.iter().enumerate().skip(start_index) {
        let Some(duration) = item.duration_ms else {
            return Recovery::Playing { item_index: i, position_ms: (now - t).max(0) };
        };
        let item_end = t + duration;
        if now < item_end {
            return Recovery::Playing { item_index: i, position_ms: (now - t).max(0) };
        }
        let is_last = i == items.len() - 1;
        let break_end = item_end + if is_last { 0 } else { item.intermission_after_ms };
        if now < break_end {
            return Recovery::Intermission { item_index: i, ends_at_ms: break_end };
        }
        t = break_end;
    }
    Recovery::Ended
}

/// Recovery for a `scheduled` event whose start time already passed.
pub fn recover_from_schedule(starts_at_ms: i64, items: &[EngineItem], now: i64) -> Recovery {
    if items.is_empty() {
        return Recovery::Ended;
    }
    roll_forward(items, 0, starts_at_ms, now)
}

/// Recovery for a `live` event from its persisted cursor.
pub fn resume_from_cursor(cursor: Cursor, items: &[EngineItem], now: i64) -> Recovery {
    if cursor.item_index < 0 || cursor.item_index as usize >= items.len() {
        return Recovery::Ended;
    }
    let index = cursor.item_index as usize;
    match cursor.phase {
        // A pause freezes the timeline — resume exactly there, however long ago.
        CursorPhase::Paused => Recovery::Paused { item_index: index, position_ms: cursor.base_pos_ms.max(0) },
        // Virtual item start = anchor minus the position it anchored.
        CursorPhase::Playing => roll_forward(items, index, cursor.base_at_ms - cursor.base_pos_ms, now),
        CursorPhase::Intermission => {
            let ends_at_ms = cursor.base_at_ms;
            if now < ends_at_ms {
                return Recovery::Intermission { item_index: index, ends_at_ms };
            }
            if index + 1 >= items.len() {
                return Recovery::Ended;
            }
            roll_forward(items, index + 1, ends_at_ms, now)
        }
        // Preshow is never persisted for a live event; treat as not started.
        CursorPhase::Preshow => Recovery::Ended,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    // Two 10-min movies with a 5-min break, then a 20-min movie. Start at t=0.
    fn items() -> Vec<EngineItem> {
        vec![
            EngineItem { duration_ms: Some(10 * MIN), intermission_after_ms: 5 * MIN },
            EngineItem { duration_ms: Some(10 * MIN), intermission_after_ms: 3 * MIN },
            // Break after the last item is ignored.
            EngineItem { duration_ms: Some(20 * MIN), intermission_after_ms: 7 * MIN },
        ]
    }

    #[test]
    fn lands_inside_first_item() {
        assert_eq!(
            recover_from_schedule(0, &items(), 4 * MIN),
            Recovery::Playing { item_index: 0, position_ms: 4 * MIN }
        );
    }

    #[test]
    fn lands_in_first_intermission() {
        assert_eq!(
            recover_from_schedule(0, &items(), 12 * MIN),
            Recovery::Intermission { item_index: 0, ends_at_ms: 15 * MIN }
        );
    }

    #[test]
    fn lands_inside_second_item() {
        assert_eq!(
            recover_from_schedule(0, &items(), 15 * MIN),
            Recovery::Playing { item_index: 1, position_ms: 0 }
        );
        assert_eq!(
            recover_from_schedule(0, &items(), 24 * MIN),
            Recovery::Playing { item_index: 1, position_ms: 9 * MIN }
        );
    }

    #[test]
    fn lands_inside_last_item_and_ends_after_it() {
        assert_eq!(
            recover_from_schedule(0, &items(), 30 * MIN),
            Recovery::Playing { item_index: 2, position_ms: 2 * MIN }
        );
        // 10+5+10+3+20 = 48 min total.
        assert_eq!(recover_from_schedule(0, &items(), 48 * MIN), Recovery::Ended);
    }

    #[test]
    fn exact_at_boundaries() {
        assert_eq!(
            recover_from_schedule(0, &items(), 10 * MIN),
            Recovery::Intermission { item_index: 0, ends_at_ms: 15 * MIN }
        );
    }

    #[test]
    fn unknown_duration_never_elapses() {
        let list = vec![
            EngineItem { duration_ms: None, intermission_after_ms: 5 * MIN },
            EngineItem { duration_ms: Some(10 * MIN), intermission_after_ms: 0 },
        ];
        assert_eq!(
            recover_from_schedule(0, &list, 500 * MIN),
            Recovery::Playing { item_index: 0, position_ms: 500 * MIN }
        );
    }

    #[test]
    fn never_rolls_past_unknown_duration() {
        let list = vec![
            EngineItem { duration_ms: Some(10 * MIN), intermission_after_ms: 0 },
            EngineItem { duration_ms: None, intermission_after_ms: 5 * MIN },
            EngineItem { duration_ms: Some(10 * MIN), intermission_after_ms: 0 },
        ];
        assert_eq!(
            recover_from_schedule(0, &list, 500 * MIN),
            Recovery::Playing { item_index: 1, position_ms: 490 * MIN }
        );
    }

    #[test]
    fn respects_start_offset() {
        assert_eq!(
            roll_forward(&items(), 1, 100 * MIN, 104 * MIN),
            Recovery::Playing { item_index: 1, position_ms: 4 * MIN }
        );
    }

    #[test]
    fn empty_lineup_ends() {
        assert_eq!(recover_from_schedule(0, &[], 0), Recovery::Ended);
    }

    #[test]
    fn resume_paused_freezes() {
        let c = Cursor { item_index: 1, phase: CursorPhase::Paused, base_pos_ms: 3 * MIN, base_at_ms: 0 };
        assert_eq!(
            resume_from_cursor(c, &items(), 999 * MIN),
            Recovery::Paused { item_index: 1, position_ms: 3 * MIN }
        );
    }

    #[test]
    fn resume_playing_rolls_forward() {
        // Item 1 was at 2 min when anchored at t=100 min.
        let c = Cursor { item_index: 1, phase: CursorPhase::Playing, base_pos_ms: 2 * MIN, base_at_ms: 100 * MIN };
        assert_eq!(
            resume_from_cursor(c, &items(), 105 * MIN),
            Recovery::Playing { item_index: 1, position_ms: 7 * MIN }
        );
        // Item 1 ends at 108, break until 111, then item 2.
        assert_eq!(
            resume_from_cursor(c, &items(), 109 * MIN),
            Recovery::Intermission { item_index: 1, ends_at_ms: 111 * MIN }
        );
        assert_eq!(
            resume_from_cursor(c, &items(), 112 * MIN),
            Recovery::Playing { item_index: 2, position_ms: MIN }
        );
    }

    #[test]
    fn resume_intermission() {
        let c = Cursor { item_index: 0, phase: CursorPhase::Intermission, base_pos_ms: 0, base_at_ms: 50 * MIN };
        assert_eq!(
            resume_from_cursor(c, &items(), 49 * MIN),
            Recovery::Intermission { item_index: 0, ends_at_ms: 50 * MIN }
        );
        assert_eq!(
            resume_from_cursor(c, &items(), 52 * MIN),
            Recovery::Playing { item_index: 1, position_ms: 2 * MIN }
        );
        let last = Cursor { item_index: 2, phase: CursorPhase::Intermission, base_pos_ms: 0, base_at_ms: 0 };
        assert_eq!(resume_from_cursor(last, &items(), MIN), Recovery::Ended);
    }

    #[test]
    fn resume_out_of_range_or_preshow_ends() {
        let c = Cursor { item_index: 7, phase: CursorPhase::Playing, base_pos_ms: 0, base_at_ms: 0 };
        assert_eq!(resume_from_cursor(c, &items(), 0), Recovery::Ended);
        let p = Cursor { item_index: 0, phase: CursorPhase::Preshow, base_pos_ms: 0, base_at_ms: 0 };
        assert_eq!(resume_from_cursor(p, &items(), 0), Recovery::Ended);
    }
}
