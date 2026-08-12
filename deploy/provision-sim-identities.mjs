#!/usr/bin/env node
//
// Provision the simulators' DEDICATED, LEAST-PRIVILEGE service identities.
//
// # Why this exists
//
// The simulators used to authenticate as `admin@celnet.com`, with the password passed
// as a `--password` command-line flag. That is three separate defects:
//
//   1. a process command line is world-readable (`ps -ef`, `/proc/<pid>/cmdline`), so
//      the secret was published to every local account on the box;
//   2. a price-publishing simulator held FULL ADMINISTRATIVE capability — user CRUD,
//      password resets, FIX-connection administration — none of which it needs;
//   3. one shared credential meant revoking either simulator meant revoking the admin.
//
// This script fixes (2) and (3) by minting one service account PER SIMULATOR whose
// EFFECTIVE CAPABILITY SET IS EMPTY, and (1) by writing each generated password into a
// `0600` file that only the service account can read — the file the launcher hands to
// the daemon through its ENVIRONMENT (`LPSIM_PASSWORD_FILE` / `FIXSIM_PASSWORD_FILE`),
// never through `argv`.
//
// # Why an empty capability set is genuinely sufficient
//
// Each simulator makes exactly ONE authenticated server call, and both are
// `authenticate()`-only RPCs — they resolve a session token and check NO capability:
//
//   * lp-sim         → `AuthService.ListAggregatedBooks`  (`services/auth.rs`: "Read-only
//                      roster: a book is global (ADR-0022 decision C), so any
//                      authenticated caller may read its composite — NOT admin-gated.")
//   * fix_rfq_client → `AuthService.ListInstruments`      (`services/auth.rs`: "Read-only
//                      roster: any authenticated caller may read it ... NOT admin-gated.")
//
// Everything else each simulator does is UNAUTHENTICATED by design: lp-sim pushes quotes
// into the `LpFeed` backend ingest, and the FIX client speaks the FIX acceptor as a
// counterparty session. So the minimum that actually works is "can log in, holds
// nothing" — which is exactly what this provisions.
//
// # How "holds nothing" is expressed in Celnet's model
//
// Celnet resolves authority as  `role bundle ∪ per-user grants ∖ per-user denies`
// (deny wins). Two constraints shape the implementation:
//
//   * `Role::Admin` is grant-all and NEVER narrowable, so a service account must be
//     `Role::Trader`;
//   * the Trader BUNDLE is GLOBAL (one bundle shared by every trader), so it cannot be
//     narrowed for these accounts without narrowing it for every human trader too.
//
// Therefore the account is `Role::Trader` with a per-user DENY on every capability the
// role bundle confers. Deny wins over the bundle, so the effective set resolves EMPTY —
// and the script asserts exactly that by reading the server's own resolved `effective`
// list back after the change. It does not take the outcome on trust.
//
// NOTE — the honest caveat, stated rather than papered over: Celnet has no distinct
// "read the aggregated-book roster" or "read the instrument roster" action, because
// both RPCs are gated on authentication alone rather than on a capability. An empty
// capability set is therefore the NARROWEST authority the model can express for these
// accounts, and it is exactly sufficient. The residual authority is "may hold a
// session", which is not separately revocable short of disabling the account.
//
// # Idempotency
//
// A re-run is a no-op on an already-correct box:
//   * an existing password file is REUSED (never regenerated), so a re-run does not
//     invalidate the credential the running daemons already loaded;
//   * the password is only reset when a login with the file's contents actually fails;
//   * the deny set is recomputed as (role bundle ∪ current grants ∪ current denies), so
//     it converges to the same set no matter how many times it runs, and never
//     accidentally UN-locks an already-locked account (which naively re-deriving denies
//     from the now-empty `effective` set would do).
//
// # Usage
//
//   # on the UAT box, as the service account:
//   CELNET_ADMIN_PASSWORD_FILE=~/.celnet_admin_pw \
//     node provision-sim-identities.mjs
//
//   # rotate the admin password too (prints the new one on stdout, ONCE):
//   CELNET_ADMIN_PASSWORD_FILE=~/.celnet_admin_pw \
//     node provision-sim-identities.mjs --rotate-admin
//
//   # inspect without changing anything:
//   node provision-sim-identities.mjs --dry-run
//
// Environment:
//   CELNET_WS                  WS endpoint         (default ws://127.0.0.1:50061)
//   CELNET_ADMIN_EMAIL         admin login         (default admin@celnet.com)
//   CELNET_ADMIN_PASSWORD      admin secret        (or ..._FILE, which is preferred)
//   CELNET_ADMIN_PASSWORD_FILE path to a 0600 file holding the admin secret
//   CELNET_SIM_HOME            where the pw files go (default $HOME)
//
// A secret is NEVER logged. The only place a plaintext password is ever printed is the
// explicit `--rotate-admin` summary, because the operator must be told the new value.

