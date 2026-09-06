/// Budget for one core-thread drain: view build + comparison + projection.
///
/// This is measurement-derived rather than an arbitrary fraction of the 75 ms tick. At production
/// scale (9,475 jobs, 37,931 links, and 9,475 summary-cache entries), the dev profile measured
/// 24 ms for the normal case and 21 ms for search on the development machine. A second machine ran
/// the same test about 1.5x slower, projecting the slowest case to roughly 36 ms. A 40 ms budget
/// gives modest cross-machine headroom while still catching a regression of roughly 1.7x, and it
/// keeps one core-thread drain comfortably within a single 75 ms tick. The optimized profile was
/// roughly 4x faster than dev in the same work. The original 20 ms value predated measurement;
/// phase 2 must rerun this test after the activity feed is added to the payload and revisit the
/// budget from that evidence.
pub const HOST_DRAIN_BUDGET_MS: u64 = 40;
