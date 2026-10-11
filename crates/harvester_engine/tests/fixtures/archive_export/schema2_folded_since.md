===== DOC START =====
url: https://example.com/a
title: Kept event
tokens: 2
fetched_utc: 2026-10-01T00:00:00Z
filename: a.md
content: full
export_schema: 2
doc: 1
signal_key: event
signal_score: 80
themes: ["AI"]
folded: [{"reason":"same_event","url":"https://example.com/b","title":"Same slug","signal_key":"event"},{"reason":"same_event","url":"https://example.com/c","title":"Other slug","signal_key":"other-slug"},{"reason":"same_event","url":"https://example.com/missing"}]

Body a.
===== DOC END =====

===== DOC START =====
url: https://example.com/d
title: Kept copy
tokens: 2
fetched_utc: 2026-10-01T00:00:00Z
filename: d.md
content: full
export_schema: 2
doc: 2
folded: [{"reason":"identical_content","url":"https://example.com/e","title":"Same text elsewhere"}]

Body d.
===== DOC END =====

===== DOC START =====
url: https://example.com/f
title: Solo
tokens: 2
fetched_utc: 2026-10-01T00:00:00Z
filename: f.md
content: full
export_schema: 2
doc: 3
folded: []

Body f.
===== DOC END =====

===== ARCHIVE INDEX =====
export_schema: 2
doc_count: 3
fetched_from: 2026-10-01T00:00:00Z
fetched_to: 2026-10-01T00:00:00Z
window_count: 6
unexported_by_priority: {"5":1,"4":1,"3":0,"2":0,"1":0,"unavailable":1}
unexported_folded: {"same_event":2,"identical_content":1}
doc | line | fetched_utc | priority | signal_key | title
1 | 1 | 2026-10-01T00:00:00Z | - | event | Kept event
2 | 18 | 2026-10-01T00:00:00Z | - | - | Kept copy
3 | 32 | 2026-10-01T00:00:00Z | - | - | Solo
===== INDEX END =====
