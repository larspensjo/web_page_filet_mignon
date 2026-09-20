===== DOC START =====
url: https://example.com/untriaged
title: untriaged
tokens: 2
fetched_utc: 2026-09-02T00:00:00Z
filename: 1-untriaged.md
content: full
export_schema: 2
doc: 1

Body untriaged.
===== DOC END =====

===== DOC START =====
url: https://example.com/scored
title: scored
tokens: 2
fetched_utc: 2026-09-03T00:00:00Z
filename: 2-scored.md
content: full
export_schema: 2
doc: 2
signal_key: scored-event
signal_score: 77
themes: ["AI","chips"]

Body scored.
===== DOC END =====

===== ARCHIVE INDEX =====
export_schema: 2
doc_count: 2
fetched_from: 2026-09-02T00:00:00Z
fetched_to: 2026-09-03T00:00:00Z
window_count: 14
unexported_by_priority: {"5":1,"4":2,"3":3,"2":1,"1":1,"unavailable":4}
doc | line | fetched_utc | priority | signal_key | title
1 | 1 | 2026-09-02T00:00:00Z | - | - | untriaged
2 | 14 | 2026-09-03T00:00:00Z | - | scored-event | scored
===== INDEX END =====
