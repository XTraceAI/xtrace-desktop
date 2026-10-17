# Telemetry

XTrace Desktop sends a small number of anonymous usage events so we can count
how many people use the app and how often. This page lists everything that can
be sent. A test in the app checks that this list and the code match exactly.

## What is never sent

No repository names, file or folder paths, prompts or other conversation text,
session titles, pull-request URLs, token counts, costs, or any raw identifier
read from your agent history. Every property below is either a fixed word from
a short list, the app's version, or your macOS version number.

## When telemetry is on

Telemetry is **on by default**. Nothing is sent until the app has shown you a
notice that says so (on the welcome page, or once in the main window after an
update). Events attempted before that are dropped, not saved for later.

You can turn it off at any time:

- Click **Turn off** in that notice.
- Use the **Telemetry** switch in **Settings**.
- Start the app with `XTRACE_TELEMETRY=0` in its environment. Also accepted:
  `XTRACE_TELEMETRY=false`, `XTRACE_TELEMETRY=off`, `XTRACE_TELEMETRY=no`
  (any capitalization). This always wins over the saved setting, and the
  Settings switch cannot turn telemetry back on while it is set.

Turning telemetry off takes effect at once: events waiting to be sent or
retried are dropped, a request in progress is stopped where possible, and
nothing dropped is sent later, even if you turn telemetry back on. An event the
server had already accepted cannot be recalled.

Only the app built with its own identifier, `ai.xtrace.app`, and using its own
data folder ever sends. Copies built with another identifier never send
anything, and neither do runs that keep their data in another folder
(`XTRACE_DATA_DIR`) or open disposable test data (fixture mode). A build made with
`XTRACE_POSTHOG_KEY` set to an empty value has no analytics project key and
never sends anything either.

## Events

<!-- telemetry-events:start -->

| Event                  | Properties of its own | Sent by this build                                                                                       |
| ---------------------- | --------------------- | -------------------------------------------------------------------------------------------------------- |
| `app_opened`           | none                  | Yes: when the app starts.                                                                                |
| `daily_active`         | none                  | Yes: at most once per local calendar day, when the app starts or its window or menu-bar popover is used. |
| `hourly_active`        | none                  | Yes: when the main window or the menu-bar popover is used, at most once an hour.                         |
| `plugin_detected`      | `host`                | Not sent yet.                                                                                            |
| `plugin_installed`     | `host`                | Not sent yet.                                                                                            |
| `rule_activated`       | `class`, `mode`       | Not sent yet.                                                                                            |
| `leaderboard_opted_in` | none                  | Not sent yet.                                                                                            |
| `cloud_connected`      | none                  | Not sent yet.                                                                                            |
| `update_installed`     | none                  | Not sent yet.                                                                                            |

<!-- telemetry-events:end -->

"Not sent yet" events are defined in the code so their shape is fixed, but the
features that would send them are not in the app yet.

Values of the event-specific properties:

- `host`: `claude`, `codex` or `cursor`.
- `class`: `hook`, `linter`, `test` or `judge`.
- `mode`: `advise` or `gate`.

## Properties on every event

<!-- telemetry-common:start -->

| Property         | Values                                                                                     |
| ---------------- | ------------------------------------------------------------------------------------------ |
| `surface`        | Always `desktop`.                                                                          |
| `app_version`    | The app's version, for example `0.1.0`.                                                    |
| `os_version`     | The macOS version number, for example `15.5`, or `unknown`.                                |
| `hosts_detected` | Which coding agents the local index holds history for: any of `claude`, `codex`, `cursor`. |

<!-- telemetry-common:end -->

Each event also carries:

- `distinct_id`: a random ID created on this Mac the first time an event is
  sent, saved in the app's database. It is not derived from your name, email,
  account or hardware.
- `timestamp`: when the event was made.
- `$set_once: {"desktop_user": true}`: marks the anonymous ID as a desktop user
  in PostHog.

## Where events go

Events are sent from the app's Rust code (not the web view) as an HTTPS request
to PostHog. No
analytics script runs in the app window, and there is no automatic click
tracking or session recording.

PostHog derives an approximate location (country and city) from the request's
IP address. Whether PostHog also stores the IP address itself is a setting of
the XTrace PostHog project, outside the app; the project is meant to have IP
storage turned off so only the derived location is kept.

## Retention

On this Mac, the app keeps a record of the last 200 events it tried to send,
with the properties sent and whether each was sent, is still waiting, or failed.
Older entries are deleted automatically. In PostHog, events are kept according
to the XTrace project's retention settings.

## Planned, not in this build

When connecting to XTrace Cloud exists, connecting will link this anonymous ID
to your XTrace account, so usage before and after connecting counts as one
person. The connect screen will say so before you connect. This build has no
cloud connection, so the ID is never linked to an account.