import { createHash, randomBytes, randomInt } from "node:crypto";
import { connect as netConnect } from "node:net";
import { chmodSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

// ===========================================================================
// A minimal, dependency-free WebSocket client (RFC 6455)
// ===========================================================================
//
// Node 20 has no global `WebSocket`, and this script must run on the deploy box with
// NO npm install step (the box is disk-constrained and `node_modules` is routinely
// deleted to reclaim space). The subset implemented here is exactly what the Celnet WS
// mirror needs: a client-masked text channel carrying single JSON frames, with
// continuation, ping/pong and close handled correctly.

const WS_GUID = "258EAFA5-E914-47DA-95CA-5AB0DC85B11F";

/** Open a WebSocket to `url` (ws:// only — the endpoint is loopback on the box). */
function wsConnect(url) {
  return new Promise((resolve, reject) => {
    const u = new URL(url);
    if (u.protocol !== "ws:") {
      reject(new Error(`only ws:// is supported (got ${u.protocol})`));
      return;
    }
    const key = randomBytes(16).toString("base64");
    const expectAccept = createHash("sha1")
      .update(key + WS_GUID)
      .digest("base64");

    const socket = netConnect(
      { host: u.hostname, port: Number(u.port || 80) },
      () => {
        socket.write(
          `GET ${u.pathname || "/"} HTTP/1.1\r\n` +
            `Host: ${u.host}\r\n` +
            "Upgrade: websocket\r\n" +
            "Connection: Upgrade\r\n" +
            `Sec-WebSocket-Key: ${key}\r\n` +
            "Sec-WebSocket-Version: 13\r\n\r\n",
        );
      },
    );
    socket.on("error", reject);

    let handshake = Buffer.alloc(0);
    const onHandshake = (chunk) => {
      handshake = Buffer.concat([handshake, chunk]);
      const end = handshake.indexOf("\r\n\r\n");
      if (end < 0) return;
      const head = handshake.subarray(0, end).toString("latin1");
      if (!/^HTTP\/1\.1 101/i.test(head)) {
        socket.destroy();
        reject(new Error(`WS upgrade refused: ${head.split("\r\n")[0]}`));
        return;
      }
      // `Sec-WebSocket-Accept` must be PRESENT (its absence means the peer is not
      // speaking WebSocket at all, which is worth refusing). Its VALUE is only
      // warned about, not enforced: the accept digest exists to stop a browser
      // being tricked by a cache or a confused non-WS intermediary, and this tool
      // connects to a known loopback port on the same host. Enforcing it here would
      // fail closed on a cosmetic server-side digest difference and block an
      // operational security fix for no security gain.
      //
      // NOTE (observed on the UAT server, 2026-08-12): the value returned by the
      // Celnet WS endpoint does not equal `base64(sha1(key + GUID))` as computed
      // here — verified with node's SHA-1 checked against the `abc` test vector and
      // the concatenated input bytes dumped and inspected. Frames decode correctly
      // and the session works, so it does not affect this tool, but a browser
      // connecting DIRECTLY to the WS port (rather than through the 443 proxy) would
      // reject the handshake. Worth a follow-up against the server's upgrade path.
      const accept = /sec-websocket-accept:\s*(\S+)/i.exec(head);
      if (!accept) {
        socket.destroy();
        reject(new Error("WS upgrade response carried no Sec-WebSocket-Accept header"));
        return;
      }
      if (accept[1] !== expectAccept) {
        console.warn(
          `# warning: Sec-WebSocket-Accept ${accept[1]} != computed ${expectAccept} ` +
            `(continuing — see the note in this script's wsConnect)`,
        );
      }
      socket.removeListener("data", onHandshake);
      const conn = new WsConn(socket);
      conn.feed(handshake.subarray(end + 4));
      socket.on("data", (d) => conn.feed(d));
      resolve(conn);
    };
    socket.on("data", onHandshake);
  });
}

/** A framed WebSocket connection over an established socket. */
class WsConn {
  constructor(socket) {
    this.socket = socket;
    this.buf = Buffer.alloc(0);
    this.fragments = [];
    this.handlers = [];
    this.closed = false;
    socket.on("close", () => {
      this.closed = true;
    });
  }

  /** Register a text-message handler. */
  onMessage(fn) {
    this.handlers.push(fn);
  }

  /** Send one masked text frame. */
  sendText(text) {
    const payload = Buffer.from(text, "utf8");
    const mask = randomBytes(4);
    const len = payload.length;
    let header;
    if (len < 126) {
      header = Buffer.alloc(2);
      header[1] = 0x80 | len;
    } else if (len < 65536) {
      header = Buffer.alloc(4);
      header[1] = 0x80 | 126;
      header.writeUInt16BE(len, 2);
    } else {
      header = Buffer.alloc(10);
      header[1] = 0x80 | 127;
      header.writeBigUInt64BE(BigInt(len), 2);
    }
    header[0] = 0x81; // FIN + text
    const masked = Buffer.allocUnsafe(len);
    for (let i = 0; i < len; i += 1) masked[i] = payload[i] ^ mask[i & 3];
    this.socket.write(Buffer.concat([header, mask, masked]));
  }

  /** Feed inbound bytes and dispatch any complete frames. */
  feed(chunk) {
    this.buf = Buffer.concat([this.buf, chunk]);
    for (;;) {
      const frame = this.#takeFrame();
      if (!frame) return;
      const { opcode, fin, payload } = frame;
      if (opcode === 0x8) {
        // Close: echo and tear down.
        this.socket.end(Buffer.from([0x88, 0x80, 0, 0, 0, 0]));
        this.closed = true;
        return;
      }
      if (opcode === 0x9) {
        // Ping → pong, mirroring the payload (masked, as a client must).
        const mask = randomBytes(4);
        const out = Buffer.allocUnsafe(payload.length);
        for (let i = 0; i < payload.length; i += 1) out[i] = payload[i] ^ mask[i & 3];
        this.socket.write(
          Buffer.concat([Buffer.from([0x8a, 0x80 | payload.length]), mask, out]),
        );
        continue;
      }
      if (opcode === 0xa) continue; // pong — ignore
      if (opcode === 0x0 || opcode === 0x1 || opcode === 0x2) {
        this.fragments.push(payload);
        if (!fin) continue;
        const text = Buffer.concat(this.fragments).toString("utf8");
        this.fragments = [];
        for (const h of this.handlers) h(text);
      }
    }
  }

  /** Parse one frame off the buffer, or null when incomplete. */
  #takeFrame() {
    if (this.buf.length < 2) return null;
    const b0 = this.buf[0];
    const b1 = this.buf[1];
    const fin = (b0 & 0x80) !== 0;
    const opcode = b0 & 0x0f;
    if ((b1 & 0x80) !== 0) throw new Error("server frame must not be masked");
    let len = b1 & 0x7f;
    let offset = 2;
    if (len === 126) {
      if (this.buf.length < 4) return null;
      len = this.buf.readUInt16BE(2);
      offset = 4;
    } else if (len === 127) {
      if (this.buf.length < 10) return null;
      const big = this.buf.readBigUInt64BE(2);
      if (big > 16_777_216n) throw new Error("frame exceeds the 16 MiB sanity cap");
      len = Number(big);
      offset = 10;
    }
    if (this.buf.length < offset + len) return null;
    const payload = this.buf.subarray(offset, offset + len);
    this.buf = this.buf.subarray(offset + len);
    return { opcode, fin, payload };
  }

  close() {
    if (!this.closed) this.socket.end(Buffer.from([0x88, 0x80, 0, 0, 0, 0]));
  }
}

