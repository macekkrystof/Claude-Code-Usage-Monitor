# Follow-up TODO

## Per-account Codex tray icons

The Classic theme currently exposes one aggregate Codex notification-area icon.
Add one tray icon for every visible Codex account so a second account is
recognisable beside the existing Claude/Codex icons. Keep icon ordering aligned
with `codex_accounts`, use each account's configured colour, and include the
account name in the tooltip. Hidden accounts must not register an icon.

Add renderer and tray synchronisation tests for zero, one, and multiple
accounts, including account removal and Explorer/taskbar restart recovery.

## Show Codex reset availability

Show the number of resets available for each Codex account next to its usage
values when the upstream response provides that information. First identify the
authoritative Codex/OpenAI field and its exact meaning; the current usage model
only guarantees reset timestamps, so the UI must not infer or invent a count.
Keep the value account-specific, localised, and safe when the field is missing.

Add parser, cache, template, widget, and tray-tooltip tests for present,
missing, and malformed reset-count values.

## Multi-monitor taskbar placement

Revisit dragging and persisted placement when the widget is embedded in a
taskbar on a single-monitor or changed-monitor setup. Preserve the current
visible clamping behaviour, but make cross-taskbar movement and the saved
offset predictable after monitor topology or DPI changes.

