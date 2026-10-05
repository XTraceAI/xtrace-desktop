# Human input-effort estimate and agent ratio

Estimated Human hours are the sum of effective counted input characters divided by configured WPM × 5 × 60. At 40 WPM, 12,000 characters equal one hour; at 80 WPM they equal half an hour. Selected and pasted text count. This estimates input effort, not physical typing or observed attention.

The writer preserves raw records and stores Human-only exclusions and retained lengths separately. Exact confirmations, source-marked skill context, Claude Code task notifications (marked by Claude Code's own `origin.kind`), complete heartbeat envelopes and established agent-created conversation inputs contribute no Human characters. Session-origin exclusions are an accepted assumption and can undercount a later person takeover. Retained input does not prove personal submission.

Messages belong to the exact half-open timestamp window and the local day containing that timestamp. Every eligible message contributes its complete retained length, even at the window start or alongside another message. There is no response-gap estimate, cap, clipping or overlap union. The compatibility summed-session field equals the total. Missing eligible length or classification makes the entire estimate, ratio and daily series unknown; an excluded missing length does not. Empty windows produce zero Human time and an unknown ratio.

Measured agent spans remain unchanged and share a read snapshot with Human characters. The ratio is measured agent minutes divided by estimated Human minutes when the latter is positive.

Synthetic checks cover configured rates, 12,000 characters, simultaneous sessions, midnight and DST boundaries, leap seconds, submillisecond membership, unknown lengths, empty windows, metadata-only replay and read snapshot consistency. Current private-data acceptance is recorded outside the repository.
