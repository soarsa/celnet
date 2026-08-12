# Simulator service identities

How the LP simulator and the FIX RFQ/ESP client authenticate against a running Celnet
server, why they hold no authority, and where their credentials live.

## 1. The problem this replaces

Both simulators used to authenticate as `admin@celnet.com`, with the password passed on
the command line:

```
lp-sim --lp-name LP-SIM ... --user admin@celnet.com --password <SECRET> --book-poll 10
```

That is three independent defects:

1. **The secret was world-readable.** A process command line is public on a Unix box —
   `ps -ef`, `ps -ww -o args=`, `/proc/<pid>/cmdline` — so every local account could read
   the administrator's password just by listing processes. No file permission protects
   against this; the only fix is to never put a secret in `argv`.
2. **A price simulator held administrative capability.** The admin role is grant-all and
   never narrowable: user CRUD, password resets, FIX-connection administration, the
   risk-routing graph. A daemon that publishes synthetic Treasury quotes needs none of
   it. If the simulator were ever compromised — or simply buggy — it had the authority to
   rewrite the identity store.
3. **One shared credential.** Both simulators used the same admin account, so revoking
   or rotating either one meant revoking the administrator and every other consumer.

## 2. What each simulator actually needs

Determined by reading the code, not by assumption. Each simulator makes **exactly one**
authenticated server call:

| Simulator | Binary | Authenticated call | Gate in `services/auth.rs` |
|---|---|---|---|
| LP feed | `lp-sim` | `AuthService.ListAggregatedBooks` | `self.authenticate(&req.session_token)?` — **no capability check** |
| FIX client | `fix_rfq_client --asset esp` | `AuthService.ListInstruments` | `self.authenticate(&req.session_token)?` — **no capability check** |

Both handlers carry an explicit in-code rationale for being ungated:

> `list_aggregated_books`: "Read-only roster: a book is global (ADR-0022 decision C), so
> any authenticated caller may read its composite — NOT admin-gated."

> `list_instruments`: "Read-only roster: any authenticated caller may read it
> (curve-building and pricing resolve against it), so it is NOT admin-gated."

Everything else the simulators do is **unauthenticated by design**:

- `lp-sim` pushes `LpQuote`s into the `LpFeed` backend ingest, which is a backend feed
  with no session;
- `fix_rfq_client` speaks the FIX 4.4 acceptor as a counterparty session, authenticated
  by the FIX logon, not by a Celnet session token.

**Therefore the minimum authority that actually works is: "can hold a session, holds no
capability."**

## 3. How "holds nothing" is expressed

Celnet resolves authority as:

```
effective  =  role bundle  ∪  per-user grants  ∖  per-user denies      (deny wins)
```

Two properties of the model constrain the implementation:

- **`Role::Admin` is grant-all and never narrowable** (`sessions.rs`), so a service
  account must be `Role::Trader`.
- **The Trader bundle is global** — `IdentityStore::role_base` returns one bundle shared
  by every trader — so it cannot be narrowed for the service accounts without narrowing
  it for every human trader on the platform.

So each service account is:

- `role`: `trader`
- `desk_ids`: `[]`, `all_desks`: `false` — **deskless**, so no desk's inbound RFQ/IOI
  flow is visible to it
- `capability_grants`: `[]`
- `capability_denies`: **every capability the trader bundle confers** — deny wins, so the
  effective set resolves **empty**

The provisioner asserts this rather than assuming it: after applying the denies it reads
the server's own resolved `effective` list back and refuses to report success unless it
is empty, then logs in as the account and confirms that (a) the one RPC the simulator
needs **succeeds**, and (b) an admin-gated RPC (`list_users`) is **refused**.

### 3.1 The honest caveat

Celnet has **no distinct action** for "read the aggregated-book roster" or "read the
instrument roster", because both RPCs are gated on *authentication alone* rather than on
a capability. An empty capability set is therefore the **narrowest authority the model
can express** for these accounts — and it happens to be exactly sufficient.

The residual authority is *"may hold a valid session"*. That is not separately revocable
short of setting `disabled: true` on the account. Narrowing it further would require a
new `Action` (e.g. `Action::ReadRoster`) and re-gating those two RPCs on it — a contract
change well beyond the scope of removing the admin credential, and one that would buy
little: both rosters are already readable by every authenticated user on the platform.

We record this rather than silently granting `administer` and calling it least
privilege.

## 4. Where the credentials live