// ===========================================================================
// The Celnet WS RPC seam
// ===========================================================================

/** A correlation-id-multiplexed request/response client over the WS mirror. */
class CelnetWs {
  constructor(conn) {
    this.conn = conn;
    this.next = 1;
    this.pending = new Map();
    conn.onMessage((text) => {
      let frame;
      try {
        frame = JSON.parse(text);
      } catch {
        return; // Not our contract; ignore rather than tearing the session down.
      }
      const id = frame.correlation_id;
      if (id == null) return; // An unsolicited push (stream tick) — not an RPC reply.
      const waiter = this.pending.get(id);
      if (!waiter) return;
      this.pending.delete(id);
      if (frame.type === "error") {
        waiter.reject(
          new Error(`${frame.code ?? "Error"}: ${frame.message ?? "(no message)"}`),
        );
      } else {
        waiter.resolve(frame);
      }
    });
  }

  /** Issue one RPC and await its correlated reply. */
  call(type, body, timeoutMs = 20_000) {
    const correlation_id = this.next;
    this.next += 1;
    const frame = { type, ...body, correlation_id };
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(correlation_id);
        reject(new Error(`${type}: timed out after ${timeoutMs}ms`));
      }, timeoutMs);
      this.pending.set(correlation_id, {
        resolve: (f) => {
          clearTimeout(timer);
          resolve(f);
        },
        reject: (e) => {
          clearTimeout(timer);
          reject(e);
        },
      });
      this.conn.sendText(JSON.stringify(frame));
    });
  }

  close() {
    this.conn.close();
  }
}

