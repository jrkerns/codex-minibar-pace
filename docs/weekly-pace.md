# Weekly workday pace

Codex Minibar Pace is a fork of [Codex Minibar](https://github.com/vertopolkaLF/codex-minibar), licensed under Apache-2.0. The larger taskbar display is inspired by [upstream-ray/codex-usage-monitor](https://github.com/upstream-ray/codex-usage-monitor). Its source was used for reference; the implementation extends Minibar's own taskbar host and renderer.

## Run the portable build

Extract the entire portable ZIP to a permanent folder, keeping `assets` beside `codex-minibar-pace.exe`. Launch that executable. Sign in to Codex and/or Claude locally for automatic quota reads. The existing Claude reader supports Claude Code and Claude Desktop.

A fresh configuration enables the weekly pace widget, Monday–Friday 09:00–17:00 in the computer's local timezone. Right-click the widget, open Settings, and select Taskbar to change working days and start/end times. Select a widget and the Weekly pace style to choose a Codex or Claude account.

Positive means quota in reserve. If 40% of scheduled work time has elapsed and 30% of quota is used, pace is **+10 percentage points**. This is a difference in allowance, not a relative burn-rate forecast. Negative means quota spent ahead of the plan. Values within two points use a neutral color.

The taskbar row shows the icon, signed pace number, and a remaining-quota progress bar. The marker on the bar shows expected remaining quota for the current work schedule. Labels and reset countdowns are kept in hover details. Hover for actual and expected usage, the precise pace difference, schedule, reset date, and last sample time. During a refresh failure or after samples age beyond twice the configured refresh interval plus two minutes, pace is unavailable; the retained quota bar is gray. Cached data without a known interval uses a 35-minute maximum age.

Work intervals are clipped to the reported quota window. Nights and excluded days do not advance expected usage, but actual spending still counts. Schedule changes recalculate the entire current cycle. No quota is refilled locally when a reset passes: a successful read must supply the next window.

Turning off working-hours pacing uses the original calendar-time comparison. Working-hours pacing applies to Codex and Claude weekly quotas; five-hour quotas and other providers keep their existing calculation. Working hours must stay within one day and include at least one weekday selection. Breaks, holiday exceptions, and per-day hours are not included in this version.

## Claude rotation

On startup, the fork detects a local Claude installation without launching it and enables its existing quota reader once. Later explicit provider disablement is respected. If Claude is installed after startup, enable it in Providers or restart Minibar. A detected installation still needs a signed-in account with a weekly allowance; unavailable or stale quota shows a dash with an explanation on hover. No extra quota requests are made for rotation.

With **Rotate Codex and Claude** enabled in Taskbar settings (the default, including upgrades), each Weekly pace widget switches every five seconds between its preferred account and the first enabled account of the other provider. A single enabled provider stays visible. The provider icon, signed number, bar, marker, hover details, and click destination change together. Hovering, pressing, or reading an open hover card pauses rotation; the next switch is five seconds after interaction ends. Disable rotation to keep the selected account visible.

## Separate from upstream

The executable, config/history directories, startup registration, Windows notification identity, taskbar window classes, and single-instance lock are distinct from upstream Minibar. Updates are read only from `jrkerns/codex-minibar-pace` releases. Existing upstream settings are never loaded automatically. To reuse them, import an exported settings file; imports without a pace schedule retain calendar pacing and their existing widget arrangement.

The inherited optional Stream Deck and Command Palette packages remain upstream components and are not part of this portable fork's acceptance checks.

## Acceptance checklist

These checks require the user's desktop; automated verification never launches the app.

1. Launch the portable executable and confirm the signed pace number is readable beside the notification area. Check hover details against the Codex usage screen.
2. Change working hours and exclude/include weekends. Confirm the widget, preview, and Codex weekly popup marker update without a restart; confirm the five-hour display is unchanged.
3. Select another configured Codex account and confirm the widget and popup refer to the same account.
4. Check Windows light/dark taskbars and 100%, 125%, 150%, and 200% scaling. Confirm the number and bar fit at the taskbar's normal and compact heights.
5. Crowd the taskbar. The progress bar shrinks, then disappears if needed; the signed pace must remain visible. Existing taskbar placement limitations still apply when no room remains.
6. Disconnect the network, refresh, and confirm PACE becomes unavailable while retained quota is marked stale. Reconnect and refresh to restore it.
7. Sleep/wake and restart Explorer. Confirm the widget returns in its expected position and recalculates. Check existing supported monitor placements.
8. Run upstream Minibar alongside the fork to confirm separate settings and tray access.

9. With both providers enabled, watch two rotations and confirm the icon, pace, bar and click destination all match. Hover for longer than five seconds and verify switching pauses. Disable Claude and confirm Codex stays visible; disable rotation and select Claude to pin it.

## Automated verification

`.github/workflows/pace.yml` runs formatting, compile checks, all-target/all-feature tests, Clippy, native taskbar preview rendering, and a portable x64 build on a Windows runner. It uploads preview PNGs and the ZIP as a workflow artifact. The build is unsigned and optimized with LTO disabled to keep fork validation practical. It does not install or launch the app.
