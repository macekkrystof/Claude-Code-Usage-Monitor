# Follow-up TODO

All follow-ups in this file are complete for v2.4.0.

## Per-account Codex tray icons

- [x] Classic creates one notification-area icon per Codex account, in
  `codex_accounts` order.
- [x] Each icon uses the account's configured colour and includes the account
  name in its tooltip.
- [x] Hidden accounts remain in the theme data for stable identity but do not
  register an icon with Explorer.
- [x] Renderer and tray synchronisation cover zero, one, and multiple accounts,
  account removal, and Explorer/taskbar restart recovery.

## Show Codex reset availability

- [x] Parse the authoritative
  `rate_limit_reset_credits.available_count` response field without inferring
  a value from reset timestamps. OpenAI documents the same value as
  `rateLimitResetCredits.availableCount`.
- [x] Keep the count account-specific, cached, localised, and safe when the
  field is missing or malformed.
- [x] Show it next to account usage values and in the account tray tooltip.
- [x] Cover parser, cache, template, widget, and tray-tooltip behaviour for
  present, zero, missing, null, and malformed values.

## Multi-monitor taskbar placement

- [x] Persist the Windows monitor device id alongside the legacy display index.
- [x] Resolve saved placement by stable monitor id first, with a visible-index
  fallback when the monitor is unavailable.
- [x] Retarget a dragged widget across taskbars and re-normalise saved offsets
  after monitor topology or DPI changes while preserving visible clamping.
- [x] Cover monitor reordering, missing monitor ids, cross-taskbar movement, and
  logical offset clamping with focused tests.