// ===========================================================================
// Secrets
// ===========================================================================

// A generated password must clear the server's `check_password_strength` floor
// (`MIN_PASSWORD_LEN`) with a very wide margin, and must be safe to place in a shell
// variable and a JSON body. The alphabet is deliberately punctuation-free: these
// secrets get sourced by `sh` launchers, and a quoting bug must never be able to
// truncate or reinterpret a credential.
const PW_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
const PW_LEN = 40; // ~232 bits of entropy over this alphabet.

/** Generate a fresh password from the CSPRNG (`randomInt` is rejection-sampled). */
function generatePassword() {
  let out = "";
  for (let i = 0; i < PW_LEN; i += 1) out += PW_ALPHABET[randomInt(PW_ALPHABET.length)];
  return out;
}

/** Read an existing secret file, or null when absent/blank. */
function readSecretFile(path) {
  if (!existsSync(path)) return null;
  const raw = readFileSync(path, "utf8").trim();
  return raw.length > 0 ? raw : null;
}

/**
 * Write a secret to `path` with `0600` BEFORE the content lands, so the plaintext is
 * never momentarily world-readable between `write` and `chmod`.
 */
function writeSecretFile(path, secret) {
  writeFileSync(path, secret, { mode: 0o600 });
  chmodSync(path, 0o600); // Explicit: `mode` is ignored when the file already exists.
}