| | LP simulator | FIX simulator |
|---|---|---|
| Identity | `lp-sim@svc.celnet.local` | `fix-sim@svc.celnet.local` |
| Env prefix | `LPSIM` | `FIXSIM` |
| Password file | `/home/celnet/.lpsim_pw` (`0600`) | `/home/celnet/.fixsim_pw` (`0600`) |
| At rest on the server | Argon2id PHC hash in `identity.json` | same |

**Separate files on purpose**: revoking or rotating one simulator never disturbs the
other.

### 4.1 Resolution precedence

Implemented once in `celnet_lp_sim::credentials` and mirrored in the FIX client's
`resolve_esp_password` (the FIX crate sits *below* `celnet-lp-sim` in the dependency
order and must not point upward):

1. **`<PREFIX>_PASSWORD_FILE`** — path to a file whose entire trimmed contents are the
   password. When the variable is set the file **must** be readable and non-empty: an
   unreadable or blank file is a hard error, **never** a silent fall-through to a weaker
   source. (A silent fallback is how a rotated box quietly keeps authenticating on a
   stale secret.) This is the deployed form.
2. **`<PREFIX>_PASSWORD`** — the literal secret in the environment. For a developer shell
   and for container secret injection.
3. **Nothing ⇒ a hard, actionable startup error.** There is deliberately **no** built-in
   default password: a daemon with no configured credential fails loudly rather than
   silently authenticating as a well-known seeded account.

The identity (`<PREFIX>_USER`) follows the same shape but *does* carry a default — an
identity is not a secret, and defaulting it keeps the deployed unit self-describing.

### 4.2 There is no `--password` flag

`lp-sim` has no such argument at all. `fix_rfq_client` recognises `--password` **only to
reject it** with an explanatory message, so an operator who copies an old command line
gets a clear error instead of a silent authentication failure — or worse, a silently
leaked secret.

`ServiceCredentials` implements `Debug` **without** the secret (rendered `<redacted>`),
so a `{:?}` of any config struct that embeds it cannot leak the password into a log line.

Verify on a running box:

```sh
ps -ww -o args= -C lp-sim | grep -c password    # → 0
```

## 5. Provisioning

`deploy/provision-sim-identities.mjs` is the single source of truth. It is dependency-free
(a minimal RFC 6455 client — the deploy box has no `node_modules`) and drives the **admin
API over the WebSocket mirror**, so the server rewrites `identity.json` itself and never
has to be stopped.

```sh
# create/repair both identities
CELNET_ADMIN_PASSWORD_FILE=~/.celnet_admin_pw node provision-sim-identities.mjs

# inspect without changing anything
node provision-sim-identities.mjs --dry-run

# also rotate the admin password (printed once, on stdout)
CELNET_ADMIN_PASSWORD_FILE=~/.celnet_admin_pw node provision-sim-identities.mjs --rotate-admin
```

### 5.1 Idempotency

A re-run is a no-op on an already-correct box:

- an existing password file is **reused, never regenerated**, so a re-run does not
  invalidate the credential the running daemons already loaded;
- the password is reset **only** when a login with the file's contents actually fails;
- the deny set is recomputed as `role bundle ∪ current grants ∪ current denies`, so it
  converges to the same set every time. Deriving it from the *bundle* rather than from
  the (now empty) `effective` set is what stops a second run from **un-locking** an
  already-locked account.

The Ansible role installs the script and runs it when an admin credential file is present
on the box, skipping with an explanatory message when it is not — a box whose identities
are already provisioned needs no admin credential to deploy.

### 5.2 Secrets are generated, never committed

Passwords are drawn from the CSPRNG (40 characters over a 56-symbol alphabet, ≈232 bits).
The alphabet is deliberately punctuation-free: these secrets are read by `sh` launchers,
and a quoting bug must never be able to truncate or reinterpret a credential.

No secret appears in this repository, in `group_vars`, in a generated control script, or
in any log line. The generated `lpsimctl`/`fixsimctl` scripts export only the *path* to
the credential file; an inline `celnet_lpsim_password` in `group_vars` is still honoured
for a developer box but is explicitly marked discouraged in the template.

## 6. Related

- `crates/celnet-lp-sim/src/credentials.rs` — the resolution seam and its tests
- `crates/celnet-server/src/config/identity.rs` — `UserDef`, `Role`, `PermissionGrant`
- `crates/celnet-entitlements/` — the `Action × AssetClass` capability kernel
- `docs/PERMISSIONS-GRANULAR-REVIEW.md` — the action taxonomy and what is held back from
  the default trader bundle
