# CLAUDE.md

## General requirements
* this is an open source project meant to be publically visible on Github and thus SHOULD NOT contain
  any PII data such as IP addresses, domain names, user names or addresses or local paths.

## Development
This project uses a NixOS flake for development. It is linux only. Always enter the dev shell first:
```bash
nix develop
```

### Git worktrees
Use the `.worktrees` as the location for all feature branches such that a new worktree Foo would be
`.worktrees/Foo`

### On Change
Whenever a change is made to the code:

#### Restarts
Call out which components will need restarted for the changes to take effect for local dev

#### Rev the versions
Update versions as needed to comply with semantic versioning. Consider each component:
* server
* agent
* mobile
* control deck

## App design
Firefight is a simple Flutter (frontend) + Rust (backend) that provides:
* a set of configurable timers that when completed fire off arbitrary linux commands and/or sounds.
* actions are meant to run on the clients only as a means of timed orchestration e.g. logout or shutdown.
* SQLite shall be used for storing timers/alarms, clients and other configuration

### Server

#### Flutter frontend
* a server control deck UI for administration where multiple timers/alarms can be configured
* server control view should allow for triggering start of timers

##### Timer/alarm cards
Each Timer/alarm should show up in the control deck as a card.
* timer/alarm cards should be assignable to clients
* timers should be reusable
* when started on the server the client should receive updates to update their display with the correct time

##### Client configuration
Each client will connect to the server and show up in the server's UI as a card
* client cards should support a drag and drop assignment of timers/alarms cards

#### Rust Backend
* an API clients will connect to to get their timer instructions

### Client
* Simple UI that shows the current timers/alarms the client is configured for
* Client UI's timers/alarms are updated by current server values and kept in sync