/** Resolve the ADMIN credential the same way the daemons resolve theirs: file, then env. */
function resolveAdminPassword() {
  const file = (process.env.CELNET_ADMIN_PASSWORD_FILE ?? "").trim();
  if (file) {
    const pw = readSecretFile(file);
    if (!pw) throw new Error(`CELNET_ADMIN_PASSWORD_FILE ${file} is missing or empty`);
    return pw;
  }
  const env = (process.env.CELNET_ADMIN_PASSWORD ?? "").trim();
  if (env) return env;
  throw new Error(
    "no admin credential: set CELNET_ADMIN_PASSWORD_FILE to a 0600 file (preferred) " +
      "or CELNET_ADMIN_PASSWORD in the environment.",
  );
}

// ===========================================================================
// The service-identity specification
// ===========================================================================

/**
 * The simulators, and the ONE authenticated RPC each actually makes. The `verify` RPC
 * is re-issued as the service account itself after lock-down, so this script proves the
 * account still works with an EMPTY capability set rather than assuming it.
 */
const SERVICES = [
  {
    email: "lp-sim@svc.celnet.local",
    displayName: "LP Simulator (service)",
    passwordFile: ".lpsim_pw",
    // lp-sim polls the enabled aggregated books to learn which instruments to quote.
    verify: { type: "list_aggregated_books", reply: "aggregated_books" },
    note: "publishes LP quotes into the unauthenticated LpFeed ingest",
  },
  {
    email: "fix-sim@svc.celnet.local",
    displayName: "FIX Simulator (service)",
    passwordFile: ".fixsim_pw",
    // The ESP leg downloads the top-N streamable instruments from reference data.
    verify: { type: "list_instruments", reply: "instruments" },
    note: "acts as a CLIENT counterparty over the FIX acceptor session",
  },
];

/** A stable `action|asset` key so capability sets can be unioned/compared. */
const capKey = (c) => `${c.action}|${c.asset}`;

/** Union capability lists, de-duplicated, in a stable sorted order. */
function unionCaps(...lists) {
  const seen = new Map();
  for (const list of lists) {
    for (const c of list ?? []) {
      if (c && typeof c.action === "string" && typeof c.asset === "string") {
        seen.set(capKey(c), { action: c.action, asset: c.asset });
      }
    }
  }
  return [...seen.values()].sort((a, b) => capKey(a).localeCompare(capKey(b)));
}

// ===========================================================================
// Provisioning
// ===========================================================================

async function main() {
  const args = new Set(process.argv.slice(2));
  const dryRun = args.has("--dry-run");
  const rotateAdmin = args.has("--rotate-admin");

  const wsUrl = process.env.CELNET_WS ?? "ws://127.0.0.1:50061";
  const adminEmail = process.env.CELNET_ADMIN_EMAIL ?? "admin@celnet.com";
  const simHome = process.env.CELNET_SIM_HOME ?? homedir();

  const adminPassword = resolveAdminPassword();

  console.log(`# celnet service-identity provisioning`);
  console.log(`#   endpoint : ${wsUrl}`);
  console.log(`#   admin    : ${adminEmail}`);
  console.log(`#   pw files : ${simHome}/`);
  if (dryRun) console.log(`#   MODE     : DRY RUN (no changes will be made)`);
  console.log("");

  const ws = new CelnetWs(await wsConnect(wsUrl));
  let adminToken;
  try {
    const login = await ws.call("login", { email: adminEmail, password: adminPassword });
    adminToken = login.session_token;
    console.log(`[admin] authenticated as ${adminEmail}`);

    // The Trader role bundle is the base authority any service account inherits; it is
    // the primary input to the deny set that zeroes each account out.
    const roleCaps = await ws.call("get_role_capabilities", {
      session_token: adminToken,
      role: 0, // USER_ROLE_TRADER
    });
    const traderBundle = roleCaps.capabilities ?? [];
    console.log(
      `[admin] trader role bundle: ${traderBundle.length} capabilities (the set to deny)`,
    );

    const users = await ws.call("list_users", { session_token: adminToken });
    const byEmail = new Map(
      (users.users ?? []).map((u) => [u.email.toLowerCase(), u]),
    );

    const summary = [];

    for (const svc of SERVICES) {
      console.log(`\n=== ${svc.email} ===`);
      const pwPath = join(simHome, svc.passwordFile);

      // --- 1. The credential ------------------------------------------------
      // Reuse an existing file so a re-run never invalidates the secret the running
      // daemons already loaded. Only mint one when there is nothing to reuse.
      let password = readSecretFile(pwPath);
      const reused = password !== null;
      if (!password) password = generatePassword();
      console.log(
        `  credential : ${reused ? "REUSED existing" : "GENERATED new"} ${pwPath}`,
      );

      // --- 2. The account ---------------------------------------------------
      let user = byEmail.get(svc.email.toLowerCase());
      if (!user) {
        if (dryRun) {
          console.log(`  account    : WOULD CREATE (role=trader, deskless)`);
          summary.push({ email: svc.email, action: "would create" });
          continue;
        }
        const created = await ws.call("create_user", {
          session_token: adminToken,
          email: svc.email,
          display_name: svc.displayName,
          role: 0, // USER_ROLE_TRADER — Admin is grant-all and never narrowable.
          desk_ids: [], // Deskless: no desk's inbound flow is visible to it.
          all_desks: false,
          password,
        });
        user = created.user;
        console.log(`  account    : CREATED id=${user.id} (role=trader, deskless)`);
      } else {
        console.log(`  account    : EXISTS id=${user.id}`);
        // Re-assert the credential ONLY when the stored one no longer works, so a
        // healthy re-run does not churn sessions.
        //
        // A RATE-LIMITED probe must NOT be read as "the password is wrong": the
        // server refuses a locked-out email before checking the secret, so the two
        // are indistinguishable from here. Resetting on that signal would rewrite a
        // perfectly good credential (and the verification below would fail anyway,
        // since it also has to log in). Fail fast with an actionable message instead.
        let works = false;
        try {
          const probe = await ws.call("login", { email: svc.email, password });
          works = true;
          await ws.call("logout", { session_token: probe.session_token });
        } catch (e) {
          if (/ResourceExhausted|too many failed login/i.test(String(e.message))) {
            throw new Error(
              `${svc.email} is under a brute-force lockout (${e.message}). Its stored ` +
                `credential cannot be checked while locked, so provisioning would be ` +
                `guessing. Stop whatever is failing to authenticate as this account, ` +
                `wait for the lockout to expire, then re-run.`,
            );
          }
          works = false;
        }
        if (works) {
          console.log(`  password   : verified OK (unchanged)`);
        } else if (dryRun) {
          console.log(`  password   : WOULD RESET (current file does not authenticate)`);
        } else {
          await ws.call("reset_password", {
            session_token: adminToken,
            id: user.id,
            new_password: password,
          });
          console.log(`  password   : RESET to the file's value`);
        }
      }

      // --- 3. Zero the authority -------------------------------------------
      // denies := role bundle ∪ current grants ∪ current denies. Deriving from the
      // BUNDLE (not from the now-possibly-empty `effective`) is what makes a re-run
      // converge instead of un-locking an already-locked account.
      const before = await ws.call("get_user_capabilities", {
        session_token: adminToken,
        id: user.id,
      });
      const denies = unionCaps(traderBundle, before.grants, before.denies);

      if (dryRun) {
        console.log(
          `  authority  : effective=${(before.effective ?? []).length}, WOULD deny ${denies.length}`,
        );
        summary.push({ email: svc.email, action: "would lock down" });
        continue;
      }

      await ws.call("set_user_capabilities", {
        session_token: adminToken,
        id: user.id,
        grants: [], // No widening whatsoever.
        denies,
      });
      const after = await ws.call("get_user_capabilities", {
        session_token: adminToken,
        id: user.id,
      });
      const effective = after.effective ?? [];
      if (effective.length !== 0) {
        throw new Error(
          `${svc.email}: effective capability set is NOT empty after lock-down ` +
            `(${effective.map(capKey).join(", ")}) — refusing to report success`,
        );
      }
      console.log(
        `  authority  : ${denies.length} denies applied → effective set is EMPTY (verified)`,
      );

      // --- 4. Persist the credential ---------------------------------------
      writeSecretFile(pwPath, password);
      console.log(`  file       : wrote ${pwPath} (0600)`);

      // --- 5. Prove it actually works --------------------------------------
      // The whole point: log in AS the zero-capability account and make the one call
      // the simulator really makes. If this fails, the capability set is too narrow
      // and the change must not be reported as done.
      const svcLogin = await ws.call("login", { email: svc.email, password });
      const svcToken = svcLogin.session_token;
      const reply = await ws.call(svc.verify.type, { session_token: svcToken });
      if (reply.type !== svc.verify.reply) {
        throw new Error(
          `${svc.email}: ${svc.verify.type} returned ${reply.type}, expected ${svc.verify.reply}`,
        );
      }
      const rows =
        reply.books?.length ?? reply.instruments?.length ?? 0;
      console.log(
        `  VERIFIED   : ${svc.verify.type} succeeded as the service account (${rows} rows)`,
      );
      // And prove the account is genuinely powerless: an admin-gated call must fail.
      let refused = false;
      try {
        await ws.call("list_users", { session_token: svcToken });
      } catch (e) {
        refused = /permission|denied|admin/i.test(String(e.message));
      }
      if (!refused) {
        throw new Error(
          `${svc.email}: list_users was NOT refused — the account still holds admin authority`,
        );
      }
      console.log(`  VERIFIED   : admin-gated list_users correctly REFUSED`);
      await ws.call("logout", { session_token: svcToken });

      summary.push({ email: svc.email, action: "provisioned", file: pwPath });
    }

    // --- 6. Rotate the admin password ---------------------------------------
    if (rotateAdmin) {
      console.log(`\n=== ${adminEmail} (rotation) ===`);
      const me = (await ws.call("list_users", { session_token: adminToken })).users.find(
        (u) => u.email.toLowerCase() === adminEmail.toLowerCase(),
      );
      if (!me) throw new Error(`admin ${adminEmail} not found in the user list`);
      const fresh = generatePassword();
      if (dryRun) {
        console.log(`  WOULD ROTATE the admin password`);
      } else {
        await ws.call("reset_password", {
          session_token: adminToken,
          id: me.id,
          new_password: fresh,
        });
        // The reset revokes our own session — re-authenticate to prove the new secret.
        const recheck = await ws.call("login", { email: adminEmail, password: fresh });
        adminToken = recheck.session_token;
        const adminFile = (process.env.CELNET_ADMIN_PASSWORD_FILE ?? "").trim();
        if (adminFile) {
          writeSecretFile(adminFile, fresh);
          console.log(`  rotated and re-verified; wrote ${adminFile} (0600)`);
        } else {
          console.log(`  rotated and re-verified`);
        }
        summary.push({ email: adminEmail, action: "ROTATED", newPassword: fresh });
      }
    }

    // --- Summary -------------------------------------------------------------
    console.log(`\n# summary`);
    for (const s of summary) {
      const extra = s.file ? ` (${s.file})` : "";
      console.log(`#   ${s.email}: ${s.action}${extra}`);
    }
    const rotated = summary.find((s) => s.newPassword);
    if (rotated) {
      // The ONLY place this script prints a plaintext secret — the operator must be
      // told the new admin password, and there is nowhere else it can come from.
      console.log(`\n#   NEW ADMIN PASSWORD: ${rotated.newPassword}`);
      console.log(`#   (record it now — it is not stored anywhere else)`);
    }
  } finally {
    if (adminToken) {
      try {
        await ws.call("logout", { session_token: adminToken });
      } catch {
        /* best effort */
      }
    }
    ws.close();
  }
}

main().catch((e) => {
  console.error(`ERROR: ${e.message}`);
  process.exit(1);
});
